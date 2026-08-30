//! What counts as a payload's extension, and how it is compared against policy.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 5.1 puts policy on the extension rather than on a sniffed content
//! type, because `ShellExecuteEx`, `open` and `xdg-open` all resolve a handler
//! from the name and none of them reads the bytes. The extension is what
//! decides what runs, so it is what policy has to be written against.
//!
//! Two questions, and they are not the same one. [`of`] is what the extension
//! *is*, in the case it was written in, which is what gets shown to a person
//! and what the platform is handed. [`policy_key`] is what it is compared
//! *as*, which folds and narrows.

use std::path::Path;

/// The payload's extension, in the case the container spells it.
///
/// The rule is `slipcase-desktop`'s, deliberately: it takes the extension with
/// `Path::extension`, and two products disagreeing about what a payload's
/// extension is would be a defect visible to anyone with both installed. So
/// `archive.tar.gz` is `gz` and never `tar.gz`, and `.bashrc` is a hidden file
/// with no extension rather than an extension of `bashrc`.
///
/// `None` where there is nothing usable. Concept 5.1 refuses that case rather
/// than launching it, because a payload the platform has no registration for
/// raises the Open With dialog, which offers the user every executable on the
/// machine inside a flow they read as *open the document*.
#[must_use]
pub fn of(payload_name: &str) -> Option<&str> {
    // A conformant container's payload name has been through SPEC 2.3, which
    // excludes both separators. Checked rather than assumed, the way the
    // sibling checks it: this decides what gets executed, and the cost of
    // asking is nothing.
    if payload_name.is_empty() || payload_name.contains(['/', '\\']) {
        return None;
    }
    match Path::new(payload_name).extension()?.to_str()? {
        "" => None,
        e => Some(e),
    }
}

/// The extension as policy compares it: ASCII-folded, and only where it is
/// ASCII alphanumeric to begin with.
///
/// **Folding has to match how the platform resolves the handler**, or policy
/// permits one thing and the shell opens another, and the disagreement is the
/// defect. Windows registry keys are case-insensitive. shared-mime-info
/// lowercases a filename before matching a glob, unless that glob is marked
/// case-sensitive. Both fold ASCII, and neither does anything more elaborate.
///
/// Full Unicode case folding would draw distinctions the launch path does not —
/// U+212A KELVIN SIGN folds to `k` — and every such divergence is somewhere the
/// allow list and the shell disagree about what a name says. So an extension
/// carrying anything but ASCII letters and digits is not comparable, answers
/// `None`, and falls into the refusal `of` returning `None` already gets.
/// Registered types are ASCII in practice, so what that excludes is small and
/// the refusal can be explained.
///
/// The deny list is compared through this too, and reaches the same set.
/// Nothing should be refusable by a rule the allow list could not have written.
#[must_use]
pub fn policy_key(payload_name: &str) -> Option<String> {
    let e = of(payload_name)?;
    e.chars()
        .all(|c| c.is_ascii_alphanumeric())
        .then(|| e.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{of, policy_key};

    #[test]
    fn takes_the_last_extension_and_not_the_whole_tail() {
        // `Path::extension`'s answer, and the sibling's. A rule that returned
        // `tar.gz` would be a rule the platform does not use to pick a handler.
        assert_eq!(of("archive.tar.gz"), Some("gz"));
    }

    #[test]
    fn a_name_that_is_all_name_has_no_extension() {
        assert_eq!(of("README"), None);
    }

    #[test]
    fn a_leading_dot_makes_a_hidden_file_rather_than_an_extension() {
        // Asking about `bashrc` would be asking about the wrong thing.
        assert_eq!(of(".bashrc"), None);
    }

    #[test]
    fn a_trailing_dot_is_not_an_extension() {
        // `Path::extension` answers `Some("")` here, which would be handed to
        // the platform as an extension of nothing.
        assert_eq!(of("report."), None);
    }

    #[test]
    fn a_name_carrying_a_separator_is_refused_rather_than_split() {
        // SPEC 2.3 excludes both, so this is unreachable through a conformant
        // container. It is checked because what it guards is execution.
        assert_eq!(of("etc/passwd.pdf"), None);
        assert_eq!(of("..\\windows\\system32\\x.dll"), None);
    }

    #[test]
    fn the_policy_key_folds_ascii_case() {
        // A container spelling it `REPORT.PDF` must not slip past a deny list
        // saying `pdf`, and must not be refused by an allow list saying the
        // same, on a platform whose own lookup is case-insensitive.
        assert_eq!(policy_key("REPORT.PDF").as_deref(), Some("pdf"));
        assert_eq!(policy_key("report.PdF").as_deref(), Some("pdf"));
    }

    #[test]
    fn an_extension_that_is_not_ascii_alphanumeric_is_not_comparable() {
        // Folding this would need Unicode rules the shell does not apply, so
        // there is no answer that policy and the platform would agree on.
        assert_eq!(policy_key("report.pdf\u{212a}"), None);
        assert_eq!(policy_key("notes.tëxt"), None);
    }

    #[test]
    fn digits_are_comparable_because_real_extensions_carry_them() {
        assert_eq!(policy_key("clip.mp3").as_deref(), Some("mp3"));
        assert_eq!(policy_key("page.7z").as_deref(), Some("7z"));
    }

    #[test]
    fn the_display_form_keeps_the_case_the_container_spelled() {
        // What a person is shown, and what the platform is handed, is the
        // container's own spelling. Only the comparison folds.
        assert_eq!(of("REPORT.PDF"), Some("PDF"));
    }
}
