//! What the tool will open, and who gets to say.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 10. Four layers — machine policy, user policy, user configuration,
//! built-in default — resolved into one answer, and one function that answers
//! it for a payload.
//!
//! **This module is the resolution and not the sources.** Reading a registry
//! subtree, a configuration profile, or a file under `/etc` is a platform's
//! business and lives behind [`Source`]. What is here is pure, which is what
//! makes the precedence testable without three operating systems.
//!
//! **Nothing here is cached, and that is the security property.** Concept 10
//! puts enforcement in the launch path, immediately before execution: a value
//! read at startup, or held across a policy push, or handed in over IPC, is a
//! bypass. [`decide`] resolves from the sources on every call for that reason,
//! and [`Effective`] exists for the interface to describe the state of things
//! rather than for the launch path to consult twice.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use crate::extension;

pub mod files;

/// Why a layer could not be read.
///
/// **A policy source that fails is not a policy source that says nothing.** An
/// administrator's deny list that cannot be read is the case concept 10 cares
/// about most, and answering `None` for it would permit whatever it was written
/// to refuse — quietly, and for as long as the typo survives. So reading a
/// layer can fail, and the failure travels to whoever is deciding.
#[derive(Debug)]
pub enum Error {
    /// The file is there and could not be read.
    Unreadable {
        /// The file.
        path: PathBuf,
        /// What the filesystem said.
        cause: std::io::Error,
    },
    /// The file is there and is not what it claims to be.
    Malformed {
        /// The file.
        path: PathBuf,
        /// What was wrong with it.
        cause: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, cause } => {
                write!(f, "{} cannot be read: {cause}", path.display())
            }
            Self::Malformed { path, cause } => write!(f, "{}: {cause}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

/// A layer, or nothing, or a reason neither could be established.
pub type Read = std::result::Result<Option<Layer>, Error>;

/// Where a layer came from, in order of authority.
///
/// Machine policy wins over user policy, which wins over the user's own
/// configuration, which wins over what this build ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// What this build ships when nothing else says otherwise.
    BuiltIn,
    /// The user's own settings. The only layer [`Layer::user_may_extend`] gates.
    Configuration,
    /// Policy applied to this user, and administered rather than chosen.
    UserPolicy,
    /// Policy applied to this machine. Nothing overrides it.
    MachinePolicy,
}

impl Origin {
    /// Whether a layer from here is administered rather than chosen, which is
    /// what the interface indicates so that a refusal reads as a decision
    /// somebody made rather than as the application being unpredictable.
    #[must_use]
    pub fn is_managed(self) -> bool {
        matches!(self, Self::UserPolicy | Self::MachinePolicy)
    }
}

/// The name a person knows a layer by, which is the vocabulary concept 10 uses
/// and the vocabulary the shipped policy file and the manual page use with it.
impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `pad` rather than `write_str`, so that a caller lining these up in a
        // column gets the width it asked for. `write_str` ignores it silently,
        // which is a formatting bug that only shows up in the output.
        f.pad(match self {
            Self::BuiltIn => "built-in",
            Self::Configuration => "configuration",
            Self::UserPolicy => "user policy",
            Self::MachinePolicy => "machine policy",
        })
    }
}

/// How much the tool says without being asked.
///
/// A threshold on [`crate::present::Weight`] rather than a list of switches, so
/// that adding a report does not add a setting: it is written at the weight it
/// deserves and lands on the right side of the line by itself.
///
/// **Questions are outside this and cannot be quietened.** They go through
/// [`crate::present::Channel::ask`] rather than `report`, so nothing here
/// reaches them. That is structural rather than a rule somebody has to
/// remember: a session waiting on a decision would otherwise be silenced into
/// stranding its payload, and concept 6.3 has nothing else to offer that person
/// until the next launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Notify {
    /// Everything, including what happens on its own.
    Everything,
    /// Everything except [`crate::present::Weight::Routine`] — so warnings,
    /// failures, questions, and the answer to anything the person did. **The
    /// default**, because the routine half is a notification per save and
    /// somebody editing a document for an hour gets dozens of them. Concept 6.2
    /// wants a session visible and wants somewhere to look when an edit is
    /// expected to have landed, and `sessions` answers that better than a
    /// stream of banners.
    #[default]
    Important,
}

