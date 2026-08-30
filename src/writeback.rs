//! Putting an edited payload back into the container.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 7. Repack to a temporary container beside the original and swap it
//! over: never modify in place, because an interruption mid-write corrupts the
//! only copy.
//!
//! **`slpc::Destination::in_place` is the swap, and reimplementing it would be
//! a regression.** It resolves the path first, so a container reached through a
//! symbolic link is replaced rather than the link; it takes the replacement's
//! permissions from the file being replaced rather than from the umask; and its
//! `commit` carries the platform's mark — Mark of the Web, `com.apple.quarantine`
//! — onto the replacement *before* the rename, failing the commit if it cannot,
//! so a marked container is never replaced by an unmarked one.
//!
//! The naive version looks correct and fails quietly. A plain `std::fs::rename`
//! is `MoveFileEx`, which carries over neither the target's ACLs nor its
//! alternate data streams, and Mark of the Web is an alternate data stream. The
//! repack-and-rename anyone would write first strips the container's trust zone
//! on the first save, with no error and no visible symptom.
//!
//! **The container is read back before it replaces anything.** `slipcase
//! repack` does this too, and this has more reason to: it runs unattended and
//! repeatedly, so a fault that would cost one person one container there costs
//! every save here.
//!
//! **The metadata member is not touched.** SPEC 5 defines no checksum or fixity
//! key and 2.2 assigns no meaning to any key beyond `slipcase_version` and
//! `payload.file`, so a changed payload falsifies nothing a conformant container
//! says about itself. A producer may have recorded its own size or digest under
//! a private key permitted by 2.5, and since the specification gives those keys
//! no meaning this cannot know which, what it covers, or how it is encoded.
//! Guessing is worse than leaving it: a wrong digest is a false claim, where a
//! stale one is at least a claim whose provenance is the producer's.

use std::fmt;
use std::fs::File;

use slpc::Destination;

use crate::session::Session;

/// Why an edit did not reach the container.
#[derive(Debug)]
pub enum Error {
    /// The payload could not be read out of the session directory.
    Payload(std::io::Error),
    /// The container could not be read, or is no longer where the session
    /// recorded it. Concept 6.4: a container may move or go while a session
    /// runs, and this is not a failure of the edit.
    Container(std::io::Error),
    /// The file at the recorded path is not the container this session was
    /// opened against — its payload goes by another name. Writing back would
    /// rename the payload of a container somebody else may be holding, so it
    /// refuses. Concept 6.3 asks the same question on the recovery side; this
    /// is the guard on the acting side, and it belongs here because it is a
    /// safety property of the write-back rather than an optimisation in
    /// whatever called it.
    ContainerChanged {
        /// What the session recorded.
        recorded: String,
        /// What the file at that path says now.
        found: String,
    },
    /// The repack itself failed. Nothing was replaced.
    Repack(slpc::Error),
    /// What the repack produced was not a conformant container, so it was not
    /// allowed to replace one. Nothing was changed.
    WouldNotBeConformant(String),
    /// The replacement could not be put in place. Includes the case concept 7
    /// cares most about: the container carries a mark that could not be carried
    /// onto its replacement, which stops the commit rather than silently
    /// laundering it.
    Swap(slpc::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Payload(e) => write!(f, "the edited payload could not be read: {e}"),
            Self::Container(e) => write!(f, "the container could not be opened: {e}"),
            Self::ContainerChanged { recorded, found } => write!(
                f,
                "the container now holds {found} rather than {recorded}, so this is not the \
                 container this session was opened against. Nothing was changed."
            ),
            Self::Repack(e) => write!(f, "the container could not be rebuilt: {e}"),
            Self::WouldNotBeConformant(v) => write!(
                f,
                "the container this would have written is {v}. Nothing was changed."
            ),
            Self::Swap(e) => write!(
                f,
                "the rebuilt container could not replace the original: {e}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Put the session's payload back into its container, and count it.
///
/// Unrecognised members survive, which `Repack` already guarantees and SPEC 3
/// requires. The payload keeps the name the session recorded, so a container
/// whose `payload.file` says one thing does not quietly acquire another.
///
/// # Errors
///
/// See [`Error`]. In every variant the original container is untouched.
pub fn write_back(session: &mut Session) -> Result<(), Error> {
    let container = session.record().container.clone();
    let payload_path = session.payload_path();

    let edited = File::open(&payload_path).map_err(Error::Payload)?;

    // Asked before anything is written. A different container at the recorded
    // path is not a container to repack into: the payload would be renamed to
    // this session's `payload.file`, which is a change nobody asked for made to
    // a file this session was never opened against.
    let found = slpc::Container::open(&container)
        .map_err(|e| match e {
            slpc::Error::Io(e) => Error::Container(e),
            other => Error::Repack(other),
        })?
        .payload_name()
        .to_string();
    if found != session.record().payload {
        return Err(Error::ContainerChanged {
            recorded: session.record().payload.clone(),
            found,
        });
    }

    let source = File::open(&container).map_err(Error::Container)?;

    // `in_place` resolves the path and reads the mode off the file it is going
    // to replace, so it is opened before the source is consumed rather than
    // after — the two are independent, and doing it here keeps the failure that
    // means *this directory is not writable* ahead of the work.
    let mut out = Destination::in_place(&container).map_err(Error::Swap)?;

    // `write` consumes the repack and with it the source handle, so the
    // container is closed before the commit renames over it. That ordering is
    // not cosmetic on Windows, where replacing a file somebody still holds open
    // is the case that fails.
    slpc::Repack::new(source)
        .payload(&session.record().payload, edited)
        .write(out.writer())
        .map_err(Error::Repack)?;

    let verdict = slpc::validate(out.written().map_err(Error::Repack)?).map_err(Error::Repack)?;
    if !verdict.is_conformant() {
        return Err(Error::WouldNotBeConformant(verdict.to_string()));
    }
    out.commit().map_err(Error::Swap)?;

    session.note_write_back().map_err(Error::Payload)
}

#[cfg(test)]
mod tests {
    use super::{write_back, Error};
    use crate::{extract, session};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn container_with(at: &Path, name: &str, payload: &[u8], extra: &str) -> PathBuf {
        let doc: slpc::toml_edit::DocumentMut =
            format!("slipcase_version = \"1.0\"\n{extra}\n[payload]\nfile = \"{name}\"\n")
                .parse()
                .unwrap();
        let path = at.join(format!("{name}.slpc"));
        slpc::pack_reader(name, payload, doc, fs::File::create(&path).unwrap()).unwrap();
        path
    }

    /// A session with the payload already extracted into it.
    fn opened(root: &Path, container: &Path, name: &str) -> session::Session {
        let s = session::create(root, container, name).unwrap();
        extract::extract(&mut slpc::Container::open(container).unwrap(), &s).unwrap();
        s
    }

    fn payload_of(container: &Path) -> Vec<u8> {
        let mut c = slpc::Container::open(container).unwrap();
        let mut out = Vec::new();
        std::io::copy(&mut c.payload().unwrap(), &mut out).unwrap();
        out
    }

    #[test]
    fn an_edit_reaches_the_container() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");

        let mut s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        write_back(&mut s).unwrap();

        assert_eq!(payload_of(&c), b"edited");
    }

    #[test]
    fn the_metadata_member_is_returned_byte_for_byte() {
        // Concept 7. SPEC 5 defines no fixity key and 2.2 gives no meaning to
        // any other, so a changed payload falsifies nothing — and a private key
        // a producer used under 2.5 is one this cannot interpret, so leaving it
        // is the only honest option.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let extra = "producer = \"something else\"\nsha256 = \"stale after this edit\"\n";
        let c = container_with(tmp.path(), "report.pdf", b"first", extra);

        let before = slpc::Container::open(&c).unwrap().metadata_bytes().to_vec();
        let mut s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        write_back(&mut s).unwrap();

        let after = slpc::Container::open(&c).unwrap().metadata_bytes().to_vec();
        assert_eq!(before, after);
    }

    #[test]
    fn the_payload_keeps_the_name_the_session_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");

        let mut s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        write_back(&mut s).unwrap();

        assert_eq!(
            slpc::Container::open(&c).unwrap().payload_name(),
            "report.pdf"
        );
    }

    #[test]
    fn each_write_back_is_counted_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");

        let mut s = opened(&root, &c, "report.pdf");
        for n in 1..=3 {
            fs::write(s.payload_path(), format!("edit {n}")).unwrap();
            write_back(&mut s).unwrap();
            assert_eq!(session::scan(&root).unwrap()[0].record().write_backs, n);
        }
        assert_eq!(payload_of(&c), b"edit 3");
    }

