//! What to do with a session that outlived the process holding it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 6.3, amended 2026-09-03. **Recovery writes back an edit whose
//! container has not moved, and asks about everything else.**
//!
//! The rule it replaces was that recovery never writes back on its own, because
//! the tool was not watching when the process died and so cannot tell a
//! complete save from a half-written one. That risk is real and is not gone.
//! What changed is the comparison it was being weighed against: the alternative
//! is not safety, it is a question, and a question that goes unanswered loses
//! the same edit more quietly. The person pressed Save; being asked afterwards
//! to choose between *write back*, *discard* and *reveal the folder* is this
//! tool's own failure handed back to them in vocabulary they never asked to
//! learn.
//!
//! Two things keep the risk small. Most applications save by writing a sibling
//! and renaming over the payload — the behaviour concept 6.1 already relies on
//! to know the application is working — so the file is the old bytes or the
//! complete new ones and not a prefix of either. And what is written back is
//! reported rather than done silently, so an outcome that looks wrong is
//! visible while the container is still open in front of somebody.
//!
//! **It only acts where it knows which side moved**, which is
//! [`State::Edited`] and nothing else. Where the container has changed too,
//! nobody but the person can say which copy is the one they want, and that is
//! the question worth interrupting for.
//!
//! The ZIP central directory already stores a CRC-32 for the payload member, so
//! recovery computes the CRC of the extracted payload and compares. Equal means
//! nothing was lost. Different means an edit never landed.
//!
//! **Comparing against the container beats recording a digest of the payload.**
//! A recorded value is a second copy of a fact and can drift from it, and the
//! moment it gets consulted is after a crash, which is when a session record is
//! least trustworthy. The container's own value needs nothing maintaining it:
//! repacking recomputes it, so the comparison stays correct across every
//! write-back in a session as a side effect of the write-backs themselves.
//!
//! **The one value the session does record is not that**, and the difference is
//! the whole reason it is allowed. *Has the payload changed* is answerable from
//! the container and is answered there. *Which side changed* is answerable from
//! neither side, because both are only observable now and the question is about
//! then — see [`crate::session::Record::agreed`], which notes what the container
//! held at the two moments the two were made to agree and nothing else.
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
    /// The payload differs from the one in the container, and the container is
    /// still holding what this session last agreed with it about. So the
    /// difference is this session's own edit and nobody else's, and it goes
    /// back.
    Edited,
    /// The payload differs from the container *and* the container is not what
    /// it was when the two last agreed. Both sides moved.
    ///
    /// **The one case worth interrupting for.** Writing back would throw away
    /// whatever changed the container, and discarding would throw away the
    /// edit; there is no answer here that is not somebody's decision. It is
    /// also rare: it needs a second writer to the same container while this
    /// session was not running.
    Diverged,
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
            Self::Diverged => write!(
                f,
                "edited, and the container changed too, so both hold work the other does not"
            ),
            Self::ContainerGone => write!(f, "the container is no longer where it was"),
            Self::ContainerChanged { recorded, found } => {
                write!(f, "the container now holds {found} rather than {recorded}")
            }
            Self::Unreadable(e) => write!(f, "cannot be read: {e}"),
        }
    }
}

/// What recovery does about a session left behind, without being told.
///
/// Exhaustive over [`State`] on purpose. Two predicates would let a state added
/// later fall into whichever bucket the negation happened to put it in, and the
/// buckets here are *delete it*, *write to somebody's container* and *interrupt
/// them* — three things that must never be picked by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Course {
    /// Nothing was lost. Remove it and say nothing (concept 6.3).
    Sweep,
    /// An edit that never reached a container that is still where it was. Put
    /// it back, and say so.
    WriteBack,
    /// Only a person can settle it.
    Ask,
}

impl State {
    /// What to do about it.
    #[must_use]
    pub fn course(&self) -> Course {
        match self {
            Self::NothingExtracted | Self::Unchanged => Course::Sweep,
            Self::Edited => Course::WriteBack,
            Self::Diverged
            | Self::ContainerGone
            | Self::ContainerChanged { .. }
            | Self::Unreadable(_) => Course::Ask,
        }
    }

