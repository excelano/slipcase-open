//! Concept 10's layers where Windows keeps them.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! **The `Policies` subtree, and not the application's own key.** Concept 10
//! names `HKLM\SOFTWARE\Policies\Excelano\Slipcase` because that subtree is
//! access-controlled against standard users and Group Policy removes what it
//! wrote when a policy is unapplied — so a control an administrator withdraws
//! stops applying, which is not true of anything this application writes for
//! itself. The same section is explicit that application settings must never be
//! read from the normal key while policy is in effect, and that is not enforced
//! here: it is [`policy::resolve`], through `user_may_extend`, which is where
//! it belongs because the rule is about precedence rather than about registries.
//!
//! **`\Open` under the family key.** The viewer claims the same extension and
//! will want its own controls; two products sharing one key would mean an
//! administrator's list for one silently governing the other. `Slipcase` stays
//! the family and each product has a subkey.
//!
//! ## What the values look like
//!
//! | Setting | Type | |
//! | --- | --- | --- |
//! | `allowed`, `denied` | `REG_MULTI_SZ` | one extension per line, no dots |
//! | `mode` | `REG_SZ` | `replace` or `append` |
//! | `notify` | `REG_SZ` | `important` or `everything` |
//! | `user_may_extend` | `REG_DWORD` | 0 or 1 |
//! | `confirm_each_write_back` | `REG_DWORD` | 0 or 1 |
//!
//! The names and the spellings are the ones `packaging/linux/open.toml` uses,
//! deliberately: an administrator who has read the documentation for one
//! platform has read it for both, and the ADMX pair generates keys with these
//! names.
//!
//! **A value that cannot be understood is a refusal and not a shrug**, and
//! `policy::Error::Malformed` names the key and the value rather than the key
//! alone. This is the case concept 10 cares about most: a control an
//! administrator wrote, ignored quietly for as long as the typo survives.
//! `notify = "loud"` is the shape of it, and it refuses.
//!
//! **A single string where a list belongs is understood rather than refused,
//! and that is a decision rather than an oversight.** Measured: a `denied`
//! written as `REG_SZ` reads back as a one-entry list, because the bindings
//! convert it. Refusing would be defensible and is worse here. Concept 10 makes
//! an unreadable policy fail closed, so refusing a near-miss would stop the
//! product opening anything at all until somebody found the wrong type — and
//! the near-miss is not dangerous in either direction: a one-entry `denied`
//! still denies, and a one-entry `allowed` permits less than was meant, never
//! more. The refusal is kept for what actually cannot be read.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{Error, Layer, Mode, Notify, Origin, Read, Source};

/// The subtree an administrator writes to, under either hive.
const POLICY_KEY: &str = r"SOFTWARE\Policies\Excelano\Slipcase\Open";

/// The application's own key, which is the user's to set and which policy
/// suppresses through `user_may_extend`.
const APPLICATION_KEY: &str = r"SOFTWARE\Excelano\Slipcase\Open";