    #[test]
    fn writing_back_repeatedly_leaves_one_container_and_no_debris() {
        // The temporary the swap goes through lives beside the container,
        // because `ReplaceFileW` and `rename` both need the same volume. It
        // must not survive the commit.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");

        let mut s = opened(&root, &c, "report.pdf");
        for n in 0..5 {
            fs::write(s.payload_path(), format!("{n}")).unwrap();
            write_back(&mut s).unwrap();
        }

        let beside: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "sessions")
            .collect();
        assert_eq!(beside, ["report.pdf.slpc"]);
    }

    #[test]
    fn a_container_that_went_away_is_reported_rather_than_recreated() {
        // Concept 6.4: a container may move or be deleted while a session runs.
        // Writing a fresh one where it used to be would be inventing a file the
        // user deleted.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");

        let mut s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        fs::remove_file(&c).unwrap();

        assert!(matches!(write_back(&mut s), Err(Error::Container(_))));
        assert!(!c.exists());
    }

    #[test]
    fn a_missing_payload_is_reported_and_the_container_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");

        let mut s = opened(&root, &c, "report.pdf");
        fs::remove_file(s.payload_path()).unwrap();

        assert!(matches!(write_back(&mut s), Err(Error::Payload(_))));
        assert_eq!(payload_of(&c), b"first");
    }

    #[test]
    fn a_container_reached_through_a_link_replaces_the_file_and_not_the_link() {
        // `Destination::in_place` resolves first. Without that the link becomes
        // a regular file and the container it pointed at is orphaned.
        #[cfg(unix)]
        {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("sessions");
            let real = container_with(tmp.path(), "report.pdf", b"first", "");
            let link = tmp.path().join("link.slpc");
            std::os::unix::fs::symlink(&real, &link).unwrap();

            let mut s = opened(&root, &link, "report.pdf");
            fs::write(s.payload_path(), b"edited").unwrap();
            write_back(&mut s).unwrap();

            assert_eq!(payload_of(&real), b"edited");
        }
    }

    #[test]
    fn a_marked_container_is_still_marked_after_a_write_back() {
        // The defect concept 7 exists to name: a plain rename is `MoveFileEx`,
        // which carries neither ACLs nor alternate data streams, and Mark of
        // the Web is an alternate data stream. `Destination::in_place` carries
        // it before the rename, so this survives — and fails the commit rather
        // than launder it if it cannot.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container_with(tmp.path(), "report.pdf", b"first", "");
        assert!(
            testsupport::mark_as_downloaded(&c),
            "this filesystem would not hold the mark, so the carry is untested here"
        );

        let mut s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        write_back(&mut s).unwrap();

        assert!(slpc::provenance::arrived_from_elsewhere(&c));
        assert_eq!(payload_of(&c), b"edited");
    }
}
