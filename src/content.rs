//! The one content check, and everything it deliberately does not do.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 5.1 settles that policy keys on the extension, because that is what
//! `ShellExecuteEx`, `open` and `xdg-open` resolve a handler from and none of
//! them reads the bytes. Sniffing a content type and checking policy against it
//! would be checking a value with no bearing on what executes.
//!
//! What survives is narrow and is not policy. It reports a payload whose bytes
//! are an executable image or a script under a name that claims neither — the
//! shape of a phishing attachment — and it refuses nothing. The extension
//! governs what runs, so a PDF reader handed a PE image fails on it harmlessly,
//! and a refusal here would be asserting a control this path does not carry.
//! The person is told and then decides.
//!
//! **It is a handful of magic numbers and not a type table.** A `.docx`
//! sniffing as a ZIP is noise and goes unmentioned, so nothing here has to tell
//! OOXML from a bare archive, which is the problem that made the sniffing
//! design collapse in the first place.

/// What the bytes are, where they are something that runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Executable {
    /// `MZ`. A Windows executable image: `.exe`, `.dll`, and the rest.
    Pe,
    /// `\x7fELF`. A Linux or BSD executable or shared object.
    Elf,
    /// A Mach-O image, in either byte order and either width.
    MachO,
    /// `#!`. A script naming its own interpreter, which is what makes it run.
    Script,
}

impl Executable {
    /// What to call it in the sentence shown to a person.
    #[must_use]
    pub fn describes(self) -> &'static str {
        match self {
            Self::Pe => "a Windows executable",
            Self::Elf => "a Linux executable",
            Self::MachO => "a macOS executable",
            Self::Script => "a script",
        }
    }
}

/// How many bytes of a payload this needs. Four for every magic number here,
/// and two for a shebang.
pub const HEAD: usize = 4;

/// What the leading bytes are, where they are something that runs.
///
/// **`cafebabe` is missing on purpose.** It is a Mach-O universal binary and it
/// is also a Java class file, and telling them apart means reading the field
/// after it and deciding whether it is an architecture count or a version. A
/// false positive here is a warning shown to somebody about a payload that is
/// fine, which costs more than missing a fat binary — and a fat binary's
/// members are Mach-O, so the single-architecture form is the common one and is
/// caught.
#[must_use]
pub fn executable(head: &[u8]) -> Option<Executable> {
    match head {
        [b'M', b'Z', ..] => Some(Executable::Pe),
        [0x7f, b'E', b'L', b'F', ..] => Some(Executable::Elf),
        // Thin Mach-O. The last byte is the width — `ce` for 32-bit, `cf` for
        // 64 — and the two arms are the two byte orders it can be written in.
        [0xfe, 0xed, 0xfa, 0xce | 0xcf, ..] | [0xce | 0xcf, 0xfa, 0xed, 0xfe, ..] => {
            Some(Executable::MachO)
        }
        [b'#', b'!', ..] => Some(Executable::Script),
        _ => None,
    }
}

/// Whether the payload is something that runs while its name says otherwise.
///
/// `None` where the bytes are not executable, and `None` where they are and the
/// extension already says so — a `.exe` that is a PE image is not
/// misrepresenting itself, whatever policy goes on to decide about it.
///
/// The extension is the folded one from [`crate::extension::policy_key`]. An
/// extension too exotic to fold is not on the list below and so does not
/// suppress the report, which is the safe direction: the payload is executable
/// and the name says something nobody can compare.
#[must_use]
pub fn misrepresents(head: &[u8], policy_key: Option<&str>) -> Option<Executable> {
    let what = executable(head)?;
    match policy_key {
        Some(k) if EXPECTED.contains(&k) => None,
        _ => Some(what),
    }
}