/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`, which is a key that is not there.
///
/// Written out because the macro is not exposed by the bindings, and named
/// because the alternative — reading every failure to open as absence — is
/// exactly the flattening concept 10 forbids: a `Policies` key an ACL refuses
/// would become a machine with no policy on it.
const KEY_NOT_THERE: i32 = 0x8007_0002_u32.cast_signed();

/// Which hive a layer is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hive {
    /// `HKEY_LOCAL_MACHINE`.
    Machine,
    /// `HKEY_CURRENT_USER`.
    User,
}

impl Hive {
    fn short(self) -> &'static str {
        match self {
            Self::Machine => "HKLM",
            Self::User => "HKCU",
        }
    }
}

/// Concept 10's layers, read from the registry.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    keys: BTreeMap<Origin, (Hive, &'static str)>,
}

impl Registry {
    /// A source that reads nothing, for tests and for building one up.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The layers this platform keeps in the registry, at the keys concept 10
    /// names.
    ///
    /// There is no machine-wide *configuration* layer and there is not meant to
    /// be: `HKLM` outside `Policies` is writable by anything running elevated
    /// and is not cleaned up when a policy is withdrawn, so a setting there
    /// would be a control nobody administers and nobody can see.
    #[must_use]
    pub fn for_this_platform() -> Self {
        let mut keys = BTreeMap::new();
        keys.insert(Origin::MachinePolicy, (Hive::Machine, POLICY_KEY));
        keys.insert(Origin::UserPolicy, (Hive::User, POLICY_KEY));
        keys.insert(Origin::Configuration, (Hive::User, APPLICATION_KEY));
        Self { keys }
    }

    /// Where each layer is read from, most authoritative first.
    ///
    /// The same order and the same reason as `files::Files::locations`: a
    /// person reading a list of layers wants the one that wins at the top.
    #[must_use]
    pub fn locations(&self) -> Vec<(Origin, String)> {
        self.keys
            .iter()
            .rev()
            .map(|(origin, (hive, key))| (*origin, format!(r"{}\{}", hive.short(), key)))
            .collect()
    }
}

impl Source for Registry {
    fn layer(&self, origin: Origin) -> Read {
        let Some((hive, subkey)) = self.keys.get(&origin) else {
            return Ok(None);
        };
        read(*hive, subkey)
    }
}

/// A `REG_MULTI_SZ` without the empty string that ends it.
///
/// **Found by running it rather than by reading the documentation.** A list
/// written with the registry editor came back with a trailing empty entry — the
/// terminator, which the format requires and which a reader is meant to drop —
/// and it reached `resolve` as an extension. The symptom was the interface
/// reporting an entry in a policy list that cannot match any payload, naming an
/// empty one, to somebody who had written a perfectly ordinary list — which is
/// worse than it sounds: a person told their policy contains something wrong
/// looks for a mistake they did not make.
///
/// Only the trailing ones go. An empty entry in the *middle* is somebody's
/// mistake, and `resolve` already names it — dropping that one silently would
/// hide the thing this sentence is complaining about.
fn without_the_terminator(mut values: Vec<String>) -> Vec<String> {
    while values.last().is_some_and(String::is_empty) {
        values.pop();
    }
    values
}

/// The name a failure reports, which is a key and not a file.
///
/// `policy::Error` carries a `PathBuf` because its first implementation read
/// files. A registry key is not a path and putting one here is a small lie for
/// a display string — worth it against a second error type whose only
/// difference is the word, and harmless because nothing opens it.
fn named(hive: Hive, subkey: &str, value: &str) -> PathBuf {
    PathBuf::from(format!(r"{}\{}\{}", hive.short(), subkey, value))
}

fn read(hive: Hive, subkey: &str) -> Read {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

    let root = match hive {
        Hive::Machine => LOCAL_MACHINE,
        Hive::User => CURRENT_USER,
    };
    // A key that is not there is a machine with no policy applied, which is the
    // common case and not a condition to report. Anything else — a key that
    // exists and will not open, which on this subtree means an ACL saying so —
    // is a refusal, for the reason the module comment gives.
    let key = match root.open(subkey) {
        Ok(key) => key,
        Err(e) if e.code().0 == KEY_NOT_THERE => return Ok(None),
        Err(e) => {
            return Err(Error::Unreadable {
                path: PathBuf::from(format!(r"{}\{}", hive.short(), subkey)),
                cause: std::io::Error::other(e.message()),
            })
        }
    };

    let bad = |value: &str, cause: String| Error::Malformed {
        path: named(hive, subkey, value),
        cause,
    };

    // Absent is `None`; present and the wrong type is a refusal. `get_*`
    // conflates those, so absence is established first and any failure after
    // that is about the value rather than about whether there is one.
    let has = |name: &str| key.get_type(name).is_ok();

    let list = |name: &str| -> std::result::Result<Option<Vec<String>>, Error> {
        if !has(name) {
            return Ok(None);
        }
        key.get_multi_string(name)
            .map(|v| Some(without_the_terminator(v)))
            .map_err(|_| {
                bad(
                    name,
                    "must be a REG_MULTI_SZ of extensions, one to a line".to_string(),
                )
            })
    };

    let flag = |name: &str| -> std::result::Result<Option<bool>, Error> {
        if !has(name) {
            return Ok(None);
        }
        key.get_u32(name)
            .map(|v| Some(v != 0))
            .map_err(|_| bad(name, "must be a REG_DWORD of 0 or 1".to_string()))
    };

    // Spelled out rather than derived, the same way `files::parse` spells them
    // out: there are two values, and an administrator who writes a third has
    // made a mistake worth a sentence.
    let word = |name: &str| -> std::result::Result<Option<String>, Error> {
        if !has(name) {
            return Ok(None);
        }
        key.get_string(name)
            .map(Some)
            .map_err(|_| bad(name, "must be a REG_SZ".to_string()))
    };

    let mode = match word("mode")?.as_deref() {
        None => None,
        Some("replace") => Some(Mode::Replace),
        Some("append") => Some(Mode::Append),
        Some(other) => {
            return Err(bad(
                "mode",
                format!("must be \"replace\" or \"append\", not \"{other}\""),
            ))
        }
    };

    let notify = match word("notify")?.as_deref() {
        None => None,
        Some("everything") => Some(Notify::Everything),
        Some("important") => Some(Notify::Important),
        Some(other) => {
            return Err(bad(
                "notify",
                format!("must be \"everything\" or \"important\", not \"{other}\""),
            ))
        }
    };

    Ok(Some(Layer {
        allowed: list("allowed")?,
        mode,
        denied: list("denied")?,
        user_may_extend: flag("user_may_extend")?,
        confirm_each_write_back: flag("confirm_each_write_back")?,
        notify,
    }))
}

#[cfg(test)]
mod tests {
    use super::{without_the_terminator, Hive, Registry, APPLICATION_KEY, POLICY_KEY};
    use crate::policy::{Origin, Source};

    #[test]
    fn the_three_layers_are_the_ones_concept_ten_names() {
        // Machine policy and user policy from the `Policies` subtree, the
        // user's own from the application key, and nothing machine-wide outside
        // `Policies` — which is the one an administrator could not withdraw.
        let r = Registry::for_this_platform();
        let found = r.locations();
        assert_eq!(found.len(), 3);
        assert_eq!(
            found[0],
            (Origin::MachinePolicy, format!(r"HKLM\{POLICY_KEY}"))
        );
        assert_eq!(
            found[1],
            (Origin::UserPolicy, format!(r"HKCU\{POLICY_KEY}"))
        );
        assert_eq!(
            found[2],
            (Origin::Configuration, format!(r"HKCU\{APPLICATION_KEY}"))
        );
    }

    #[test]
    fn the_layers_are_listed_with_the_one_that_wins_at_the_top() {
        // A person reading this list is looking for what is overriding them,
        // and the resolution wants the opposite order, so the reversal has to
        // be here rather than assumed at either end.
        let found = Registry::for_this_platform().locations();
        let origins: Vec<Origin> = found.iter().map(|(o, _)| *o).collect();
        let mut sorted = origins.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(origins, sorted);
    }

    #[test]
    fn a_source_asked_for_a_layer_it_has_no_key_for_says_nothing() {
        // Not an error and not a refusal. `Registry::none` is what the tests
        // build on and what a platform with no such layer would answer.
        let r = Registry::none();
        assert!(r.layer(Origin::MachinePolicy).unwrap().is_none());
        assert!(r.layer(Origin::BuiltIn).unwrap().is_none());
    }

    #[test]
    fn the_empty_string_that_ends_a_multi_sz_is_not_an_extension() {
        // Measured against a real key: a two-entry list read back with a third,
        // empty, entry, which reached the resolution and made the interface
        // complain about a list nobody had got wrong.
        assert_eq!(
            without_the_terminator(vec!["txt".into(), "dwg".into(), String::new()]),
            vec!["txt".to_string(), "dwg".to_string()]
        );
        // More than one, and a list that is only terminator.
        assert_eq!(
            without_the_terminator(vec!["txt".into(), String::new(), String::new()]),
            vec!["txt".to_string()]
        );
        assert!(without_the_terminator(vec![String::new()]).is_empty());
        // And nothing to do to a list that does not carry one.
        assert_eq!(
            without_the_terminator(vec!["txt".into()]),
            vec!["txt".to_string()]
        );
    }

    #[test]
    fn an_empty_entry_in_the_middle_is_left_for_the_resolution_to_object_to() {
        // That one is a mistake somebody made rather than a terminator, and
        // `resolve` says so. Dropping it here would hide it.
        assert_eq!(
            without_the_terminator(vec!["txt".into(), String::new(), "dwg".into()]),
            vec!["txt".to_string(), String::new(), "dwg".to_string()]
        );
    }

    /// Every value the reader understands, which is the set the ADMX must write
    /// and no other.
    const EVERY_VALUE: [&str; 6] = [
        "allowed",
        "mode",
        "denied",
        "user_may_extend",
        "confirm_each_write_back",
        "notify",
    ];

    #[test]
    fn the_admx_writes_the_keys_and_values_this_module_reads() {
        // The pair that drifts. An ADMX is a separate administrator download
        // and nothing at run time consults it, so a value renamed here and not
        // there produces a policy that appears to apply, reports itself as
        // applied in the Group Policy editor, and governs nothing — with no
        // error anywhere to find. Nobody would notice until somebody opened
        // what the policy was written to refuse.
        //
        // Read from the file rather than restated, because a copy of the names
        // in a test is a third place for them to disagree.
        let admx = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packaging/windows/policy/SlipcaseOpen.admx");
        let text =
            std::fs::read_to_string(&admx).unwrap_or_else(|e| panic!("{}: {e}", admx.display()));

        let attribute = |name: &str| -> Vec<String> {
            let needle = format!("{name}=\"");
            text.match_indices(&needle)
                .filter_map(|(at, _)| {
                    let rest = &text[at + needle.len()..];
                    rest.find('"').map(|end| rest[..end].to_string())
                })
                .collect()
        };

        // Every key an administrator's setting lands in is the one that is read.
        let keys = attribute("key");
        assert!(!keys.is_empty(), "the ADMX names no keys at all");
        for key in &keys {
            assert_eq!(key, POLICY_KEY, "the ADMX writes a key nothing reads");
        }

        // And every value, in both directions: one the ADMX writes and the
        // reader ignores is a control that does nothing, and one the reader
        // reads and the ADMX cannot write is a setting with no way to set it.
        let mut written: Vec<String> = attribute("valueName");
        written.sort();
        written.dedup();
        let mut read: Vec<String> = EVERY_VALUE.iter().map(|s| (*s).to_string()).collect();
        read.sort();
        assert_eq!(written, read);
    }

    #[test]
    fn a_hive_is_named_the_way_an_administrator_would_type_it() {
        // The strings here end up in `slipcase-open policy`, which exists so
        // that what it prints can be pasted into `reg` or into the editor.
        assert_eq!(Hive::Machine.short(), "HKLM");
        assert_eq!(Hive::User.short(), "HKCU");
    }
}