/// Whether a layer's allowed list stands alone or adds to what is beneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The list is the permitted set. Everything beneath is discarded.
    #[default]
    Replace,
    /// The list is added to what is beneath.
    Append,
}

/// One layer's contribution. Every field is optional, and `None` means the
/// layer says nothing about it rather than saying no.
#[derive(Debug, Clone, Default)]
pub struct Layer {
    /// Extensions this layer permits.
    pub allowed: Option<Vec<String>>,
    /// Whether `allowed` stands alone or adds. **Defaults to [`Mode::Replace`]
    /// deliberately.** Concept 10 names the silent hole: an administrator who
    /// writes a list expecting it to be exhaustive and gets it unioned with the
    /// defaults has permitted things they never listed, and never finds out.
    /// Appending is the surprising reading, so it is the one that has to be
    /// asked for.
    pub mode: Option<Mode>,
    /// Extensions this layer refuses. Always wins, everywhere.
    pub denied: Option<Vec<String>>,
    /// Whether the user's own configuration is honoured at all. Only a policy
    /// layer setting this to `false` has any effect, and what it suppresses is
    /// [`Origin::Configuration`] — not another policy layer, which is
    /// administered too.
    pub user_may_extend: Option<bool>,
    /// Whether every write-back is confirmed, rather than only the session
    /// close. Concept 6.2 keeps this off by default: a repack is atomic and
    /// unremarkable, and a prompt on every save is friction for everyone who is
    /// not archiving.
    pub confirm_each_write_back: Option<bool>,
    /// How much the tool says without being asked. See [`Notify`].
    pub notify: Option<Notify>,
}

impl Layer {
    /// Whether this layer sets nothing at all.
    ///
    /// A file that parses and holds no key is a layer that exists and has no
    /// opinion, which is different from one that is not there — but not
    /// different in any way a decision can see. What it changes is whether
    /// [`Effective::managed`] should call the machine administered, and it
    /// should not: see the note there.
    ///
    /// `allowed = []` is not this. An empty list is a layer permitting nothing,
    /// which says a great deal.
    #[must_use]
    pub fn says_nothing(&self) -> bool {
        self.allowed.is_none()
            && self.mode.is_none()
            && self.denied.is_none()
            && self.user_may_extend.is_none()
            && self.confirm_each_write_back.is_none()
            && self.notify.is_none()
    }
}

/// Where layers come from. One implementation per platform, plus whatever the
/// tests need.
///
/// Returning `None` for a layer means this platform has no such source, or the
/// source is empty. It is not an error and is not a refusal.
pub trait Source {
    /// This layer, read now rather than remembered. An implementation that
    /// caches is the bypass concept 10 warns about.
    ///
    /// `Ok(None)` means this platform has no such source, or it is not there.
    /// That is not an error and not a refusal. An `Err` means the source exists
    /// and could not be understood, which is a different thing entirely and
    /// must not be flattened into the first.
    ///
    /// # Errors
    ///
    /// See [`Error`].
    fn layer(&self, origin: Origin) -> Read;
}

/// What this build permits when nothing else says otherwise.
///
/// Documents and images: what an information-management shop actually sends
/// somebody. It errs small, because everything missing from it is one setting
/// away and everything wrongly on it is a guardrail this tool claimed and did
/// not provide.
///
/// **`slpc` is not here**, and concept 10 explains why. A container whose
/// payload is a container is usually somebody having packed one by mistake, or
/// an archival wrapper, and neither wants an automatic recursive open. It stays
/// allowlistable for the archival user who nests deliberately.
///
/// Nor is anything the browser opens — `html`, `htm`, `svg`, `xhtml`. They are
/// harmless through this path, because the handler is a browser and a browser
/// is built for hostile documents, but they are the one document family whose
/// content routinely executes and the default set is not the place to argue
/// about it.
pub const BUILT_IN_ALLOWED: &[&str] = &[
    // Documents
    "pdf", "rtf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "odg",
    // Plain text and its families
    "txt", "md", "csv", "tsv", "log", "json", "xml", "yaml", "yml", // Images
    "jpg", "jpeg", "png", "gif", "tif", "tiff", "webp", "heic", "bmp",
];

