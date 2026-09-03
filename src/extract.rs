//! Putting the payload where the target application can open it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 5, steps 3 and 4: the payload is written into the session's own
//! `payload/` directory (concept 6.4) and then carries whatever the platform
//! records about where the container came from.
//!
//! **A payload the mark could not be carried onto does not survive this
//! function.** `slpc::provenance::carry` fails only where the platform gates
//! opening on a mark, the container has one, and the copy would end up with
//! none — which is to say, only where leaving the file would produce the one
//! thing this step exists to prevent: a document that opens without the warning
//! its origin earned. `slipcase unpack` takes the same line, and this has the
//! stronger obligation, because it is about to hand the file to the system
//! itself rather than leave it on disk for somebody else to double-click.
//!
//! **The window between placing the file and marking it is closed by the
//! directory rather than by ordering.** `Destination::in_place` carries a mark
//! before its rename precisely so that no unmarked file is ever reachable under
//! the final name; `Destination::new` cannot, because a caller naming an output
//! file is creating one and the library does not know which container the bytes
//! came from. Here the file lands inside a session directory that is the user's
//! own and owner-only, nothing has been launched yet, and the mark is on before
//! anything is told the payload exists.

use std::fmt;
use std::io::{Read, Seek};
use std::path::Path;

use slpc::provenance::Mark;
use slpc::{Container, Destination};

use crate::session::Session;

/// Why a payload did not reach the session directory.
#[derive(Debug)]
pub enum Error {
    /// The payload could not be read out of the container: encrypted, stored
    /// with a compression method this build carries no decoder for, or a
    /// container declaring a version it does not implement.
    Unreadable(slpc::Error),
    /// The payload could not be written.
    Write(slpc::Error),
    /// Where the container came from could not be carried onto the payload, so
    /// opening the payload would not raise the warning opening the container
    /// would have.
    ///
    /// Untested, and this says so rather than implying otherwise: reaching it
    /// needs a platform that gates opening on a mark and then refuses the
    /// write, which Linux does not do — it keeps provenance as a note. The
    /// arms that can be reached here are, and `flow::open` re-asks the
    /// filesystem before reporting `payload_removed`, because the sentence has
    /// to be true when it is printed rather than when it was built.
    Unmarked {
        /// What the carry reported.
        cause: slpc::Error,
        /// Whether the payload was removed again. False means it is on disk
        /// and ungated, which the caller has to say out loud.
        payload_removed: bool,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "the payload cannot be read: {e}"),
            Self::Write(e) => write!(f, "the payload could not be written: {e}"),
            Self::Unmarked {
                cause,
                payload_removed,
            } => write!(
                f,
                "where the container came from could not be carried onto its payload: {cause}\n\
                 The payload {}, because opening it would not raise the warning the container \
                 would have.",
                if *payload_removed {
                    "has been removed"
                } else {
                    "could not be removed either and is ungated"
                }
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Write the payload into `session`, then carry the container's mark onto it.
///
/// The container's path is taken from the session's own record rather than
/// passed in again, so that what gets marked from is the resolved path the
/// session was opened against.
///
/// The payload gets the permissions a newly created file would ordinarily
/// receive. SPEC 3 requires that, and forbids applying the bits the archive
/// records — a conformant container may say setuid, and honouring it would put
/// a setuid file on disk. `Destination::new` is what supplies the umask's
/// answer, and it never consults `payload_mode`.
///
/// # Errors
///
/// See [`Error`]. Every variant means the payload must not be launched, and the
/// [`Error::Unmarked`] one means it must not be left behind either.
pub fn extract<R: Read + Seek>(
    container: &mut Container<R>,
    session: &mut Session,
) -> Result<Mark, Error> {
    // Asked before anything is written, so an encrypted payload or one stored
    // by a method this build cannot decode is a sentence rather than a
    // half-written file. `slipcase-desktop` asks the same question before it
    // offers the button.
    container
        .check_payload_readable()
        .map_err(|u| Error::Unreadable(u.into()))?;

    let out = session.payload_path();
    // No-clobber. The session directory was made for this payload a moment ago,
    // so anything already under the name is something else's, and SPEC 3
    // forbids replacing a file the caller did not ask to replace.
    let mut dest = Destination::new(&out, false).map_err(Error::Write)?;
    {
        let mut payload = container.payload().map_err(Error::Unreadable)?;
        std::io::copy(&mut payload, dest.writer()).map_err(|e| Error::Write(e.into()))?;
    }
    dest.commit().map_err(Error::Write)?;

    // The first of the two moments the session and its container are known to
    // agree, and this is where it is established, so this is where it is
    // written down. Recovery needs it to tell an edit that never landed from a
    // container that moved underneath a dead session.
    //
    // Best effort: a session that could not note it asks on recovery instead of
    // acting, which is the cautious direction and no reason to fail an
    // extraction that succeeded.
    if let Ok(crc) = container.payload_crc() {
        let _ = session.note_agreement(crc);
    }

    match slpc::provenance::carry(&session.record().container, &out) {
        Ok(mark) => Ok(mark),
        Err(cause) => Err(Error::Unmarked {
            payload_removed: remove(&out),
            cause,
        }),
    }
}

/// Take the payload back off disk. Reported rather than propagated: the caller
/// is already failing, and whether the ungated file is still there changes what
/// the sentence has to say rather than whether there is one.
fn remove(at: &Path) -> bool {
    std::fs::remove_file(at).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{extract, Error};
    use crate::session;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A container on disk holding `payload` under `name`.
    fn container(at: &Path, name: &str, payload: &[u8]) -> PathBuf {
        let doc: slpc::toml_edit::DocumentMut =
            format!("slipcase_version = \"1.0\"\n\n[payload]\nfile = \"{name}\"\n")
                .parse()
                .unwrap();
        let path = at.join(format!("{name}.slpc"));
        let out = fs::File::create(&path).unwrap();
        slpc::pack_reader(name, payload, doc, out).unwrap();
        path
    }

    fn open(at: &Path) -> slpc::Container<fs::File> {
        slpc::Container::open(at).unwrap()
    }

    #[test]
    fn the_payload_lands_under_its_own_name_with_its_own_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"%PDF-1.7 not really\n");

        let mut s = session::create(&root, &c, "report.pdf").unwrap();
        extract(&mut open(&c), &mut s).unwrap();

        assert_eq!(s.payload_path().file_name().unwrap(), "report.pdf");
        assert_eq!(
            fs::read(s.payload_path()).unwrap(),
            b"%PDF-1.7 not really\n"
        );
    }

