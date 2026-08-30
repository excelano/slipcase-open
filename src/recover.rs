//! What to do with a session that outlived the process holding it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 6.3. **Recovery never writes back on its own.** The tool was not
//! watching when the process died, so it cannot tell a complete save from a
//! half-written one, and putting a truncated save over the container is the
//! worst outcome this tool is capable of producing. Everything here reports;
//! nothing here acts.
//!
//! What it can do is narrow the question without keeping a record of its own.
//! The ZIP central directory already stores a CRC-32 for the payload member, so
//! recovery computes the CRC of the extracted payload and compares. Equal means
//! nothing was lost. Different means an edit never landed.
//!
//! **Comparing against the container beats recording a digest.** A recorded
//! value is a second copy of a fact and can drift from it, and the moment it
//! gets consulted is after a crash, which is when a session record is least
//! trustworthy. The container's own value needs nothing maintaining it:
//! repacking recomputes it, so the comparison stays correct across every
//! write-back in a session as a side effect of the write-backs themselves.
//!
//! **It is change detection and never fixity.** The question is whether the
//! file changed, not whether it can be proved untampered — anybody able to
//! write into the user's own owner-only session directory can do worse than
//! forge a checksum. SPEC 5 declined to define a fixity key and nothing here
//! becomes one.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::session::Session;

/// What a session left behind turned out to be.
#[derive(Debug)]
pub enum State {
    /// No payload in the session directory. The session died between being
    /// created and being filled, so there is nothing to recover and nothing to
    /// ask about.
    NothingExtracted,
    /// The payload still matches the one in the container. Nothing was lost:
    /// clean up and say nothing.
    Unchanged,
    /// The payload differs from the one in the container, so an edit never
    /// landed. Whether it is a complete save or a truncated one cannot be told
    /// apart, which is why this is offered and never applied.
    Edited,
    /// The container is no longer where the session recorded it. Concept 6.4
    /// requires surviving this rather than failing at the rename: the payload
    /// is still here, and the person can be offered somewhere else to put it.
    ContainerGone,
    /// Something is at the recorded path, and it is not the container this
    /// session was opened against — its payload goes by another name. Writing
    /// back would rename the payload of a container somebody else's session may
    /// be holding.
    ContainerChanged {
        /// What the session recorded.
        recorded: String,
        /// What the file at that path says now.
        found: String,
    },
    /// The container is there and cannot be read, or the payload cannot be. A
    /// question for a person rather than an answer.
    Unreadable(String),
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingExtracted => write!(f, "nothing was extracted"),
            Self::Unchanged => write!(f, "unchanged since it came out of the container"),
            Self::Edited => write!(f, "edited, and the edit never reached the container"),
            Self::ContainerGone => write!(f, "the container is no longer where it was"),
            Self::ContainerChanged { recorded, found } => {
                write!(f, "the container now holds {found} rather than {recorded}")
            }
            Self::Unreadable(e) => write!(f, "cannot be read: {e}"),
        }
    }
}

impl State {
    /// Whether a person has to be asked about this one.
    ///
    /// The quiet cases clean themselves up. Everything else is a question,
    /// because concept 6.3 gives recovery no authority to answer any of them.
    #[must_use]
    pub fn needs_a_person(&self) -> bool {
        !matches!(self, Self::NothingExtracted | Self::Unchanged)
    }
}

/// What became of a session left behind.
#[must_use]
pub fn state(session: &Session) -> State {
    let payload = session.payload_path();
    if !payload.is_file() {
        return State::NothingExtracted;
    }

    let container = match slpc::Container::open(&session.record().container) {
        Ok(c) => c,
        Err(slpc::Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            return State::ContainerGone
        }
        Err(e) => return State::Unreadable(e.to_string()),
    };

    // Asked before the CRC, because a container holding a different payload
    // answers the wrong question rather than answering it wrongly.
    if container.payload_name() != session.record().payload {
        return State::ContainerChanged {
            recorded: session.record().payload.clone(),
            found: container.payload_name().to_string(),
        };
    }

    let (stored, made) = match (container.payload_crc(), crc_of(&payload)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) => return State::Unreadable(e.to_string()),
        (_, Err(e)) => return State::Unreadable(e.to_string()),
    };

    if stored == made {
        State::Unchanged
    } else {
        State::Edited
    }
}