/// The resolved answer, for an interface that wants to describe it.
///
/// The launch path calls [`decide`] and does not hold one of these. See the
/// module note on caching.
#[derive(Debug, Clone)]
pub struct Effective {
    allowed: BTreeSet<String>,
    denied: BTreeSet<String>,
    /// Whether a policy layer set anything, so the interface can say that
    /// settings are administered.
    ///
    /// **Set something, rather than merely be there.** The Linux package ships
    /// `/etc/slipcase/open.toml` documenting every key and setting none of
    /// them, so a rule that counted the file's presence would have every
    /// machine that installed the package told its settings were administered
    /// when nothing had been administered — which is the support load concept
    /// 10 wants this to reduce, arriving by the front door.
    pub managed: bool,
    /// Whether the user's own configuration was suppressed by policy.
    pub configuration_suppressed: bool,
    /// See [`Layer::confirm_each_write_back`].
    pub confirm_each_write_back: bool,
    /// See [`Notify`].
    ///
    /// **Read once and held, unlike the lists.** The note at the top of this
    /// module is about what may be opened, where a value cached across a policy
    /// push is a bypass. This one governs how loud the tool is and gates no
    /// decision, so nothing is lost by resolving it when the instance starts.
    pub notify: Notify,
    /// List entries that can never match anything, because they are not
    /// comparable under concept 5.2. Surfaced rather than dropped: an entry an
    /// administrator wrote and this will never honour is worth a word, and
    /// silently ignoring one on a deny list is the worse half.
    pub uncomparable_entries: Vec<String>,
}

/// What is to be done with a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Permitted. The folded extension is carried so a caller reports the same
    /// value the decision was made against.
    Open { key: String },
    /// On a deny list, which wins over every allow list.
    Denied { key: String },
    /// Not on any allow list. A different answer from `Denied`, because the
    /// remedy differs and so does the sentence a person should be shown.
    NotPermitted { key: String },
    /// No extension, or one nothing can compare. Concept 5.1 refuses this
    /// whatever the lists say, because `ShellExecuteEx` answers it with the
    /// Open With dialog, which offers every executable on the machine inside a
    /// flow the person reads as *open the document*.
    NoUsableExtension,
}

/// Resolve the layers and answer for this payload, in one call.
///
/// **Takes the payload's name and not an extension.** Concept 10 requires the
/// decision to be made after the extension is taken and folded, and a signature
/// accepting a key would let a caller hand in one it computed some other way.
/// The folding is [`extension::policy_key`]'s, once, here.
///
/// # Errors
///
/// Where a layer exists and cannot be read. Nothing is decided in that case,
/// because a policy that cannot be established is not a policy that permits.
pub fn decide(source: &dyn Source, payload_name: &str) -> std::result::Result<Decision, Error> {
    let Some(key) = extension::policy_key(payload_name) else {
        return Ok(Decision::NoUsableExtension);
    };
    let effective = resolve(source)?;
    Ok(if effective.denied.contains(&key) {
        Decision::Denied { key }
    } else if effective.allowed.contains(&key) {
        Decision::Open { key }
    } else {
        Decision::NotPermitted { key }
    })
}