    #[test]
    fn a_zero_length_payload_is_written_rather_than_refused() {
        // SPEC 2.3 permits one, and a container holding nothing is still a
        // container somebody wants opened.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "empty.txt", b"");

        let mut s = session::create(&root, &c, "empty.txt").unwrap();
        extract(&mut open(&c), &mut s).unwrap();
        assert_eq!(fs::read(s.payload_path()).unwrap(), b"");
    }

    #[test]
    fn the_payload_is_the_only_thing_written_into_the_payload_directory() {
        // Concept 6.1 reads anything else appearing there as the target
        // application's work, and that inference is what the sibling signal
        // rests on. SPEC 3 says the same from the other side: nothing but the
        // payload is written when extracting.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"x");

        let mut s = session::create(&root, &c, "report.pdf").unwrap();
        extract(&mut open(&c), &mut s).unwrap();

        let mut found: Vec<_> = fs::read_dir(s.payload_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        found.sort();
        assert_eq!(found, ["report.pdf"]);
    }

    #[test]
    fn extracting_twice_into_one_session_refuses_rather_than_replaces() {
        // SPEC 3 forbids replacing a file the caller did not ask to replace,
        // and a second extraction into a live session would be overwriting a
        // payload somebody may be editing.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let mut s = session::create(&root, &c, "report.pdf").unwrap();
        extract(&mut open(&c), &mut s).unwrap();
        fs::write(s.payload_path(), b"edited by somebody").unwrap();

        assert!(matches!(
            extract(&mut open(&c), &mut s),
            Err(Error::Write(_))
        ));
        assert_eq!(fs::read(s.payload_path()).unwrap(), b"edited by somebody");
    }

    #[test]
    fn a_container_that_arrived_from_elsewhere_marks_the_payload_it_yields() {
        // The reason this step exists. Unpacking without it is laundering: the
        // payload reaches its handler as something this machine made, and the
        // warning the platform would have shown never appears.
        //
        // The answer differs by platform and the assertion is that it is not
        // `Silent` rather than which of the others it is. Linux keeps
        // provenance as a note rather than a gate, so `Noted` is the right
        // answer there and would be the wrong one on Windows.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"%PDF");

        // Not skipped quietly where the filesystem will not hold an attribute:
        // a test that no-ops on the machine it runs on proves nothing, and this
        // is the arm that matters most.
        assert!(
            testsupport::mark_as_downloaded(&c),
            "this filesystem would not hold the mark, so the carry is untested here"
        );

        let mut s = session::create(&root, &c, "report.pdf").unwrap();
        let mark = extract(&mut open(&c), &mut s).unwrap();
        assert_ne!(mark, slpc::provenance::Mark::Silent);
        assert!(slpc::provenance::arrived_from_elsewhere(&s.payload_path()));
    }

    #[test]
    fn a_container_that_says_nothing_yields_a_payload_that_says_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"%PDF");