/// The CRC-32 of a file on disk, computed the way the archive computed the one
/// it recorded.
///
/// # Errors
///
/// Where the file cannot be read.
pub fn crc_of(path: &Path) -> io::Result<u32> {
    let mut file = File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    // Streamed rather than read whole: a payload may be any size, and holding
    // one in memory to checksum it would make recovery fail on the containers
    // most worth recovering.
    // On the heap. Sixty-four kilobytes of stack is a lot to ask of a thread
    // whose size this crate does not choose.
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buf)? {
            0 => break,
            n => hasher.update(&buf[..n]),
        }
    }
    Ok(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{crc_of, state, State};
    use crate::{extract, session, writeback};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn container(at: &Path, name: &str, payload: &[u8]) -> PathBuf {
        let doc: slpc::toml_edit::DocumentMut =
            format!("slipcase_version = \"1.0\"\n\n[payload]\nfile = \"{name}\"\n")
                .parse()
                .unwrap();
        let path = at.join(format!("{name}.slpc"));
        slpc::pack_reader(name, payload, doc, fs::File::create(&path).unwrap()).unwrap();
        path
    }

    fn opened(root: &Path, c: &Path, name: &str) -> session::Session {
        let s = session::create(root, c, name).unwrap();
        extract::extract(&mut slpc::Container::open(c).unwrap(), &s).unwrap();
        s
    }

    #[test]
    fn a_payload_nobody_touched_is_the_quiet_case() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        assert!(matches!(state(&s), State::Unchanged));
        assert!(!state(&s).needs_a_person());
    }

    #[test]
    fn an_edit_that_never_landed_is_a_question() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited and then the process died").unwrap();
        assert!(matches!(state(&s), State::Edited));
        assert!(state(&s).needs_a_person());
    }

    #[test]
    fn a_write_back_returns_the_session_to_quiet() {
        // The property that makes comparing against the container work at all:
        // repacking recomputes the stored CRC, so nothing has to maintain the
        // comparison across a session's write-backs.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let mut s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        assert!(matches!(state(&s), State::Edited));

        writeback::write_back(&mut s).unwrap();
        assert!(matches!(state(&s), State::Unchanged));
    }

    #[test]
    fn a_session_that_died_before_extracting_has_nothing_to_ask_about() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = session::create(&root, &c, "report.pdf").unwrap();
        assert!(matches!(state(&s), State::NothingExtracted));
        assert!(!state(&s).needs_a_person());
    }

    #[test]
    fn a_container_that_went_away_leaves_the_payload_worth_offering() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        fs::remove_file(&c).unwrap();

        assert!(matches!(state(&s), State::ContainerGone));
        assert!(state(&s).needs_a_person());
        assert!(s.payload_path().is_file());
    }

    #[test]
    fn a_different_container_at_the_same_path_is_not_written_over() {
        // Writing back here would rename the payload of a container this
        // session was never opened against.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();

        // Something else entirely, at the path the session recorded.
        let other = container(tmp.path(), "plan.dwg", b"unrelated");
        fs::rename(&other, &c).unwrap();

        match state(&s) {
            State::ContainerChanged { recorded, found } => {
                assert_eq!(recorded, "report.pdf");
                assert_eq!(found, "plan.dwg");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_zero_length_payload_compares_rather_than_erroring() {
        // SPEC 2.3 permits one, and CRC-32 of nothing is zero on both sides.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "empty.txt", b"");

        let s = opened(&root, &c, "empty.txt");
        assert!(matches!(state(&s), State::Unchanged));

        fs::write(s.payload_path(), b"no longer empty").unwrap();
        assert!(matches!(state(&s), State::Edited));
    }

    #[test]
    fn the_crc_is_streamed_rather_than_read_whole() {
        // A payload may be any size, and a recovery that needs one in memory
        // fails on the containers most worth recovering. Larger than the
        // buffer, so the loop runs more than once.
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big.bin");
        let bytes = vec![0xa5u8; 300 * 1024];
        fs::write(&big, &bytes).unwrap();
        assert_eq!(crc_of(&big).unwrap(), crc32fast::hash(&bytes));
    }
}