/// Extensions where executable content is what a person would expect.
///
/// Not a type table and not a policy list — nothing is permitted or refused by
/// being here. It exists so that the warning does not fire on a payload that is
/// exactly what its name says, and it is short because it only has to cover the
/// names people actually use for things that run. An extension missing from it
/// costs a warning shown about an honest payload, which is the direction to err
/// in.
const EXPECTED: &[&str] = &[
    // Windows
    "exe", "dll", "com", "scr", "sys", "cpl", "ocx", "drv", "efi", // Unix
    "so", "o", "a", "bin", "elf", "ko", // macOS
    "dylib", "bundle", // scripts, for the shebang arm
    "sh", "bash", "zsh", "csh", "ksh", "fish", "py", "pl", "rb", "lua", "tcl", "awk", "sed", "r",
    "ps1",
];

#[cfg(test)]
mod tests {
    use super::{executable, misrepresents, Executable};

    #[test]
    fn recognises_the_four_things_that_run() {
        assert_eq!(executable(b"MZ\x90\x00"), Some(Executable::Pe));
        assert_eq!(executable(b"\x7fELF"), Some(Executable::Elf));
        assert_eq!(executable(b"\xcf\xfa\xed\xfe"), Some(Executable::MachO));
        assert_eq!(executable(b"#!/bin/sh"), Some(Executable::Script));
    }

    #[test]
    fn mach_o_is_recognised_in_both_orders_and_both_widths() {
        for magic in [
            b"\xfe\xed\xfa\xce",
            b"\xce\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xcf\xfa\xed\xfe",
        ] {
            assert_eq!(executable(magic), Some(Executable::MachO), "{magic:x?}");
        }
    }

    #[test]
    fn a_universal_binary_is_not_reported() {
        // `cafebabe` is a Java class file too, and a warning shown about an
        // honest payload costs more than missing a fat binary whose members
        // are Mach-O anyway. See the note on `executable`.
        assert_eq!(executable(b"\xca\xfe\xba\xbe"), None);
    }

    #[test]
    fn a_pdf_is_not_something_that_runs() {
        assert_eq!(executable(b"%PDF"), None);
    }

    #[test]
    fn a_zip_is_not_something_that_runs() {
        // The case that sank the sniffing design: this is a `.docx`, an `.odt`,
        // a `.jar` and a bare archive, and nothing here has to know which.
        assert_eq!(executable(b"PK\x03\x04"), None);
    }

    #[test]
    fn short_input_answers_rather_than_panicking() {
        // A zero-length payload is conformant under SPEC 2.3, and a one-byte
        // one is a slice every pattern here is longer than.
        assert_eq!(executable(b""), None);
        assert_eq!(executable(b"M"), None);
        assert_eq!(executable(b"\x7fEL"), None);
        // Two bytes are enough for a shebang and not for the rest.
        assert_eq!(executable(b"#!"), Some(Executable::Script));
    }

    #[test]
    fn an_executable_wearing_a_documents_name_is_reported() {
        assert_eq!(
            misrepresents(b"MZ\x90\x00", Some("pdf")),
            Some(Executable::Pe)
        );
    }

    #[test]
    fn an_executable_wearing_its_own_name_is_not() {
        assert_eq!(misrepresents(b"MZ\x90\x00", Some("exe")), None);
        assert_eq!(misrepresents(b"\x7fELF", Some("so")), None);
        assert_eq!(misrepresents(b"#!/bin/sh", Some("sh")), None);
    }

    #[test]
    fn a_document_is_never_reported_whatever_it_is_called() {
        assert_eq!(misrepresents(b"%PDF", Some("pdf")), None);
        assert_eq!(misrepresents(b"%PDF", Some("exe")), None);
        assert_eq!(misrepresents(b"PK\x03\x04", Some("docx")), None);
    }

    #[test]
    fn an_extension_too_exotic_to_fold_does_not_suppress_the_report() {
        // `policy_key` answers `None` for one that is not ASCII alphanumeric.
        // The payload is executable and the name says something nothing can
        // compare, which is the case to report rather than the case to excuse.
        assert_eq!(misrepresents(b"MZ\x90\x00", None), Some(Executable::Pe));
    }
}
