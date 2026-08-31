//! Policy layers read from files.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 10's four layers, as TOML documents. The paths are given rather than
//! discovered, so the precedence can be tested without three operating systems
//! and so a test never reads a real machine's policy.
//!
//! **This is the portable shape and not the whole of concept 10.** Windows
//! reads its two policy layers from the `Policies` registry subtree, which is
//! access-controlled against standard users and cleaned up by Group Policy on
//! unapply, and macOS reads a configuration profile through
//! `CFPreferencesAppValueIsForced`. Neither is a file and neither belongs here.
//! What is here is Linux's shape, and the trait implementation every test uses.
//!
//! ## What a layer looks like
//!
//! ```toml
//! allowed = ["pdf", "docx", "odt"]
//! mode = "replace"                  # or "append"; replace is the default
//! denied = ["exe", "dll"]
//! user_may_extend = false
//! confirm_each_write_back = true
//! ```
//!
//! Every key is optional, and omitting one means this layer says nothing about
//! it rather than saying no.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Error, Layer, Mode, Origin, Read, Source};

/// Policy layers, each read from a path.
///
/// A layer with no path, or whose path is not there, says nothing.
#[derive(Debug, Default, Clone)]
pub struct Files {
    paths: BTreeMap<Origin, PathBuf>,
}

impl Files {
    /// Nothing anywhere. Layers are added with [`at`](Self::at).
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Read this layer from this path.
    #[must_use]
    pub fn at(mut self, origin: Origin, path: impl Into<PathBuf>) -> Self {
        self.paths.insert(origin, path.into());
        self
    }

    /// The layers this platform keeps in files, at the places concept 10 names.
    ///
    /// Linux only, and deliberately: a root-owned `/etc/slipcase` taking
    /// precedence over the user's own configuration. There is no per-user
    /// *policy* layer here, because Linux has no mechanism that would
    /// administer one — `Origin::UserPolicy` is Windows and macOS vocabulary,
    /// and inventing a file for it would be offering an administrator a control
    /// that nothing enforces.
    ///
    /// Every other platform gets nothing from this and reads its policy through
    /// its own mechanism (PLAN.md Phases 4 and 5). The exact filenames are
    /// confirmed against the package in Phase 3, which is what installs them.
    #[must_use]
    pub fn for_this_platform() -> Self {
        #[cfg(target_os = "linux")]
        {
            let mut files = Self::none().at(Origin::MachinePolicy, "/etc/slipcase/open.toml");
            if let Some(dir) = config_home() {
                files = files.at(Origin::Configuration, dir.join("slipcase-open/policy.toml"));
            }
            files
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::none()
        }
    }
}

/// `$XDG_CONFIG_HOME`, or the fallback the specification names.
#[cfg(target_os = "linux")]
fn config_home() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x));
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".config"))
}

impl Source for Files {
    fn layer(&self, origin: Origin) -> Read {
        let Some(path) = self.paths.get(&origin) else {
            return Ok(None);
        };
        match std::fs::read_to_string(path) {
            // Not there is not an answer of *no*. A machine with no policy
            // applied has no policy file, which is the common case and not a
            // condition to report.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(cause) => Err(Error::Unreadable {
                path: path.clone(),
                cause,
            }),
            Ok(text) => parse(path, &text).map(Some),
        }
    }
}