        let mut s = session::create(&root, &c, "report.pdf").unwrap();
        let mark = extract(&mut open(&c), &mut s).unwrap();
        assert_eq!(mark, slpc::provenance::Mark::Silent);
    }

    #[test]
    fn an_unreadable_payload_is_refused_before_anything_is_written() {
        // An encrypted payload, which SPEC 2.5 forbids rejecting the container
        // over and which this build cannot decode. The refusal is a sentence
        // rather than a half-written file in the session directory.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = tmp.path().join("locked.slpc");
        fs::write(&c, encrypted_container()).unwrap();

        let mut s = session::create(&root, &c, "secret.pdf").unwrap();
        assert!(matches!(
            extract(&mut open(&c), &mut s),
            Err(Error::Unreadable(_))
        ));
        assert!(!s.payload_path().exists());
        assert_eq!(fs::read_dir(s.payload_dir()).unwrap().count(), 0);
    }

    /// A container whose payload sets general purpose bit 0, which is what
    /// `check_payload_readable` refuses. Built by hand, because the writer will
    /// not produce one.
    fn encrypted_container() -> Vec<u8> {
        let doc = "slipcase_version = \"1.0\"\n\n[payload]\nfile = \"secret.pdf\"\n";
        let mut bytes = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file(slpc::METADATA_MEMBER, opts).unwrap();
            std::io::Write::write_all(&mut w, doc.as_bytes()).unwrap();
            w.start_file("secret.pdf", opts).unwrap();
            std::io::Write::write_all(&mut w, b"ciphertext").unwrap();
            w.finish().unwrap();
        }
        // Set general purpose bit 0 in both the local header and the central
        // directory entry, which is where `entries_of` reads it from.
        set_encrypted_flag(&mut bytes);
        bytes
    }

    /// Turn on general purpose bit 0 on the payload's local file header and its
    /// central directory entry, and on neither of the metadata member's.
    ///
    /// The name is read at the offset the header says it is at, rather than
    /// searched for. Searching finds `secret.pdf` after the metadata header too,
    /// which sets the flag on the metadata member and makes the container
    /// undetermined under SPEC 2.2 instead of holding an unreadable payload —
    /// a different refusal, arriving before the one this test is about.
    fn set_encrypted_flag(bytes: &mut [u8]) {
        // (signature, flag offset, name-length offset, name offset)
        for (signature, flag, len_at, name_at) in [
            ([0x50u8, 0x4b, 0x03, 0x04], 6usize, 26usize, 30usize),
            ([0x50, 0x4b, 0x01, 0x02], 8, 28, 46),
        ] {
            for i in 0..bytes.len().saturating_sub(name_at) {
                if bytes[i..i + 4] != signature {
                    continue;
                }
                let n = u16::from_le_bytes([bytes[i + len_at], bytes[i + len_at + 1]]) as usize;
                let from = i + name_at;
                if bytes.get(from..from + n) == Some(b"secret.pdf".as_slice()) {
                    bytes[i + flag] |= 0x01;
                }
            }
        }
    }
}