    /// Whether a person has to be asked about this one.
    #[must_use]
    pub fn needs_a_person(&self) -> bool {
        self.course() == Course::Ask
    }

    /// Whether this one is recovery's own to put back.
    #[must_use]
    pub fn is_ours_to_write_back(&self) -> bool {
        self.course() == Course::WriteBack
    }

    /// Whether it can be removed with nothing said.
    ///
    /// **Not `!needs_a_person()`**, which is what it used to be and what would
    /// now sweep away an edit. See [`Course`].
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.course() == Course::Sweep
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
        return State::Unchanged;
    }

    // The payload and the container disagree. Which of them moved is the whole
    // question, and only the record answers it: `agreed` is what the container
    // held the last time this session and it were made to agree.
    //
    // Not known — an older build, or a container unreadable at the time — is
    // read as *not known to agree* and asks. That is the cautious direction and
    // the one that cannot lose anything.
    match session.record().agreed {
        Some(agreed) if agreed == stored => State::Edited,
        _ => State::Diverged,
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
    use super::{crc_of, state, Course, State};
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
        let mut s = session::create(root, c, name).unwrap();
        extract::extract(&mut slpc::Container::open(c).unwrap(), &mut s).unwrap();
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
    fn an_edit_that_never_landed_goes_back_without_being_asked_about() {
        // The container is still holding what this session agreed with it
        // about, so the difference is this session's own edit and nobody
        // else's. There is nothing for a person to decide.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited and then the process died").unwrap();
        assert!(matches!(state(&s), State::Edited));
        assert_eq!(state(&s).course(), Course::WriteBack);
        assert!(!state(&s).needs_a_person());
        assert!(!state(&s).is_quiet(), "it must not be swept away");
    }

    #[test]
    fn an_edit_whose_container_also_moved_is_a_question() {
        // Both sides changed, so writing back throws away whatever changed the
        // container and discarding throws away the edit. Nobody but the person
        // can pick.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"our edit").unwrap();
        // Somebody else repacked it while this session was not running.
        container(tmp.path(), "report.pdf", b"somebody else's second thoughts");

        assert!(matches!(state(&s), State::Diverged));
        assert_eq!(state(&s).course(), Course::Ask);
    }

    #[test]
    fn a_session_that_never_recorded_an_agreement_is_asked_about() {
        // What an older build's session looks like, and the cautious reading of
        // it: not known to agree is not the same as agreeing, so it asks rather
        // than writing into a container it cannot vouch for.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        let s = opened(&root, &c, "report.pdf");
        fs::write(s.payload_path(), b"edited").unwrap();
        assert_eq!(state(&s).course(), Course::WriteBack);

        // Strike the line an older build would never have written.
        let record = s.dir().join("session.toml");
        let text = fs::read_to_string(&record).unwrap();
        let without: Vec<&str> = text.lines().filter(|l| !l.starts_with("agreed")).collect();
        fs::write(&record, without.join("\n")).unwrap();
        let reread = &session::scan(&root).unwrap()[0];

        assert!(reread.record().agreed.is_none());
        assert_eq!(state(reread).course(), Course::Ask);
    }

    #[test]
    fn every_state_takes_exactly_one_course() {
        // `Course` is exhaustive over `State` on purpose: the three outcomes
        // are delete it, write to somebody's container, and interrupt them, and
        // a state added later must not fall into one of those by whichever way
        // a negation happened to go.
        for (state, want) in [
            (State::NothingExtracted, Course::Sweep),
            (State::Unchanged, Course::Sweep),
            (State::Edited, Course::WriteBack),
            (State::Diverged, Course::Ask),
            (State::ContainerGone, Course::Ask),
            (
                State::ContainerChanged {
                    recorded: "a".into(),
                    found: "b".into(),
                },
                Course::Ask,
            ),
            (State::Unreadable("why".into()), Course::Ask),
        ] {
            assert_eq!(state.course(), want, "{state:?}");
            assert_eq!(state.is_quiet(), want == Course::Sweep, "{state:?}");
            assert_eq!(state.needs_a_person(), want == Course::Ask, "{state:?}");
            assert_eq!(
                state.is_ours_to_write_back(),
                want == Course::WriteBack,
                "{state:?}"
            );
        }
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