fn parse(path: &Path, text: &str) -> std::result::Result<Layer, Error> {
    let bad = |cause: String| Error::Malformed {
        path: path.to_owned(),
        cause,
    };
    let doc: toml_edit::DocumentMut = text.parse().map_err(|e| bad(format!("{e}")))?;

    let list = |key: &str| -> std::result::Result<Option<Vec<String>>, Error> {
        let Some(item) = doc.get(key) else {
            return Ok(None);
        };
        let array = item
            .as_array()
            .ok_or_else(|| bad(format!("`{key}` must be an array of strings")))?;
        array
            .iter()
            .map(|v| {
                v.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| bad(format!("`{key}` must be an array of strings")))
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(Some)
    };

    let flag = |key: &str| -> std::result::Result<Option<bool>, Error> {
        doc.get(key)
            .map(|v| {
                v.as_bool()
                    .ok_or_else(|| bad(format!("`{key}` must be true or false")))
            })
            .transpose()
    };

    // Spelled out rather than derived. There are two values and an
    // administrator who writes a third has made a mistake worth a sentence,
    // where a permissive parser would hand them `replace` and let them find out
    // from the behaviour.
    let mode = match doc.get("mode").map(|v| v.as_str()) {
        None => None,
        Some(Some("replace")) => Some(Mode::Replace),
        Some(Some("append")) => Some(Mode::Append),
        Some(other) => {
            return Err(bad(format!(
                "`mode` must be \"replace\" or \"append\", not {}",
                other.map_or_else(|| "that".to_string(), |s| format!("\"{s}\"")),
            )))
        }
    };

    Ok(Layer {
        allowed: list("allowed")?,
        mode,
        denied: list("denied")?,
        user_may_extend: flag("user_may_extend")?,
        confirm_each_write_back: flag("confirm_each_write_back")?,
    })
}

#[cfg(test)]
mod tests {
    use super::Files;
    use crate::policy::{decide, resolve, Decision, Error, Origin, Source};
    use std::fs;

    fn write(dir: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn a_layer_that_is_not_there_says_nothing() {
        let files = Files::none().at(Origin::MachinePolicy, "/nonexistent/policy.toml");
        assert!(files.layer(Origin::MachinePolicy).unwrap().is_none());
        // And the shipped set still answers.
        assert!(matches!(
            decide(&files, "report.pdf").unwrap(),
            Decision::Open { .. }
        ));
    }

    #[test]
    fn a_layer_reads_every_key_it_carries() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(
            tmp.path(),
            "policy.toml",
            "allowed = [\"pdf\", \"txt\"]\nmode = \"append\"\ndenied = [\"exe\"]\n\
             user_may_extend = false\nconfirm_each_write_back = true\n",
        );
        let files = Files::none().at(Origin::MachinePolicy, p);
        let layer = files.layer(Origin::MachinePolicy).unwrap().unwrap();

        assert_eq!(
            layer.allowed.as_deref(),
            Some(&["pdf".into(), "txt".into()][..])
        );
        assert_eq!(layer.mode, Some(crate::policy::Mode::Append));
        assert_eq!(layer.denied.as_deref(), Some(&["exe".into()][..]));
        assert_eq!(layer.user_may_extend, Some(false));
        assert_eq!(layer.confirm_each_write_back, Some(true));
    }

    #[test]
    fn an_omitted_key_says_nothing_rather_than_no() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(tmp.path(), "policy.toml", "denied = [\"exe\"]\n");
        let layer = Files::none()
            .at(Origin::MachinePolicy, p)
            .layer(Origin::MachinePolicy)
            .unwrap()
            .unwrap();
        assert!(layer.allowed.is_none());
        assert!(layer.user_may_extend.is_none());
    }

    #[test]
    fn a_policy_file_that_will_not_parse_stops_the_decision() {
        // The case concept 10 cares about most. Answering "says nothing" here
        // would permit whatever the file was written to refuse, quietly, for as
        // long as the typo survives.
        let tmp = tempfile::tempdir().unwrap();
        let p = write(tmp.path(), "policy.toml", "denied = [\"exe\"\n");
        let files = Files::none().at(Origin::MachinePolicy, &p);

        match decide(&files, "report.pdf") {
            Err(Error::Malformed { path, .. }) => assert_eq!(path, p),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_key_of_the_wrong_type_is_named_rather_than_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        for (text, want) in [
            (
                "allowed = \"pdf\"\n",
                "`allowed` must be an array of strings",
            ),
            (
                "allowed = [1, 2]\n",
                "`allowed` must be an array of strings",
            ),
            (
                "user_may_extend = \"no\"\n",
                "`user_may_extend` must be true or false",
            ),
            (
                "mode = \"merge\"\n",
                "`mode` must be \"replace\" or \"append\", not \"merge\"",
            ),
            (
                "mode = 3\n",
                "`mode` must be \"replace\" or \"append\", not that",
            ),
        ] {
            let p = write(tmp.path(), "policy.toml", text);
            match Files::none()
                .at(Origin::MachinePolicy, &p)
                .layer(Origin::MachinePolicy)
            {
                Err(Error::Malformed { cause, .. }) => assert_eq!(cause, want, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_machine_list_discards_what_the_user_added_beneath_it() {
        // The point of `replace` being the default. An administrator writing an
        // exhaustive list gets an exhaustive one, and the user's own additions
        // sit beneath it and go — which is the whole reason concept 10 calls
        // append-by-default a silent hole.
        let tmp = tempfile::tempdir().unwrap();
        let machine = write(tmp.path(), "machine.toml", "allowed = [\"txt\"]\n");
        let config = write(
            tmp.path(),
            "config.toml",
            "allowed = [\"dwg\"]\nmode = \"append\"\n",
        );
        let files = Files::none()
            .at(Origin::MachinePolicy, machine)
            .at(Origin::Configuration, config);

        assert!(matches!(
            decide(&files, "notes.txt").unwrap(),
            Decision::Open { .. }
        ));
        assert!(matches!(
            decide(&files, "plan.dwg").unwrap(),
            Decision::NotPermitted { .. }
        ));
        assert!(matches!(
            decide(&files, "report.pdf").unwrap(),
            Decision::NotPermitted { .. }
        ));
        assert!(resolve(&files).unwrap().managed);
    }

    #[test]
    fn a_machine_layer_that_only_denies_leaves_the_user_free_to_add() {
        // An administrator who wants to forbid one thing rather than dictate
        // the whole list writes only `denied`, and everything beneath still
        // stacks.
        let tmp = tempfile::tempdir().unwrap();
        let machine = write(tmp.path(), "machine.toml", "denied = [\"exe\"]\n");
        let config = write(
            tmp.path(),
            "config.toml",
            "allowed = [\"dwg\"]\nmode = \"append\"\n",
        );
        let files = Files::none()
            .at(Origin::MachinePolicy, machine)
            .at(Origin::Configuration, config);

        assert!(matches!(
            decide(&files, "plan.dwg").unwrap(),
            Decision::Open { .. }
        ));
        assert!(matches!(
            decide(&files, "report.pdf").unwrap(),
            Decision::Open { .. }
        ));
        assert!(matches!(
            decide(&files, "setup.exe").unwrap(),
            Decision::Denied { .. }
        ));
    }

    #[test]
    fn a_suppressed_configuration_is_not_read_at_all() {
        // So that a broken file the administrator has already overruled cannot
        // fail a decision it would have played no part in.
        let tmp = tempfile::tempdir().unwrap();
        let machine = write(
            tmp.path(),
            "machine.toml",
            "allowed = [\"txt\"]\nuser_may_extend = false\n",
        );
        let config = write(tmp.path(), "config.toml", "this is not toml at all [[[\n");
        let files = Files::none()
            .at(Origin::MachinePolicy, machine)
            .at(Origin::Configuration, config);

        assert!(matches!(
            decide(&files, "notes.txt").unwrap(),
            Decision::Open { .. }
        ));
        assert!(resolve(&files).unwrap().configuration_suppressed);
    }

    #[test]
    fn a_deny_in_the_users_own_file_still_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = write(tmp.path(), "machine.toml", "allowed = [\"pdf\"]\n");
        let config = write(tmp.path(), "config.toml", "denied = [\"pdf\"]\n");
        let files = Files::none()
            .at(Origin::MachinePolicy, machine)
            .at(Origin::Configuration, config);
        assert!(matches!(
            decide(&files, "report.pdf").unwrap(),
            Decision::Denied { .. }
        ));
    }

    #[test]
    fn the_policy_file_the_package_ships_says_nothing() {
        // Concept 10 makes `/etc/slipcase/open.toml` the highest layer on this
        // platform, so a stray uncommented line in the shipped file is a policy
        // nobody wrote being enforced on every machine that installs the
        // package. The file is documentation until an administrator edits it,
        // and this is what says so.
        let shipped =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/linux/open.toml");
        assert!(shipped.exists(), "{} is not there", shipped.display());

        let files = Files::none().at(Origin::MachinePolicy, &shipped);
        let effective = resolve(&files).unwrap();
        assert!(
            !effective.managed,
            "the shipped file must not read as policy"
        );
        assert!(!effective.confirm_each_write_back);
        assert!(effective.uncomparable_entries.is_empty());

        // And the built-in set is what decides, which is the same statement
        // made from the other end.
        for name in ["report.pdf", "notes.txt", "sheet.xlsx"] {
            assert!(
                matches!(decide(&files, name).unwrap(), Decision::Open { .. }),
                "{name}"
            );
        }
        assert!(matches!(
            decide(&files, "inner.zip").unwrap(),
            Decision::NotPermitted { .. }
        ));
    }
}