/// Resolve the layers without deciding anything, for an interface describing
/// the state of things.
///
/// # Errors
///
/// Where a layer exists and cannot be read.
pub fn resolve(source: &dyn Source) -> std::result::Result<Effective, Error> {
    let mut uncomparable = Vec::new();

    // Highest authority first, so that `user_may_extend` is known before the
    // layer it gates is looked at.
    let machine = source.layer(Origin::MachinePolicy)?;
    let user_policy = source.layer(Origin::UserPolicy)?;

    let may_extend = machine
        .as_ref()
        .and_then(|l| l.user_may_extend)
        .or_else(|| user_policy.as_ref().and_then(|l| l.user_may_extend))
        .unwrap_or(true);

    // Not read at all where policy has suppressed it, which is the point: a
    // suppressed layer is not consulted, so a broken one cannot fail a decision
    // that would have ignored it anyway.
    let configuration = if may_extend {
        source.layer(Origin::Configuration)?
    } else {
        None
    };

    let built_in = source.layer(Origin::BuiltIn)?.unwrap_or(Layer {
        allowed: Some(BUILT_IN_ALLOWED.iter().map(|s| (*s).to_string()).collect()),
        ..Layer::default()
    });

    let stack = [
        (Origin::MachinePolicy, machine),
        (Origin::UserPolicy, user_policy),
        (Origin::Configuration, configuration),
        (Origin::BuiltIn, Some(built_in)),
    ];

    // Every layer's refusals, unioned. A deny is never overridden, including by
    // a layer with more authority: a user refusing something for themselves is
    // a preference an administrator has no reason to overrule, and concept 10
    // says the deny list wins regardless of every other setting.
    let mut denied = BTreeSet::new();
    for layer in stack.iter().filter_map(|(_, l)| l.as_ref()) {
        fold_into(&mut denied, layer.denied.as_deref(), &mut uncomparable);
    }

    // Allowed is built from the bottom up, so that a `Replace` discards what is
    // beneath it and an `Append` adds to it.
    let mut allowed = BTreeSet::new();
    for layer in stack.iter().rev().filter_map(|(_, l)| l.as_ref()) {
        let Some(list) = layer.allowed.as_deref() else {
            continue;
        };
        if layer.mode.unwrap_or_default() == Mode::Replace {
            allowed.clear();
        }
        fold_into(&mut allowed, Some(list), &mut uncomparable);
    }

    let managed = stack
        .iter()
        .any(|(o, l)| o.is_managed() && l.as_ref().is_some_and(|l| !l.says_nothing()));

    let confirm = stack
        .iter()
        .find_map(|(_, l)| l.as_ref().and_then(|l| l.confirm_each_write_back))
        .unwrap_or(false);

    let notify = stack
        .iter()
        .find_map(|(_, l)| l.as_ref().and_then(|l| l.notify))
        .unwrap_or_default();

    uncomparable.sort_unstable();
    uncomparable.dedup();

    Ok(Effective {
        allowed,
        denied,
        managed,
        configuration_suppressed: !may_extend,
        confirm_each_write_back: confirm,
        notify,
        uncomparable_entries: uncomparable,
    })
}

impl Effective {
    /// The permitted set, folded, in a stable order.
    pub fn allowed(&self) -> impl Iterator<Item = &str> {
        self.allowed.iter().map(String::as_str)
    }

    /// The refused set, folded, in a stable order.
    pub fn denied(&self) -> impl Iterator<Item = &str> {
        self.denied.iter().map(String::as_str)
    }
}

/// Fold each entry the way a payload's extension is folded, so that a list and
/// a filename are compared as the same kind of thing. An entry that will not
/// fold is collected rather than dropped.
fn fold_into(into: &mut BTreeSet<String>, list: Option<&[String]>, uncomparable: &mut Vec<String>) {
    for entry in list.unwrap_or_default() {
        // Written as `pdf` or as `.pdf`; both mean the same thing to whoever
        // typed it, and refusing one of them would be pedantry with a support
        // cost.
        let bare = entry.strip_prefix('.').unwrap_or(entry);
        if !bare.is_empty() && bare.chars().all(|c| c.is_ascii_alphanumeric()) {
            into.insert(bare.to_ascii_lowercase());
        } else {
            uncomparable.push(entry.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decide, resolve, Decision, Layer, Mode, Origin, Read, Source, BUILT_IN_ALLOWED};

    /// A source built from whatever a test wants to say, layer by layer.
    #[derive(Default)]
    struct Stack {
        machine: Option<Layer>,
        user_policy: Option<Layer>,
        configuration: Option<Layer>,
    }

    impl Source for Stack {
        fn layer(&self, origin: Origin) -> Read {
            Ok(match origin {
                Origin::MachinePolicy => self.machine.clone(),
                Origin::UserPolicy => self.user_policy.clone(),
                Origin::Configuration => self.configuration.clone(),
                // `resolve` supplies the shipped set where a source says
                // nothing, which is what every arm below relies on.
                Origin::BuiltIn => None,
            })
        }
    }

    /// Wrapped, because that is the shape every `Layer` field takes: a list
    /// and *said nothing* are different answers.
    #[allow(clippy::unnecessary_wraps)]
    fn list(of: &[&str]) -> Option<Vec<String>> {
        Some(of.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn with_nothing_configured_the_shipped_set_is_what_answers() {
        let s = Stack::default();
        assert_eq!(
            decide(&s, "report.pdf").unwrap(),
            Decision::Open {
                key: "pdf".to_string()
            }
        );
        assert_eq!(
            decide(&s, "setup.exe").unwrap(),
            Decision::NotPermitted {
                key: "exe".to_string()
            }
        );
    }

    #[test]
    fn a_nested_container_is_not_permitted_by_default() {
        // Concept 10. Allowlistable, and not shipped allowed.
        assert!(!BUILT_IN_ALLOWED.contains(&"slpc"));
        assert_eq!(
            decide(&Stack::default(), "inner.slpc").unwrap(),
            Decision::NotPermitted {
                key: "slpc".to_string()
            }
        );
    }

    #[test]
    fn a_policy_list_replaces_rather_than_appends_when_it_does_not_say() {
        // The silent hole concept 10 names: an administrator writing an
        // exhaustive list and getting it unioned with the defaults has
        // permitted things they never listed and will not find out.
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["txt"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert_eq!(
            decide(&s, "notes.txt").unwrap(),
            Decision::Open {
                key: "txt".to_string()
            }
        );
        assert_eq!(
            decide(&s, "report.pdf").unwrap(),
            Decision::NotPermitted {
                key: "pdf".to_string()
            }
        );
    }

    #[test]
    fn appending_is_available_and_has_to_be_asked_for() {
        let s = Stack {
            configuration: Some(Layer {
                allowed: list(&["slpc"]),
                mode: Some(Mode::Append),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(matches!(
            decide(&s, "inner.slpc").unwrap(),
            Decision::Open { .. }
        ));
        assert!(matches!(
            decide(&s, "report.pdf").unwrap(),
            Decision::Open { .. }
        ));
    }

    #[test]
    fn a_deny_wins_over_an_allow_in_the_same_layer() {
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["pdf", "txt"]),
                denied: list(&["pdf"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert_eq!(
            decide(&s, "report.pdf").unwrap(),
            Decision::Denied {
                key: "pdf".to_string()
            }
        );
    }

    #[test]
    fn a_deny_beneath_wins_over_an_allow_above_it() {
        // Concept 10 says the deny list wins regardless of every other
        // setting, and that includes authority. A user refusing something for
        // themselves is a preference an administrator has no reason to
        // overrule, and nothing is made less safe by honouring it.
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["pdf"]),
                ..Layer::default()
            }),
            configuration: Some(Layer {
                denied: list(&["pdf"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(matches!(
            decide(&s, "report.pdf").unwrap(),
            Decision::Denied { .. }
        ));
    }

    #[test]
    fn policy_can_suppress_the_users_own_configuration() {
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["pdf"]),
                user_may_extend: Some(false),
                ..Layer::default()
            }),
            configuration: Some(Layer {
                allowed: list(&["exe"]),
                mode: Some(Mode::Append),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(matches!(
            decide(&s, "setup.exe").unwrap(),
            Decision::NotPermitted { .. }
        ));
        assert!(resolve(&s).unwrap().configuration_suppressed);
    }

    #[test]
    fn suppressing_the_configuration_does_not_suppress_the_other_policy_layer() {
        // `user_may_extend` gates what the user chose, not what was
        // administered to them. Both policy layers are somebody's decision.
        let s = Stack {
            machine: Some(Layer {
                user_may_extend: Some(false),
                ..Layer::default()
            }),
            user_policy: Some(Layer {
                allowed: list(&["dwg"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(matches!(
            decide(&s, "plan.dwg").unwrap(),
            Decision::Open { .. }
        ));
    }

    #[test]
    fn machine_policy_outranks_user_policy_on_the_allowed_set() {
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["txt"]),
                ..Layer::default()
            }),
            user_policy: Some(Layer {
                allowed: list(&["dwg"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(matches!(
            decide(&s, "notes.txt").unwrap(),
            Decision::Open { .. }
        ));
        assert!(matches!(
            decide(&s, "plan.dwg").unwrap(),
            Decision::NotPermitted { .. }
        ));
    }

    #[test]
    fn a_payload_with_no_usable_extension_is_refused_whatever_the_lists_say() {
        // Concept 5.1: there is no setting for this, because the dialog it
        // would otherwise raise offers every executable on the machine.
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["pdf"]),
                mode: Some(Mode::Append),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert_eq!(decide(&s, "README").unwrap(), Decision::NoUsableExtension);
        assert_eq!(decide(&s, ".bashrc").unwrap(), Decision::NoUsableExtension);
        assert_eq!(
            decide(&s, "notes.tëxt").unwrap(),
            Decision::NoUsableExtension
        );
    }

    #[test]
    fn list_entries_are_folded_the_way_a_payload_name_is() {
        let s = Stack {
            machine: Some(Layer {
                allowed: list(&["PDF", ".Txt"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        // Both spellings on both sides: the list may be shouted or dotted, and
        // the container may spell its own name however it likes.
        assert!(matches!(
            decide(&s, "REPORT.PDF").unwrap(),
            Decision::Open { .. }
        ));
        assert!(matches!(
            decide(&s, "notes.txt").unwrap(),
            Decision::Open { .. }
        ));
    }

    #[test]
    fn the_decision_carries_the_key_it_was_made_against() {
        // So that whatever reports the refusal names the value that was
        // compared, rather than folding the name a second time and possibly
        // differently.
        assert_eq!(
            decide(&Stack::default(), "SETUP.EXE").unwrap(),
            Decision::NotPermitted {
                key: "exe".to_string()
            }
        );
    }

    #[test]
    fn an_entry_nothing_can_compare_is_surfaced_rather_than_dropped() {
        // An administrator wrote it and this will never honour it. Silently
        // ignoring one on a deny list is the half that matters.
        let s = Stack {
            machine: Some(Layer {
                denied: list(&["exe", "*.exe", "ex\u{212a}"]),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        let e = resolve(&s).unwrap();
        assert_eq!(e.uncomparable_entries, vec!["*.exe", "ex\u{212a}"]);
        assert!(e.denied().any(|d| d == "exe"));
    }

    #[test]
    fn managed_says_whether_a_policy_layer_contributed() {
        assert!(!resolve(&Stack::default()).unwrap().managed);
        // Present and empty is not administered. The package ships a policy
        // file that sets nothing, and this is the rule that keeps every install
        // of it from claiming otherwise.
        let empty = Stack {
            user_policy: Some(Layer::default()),
            ..Stack::default()
        };
        assert!(!resolve(&empty).unwrap().managed);
        // Setting anything is, including permitting nothing.
        let refusing_everything = Stack {
            user_policy: Some(Layer {
                allowed: Some(Vec::new()),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(resolve(&refusing_everything).unwrap().managed);
    }

    #[test]
    fn confirming_each_write_back_is_off_until_a_layer_asks() {
        assert!(!resolve(&Stack::default()).unwrap().confirm_each_write_back);
        let s = Stack {
            configuration: Some(Layer {
                confirm_each_write_back: Some(true),
                ..Layer::default()
            }),
            ..Stack::default()
        };
        assert!(resolve(&s).unwrap().confirm_each_write_back);
    }
}
