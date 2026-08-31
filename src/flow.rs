//! Concept 5's steps, joined into a session.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Open and validate, decide, extract, mark, launch, watch, write back, close.
//! Everything security-relevant that this tool does happens on the path through
//! [`open`], which is what concept 8 means by the engine being one body of code
//! on three platforms.
//!
//! **The policy check is here and immediately before the launch.** Concept 10
//! says enforcement lives in the launch path: a value read at startup, held
//! across a policy push, or handed in over IPC is a bypass. So [`open`] resolves
//! policy itself, from sources it is given rather than from an answer somebody
//! else computed, and nothing between that decision and the launch can change
//! what runs.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::platform::Launcher;
use crate::policy::{self, Decision};
use crate::session::{self, Session};
use crate::watch::{Change, Watch};
use crate::{content, extract, recover, writeback};

/// Why a container did not open.
#[derive(Debug)]
pub enum Error {
    /// It is not a container, or not one this build can read.
    Container(slpc::Error),
    /// Policy will not have it opened. Carries the decision, so the refusal can
    /// say which of the several reasons applies.
    Refused(Decision),
    /// Policy could not be established. Distinct from a refusal: nothing has
    /// decided that this payload may not be opened, and the remedy is to fix
    /// the source rather than to change the lists.
    Policy(policy::Error),
    /// The session directory could not be made.
    Session(std::io::Error),
    /// The payload did not reach the session directory.
    Extract(extract::Error),
    /// The desktop would not open it.
    Launch(std::io::Error),
    /// The payload directory could not be watched. Fatal rather than
    /// degraded: concept 6 already concedes that detection is unreliable, and a
    /// session with no watch at all would write back only at close while
    /// looking like one that writes back on every save.
    Watch(notify::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Container(e) => write!(f, "{e}"),
            Self::Refused(d) => match d {
                Decision::Denied { key } => write!(f, "{key} is on the deny list"),
                Decision::NotPermitted { key } => write!(f, "{key} is not in the allowed set"),
                Decision::NoUsableExtension => write!(
                    f,
                    "the payload has no usable extension, so the desktop would ask which \
                     application to run it with"
                ),
                Decision::Open { .. } => write!(f, "permitted"),
            },
            Self::Policy(e) => write!(f, "policy could not be read: {e}"),
            Self::Session(e) => write!(f, "the session could not be started: {e}"),
            Self::Extract(e) => write!(f, "{e}"),
            Self::Launch(e) => write!(f, "the payload could not be opened: {e}"),
            Self::Watch(e) => write!(f, "the payload directory could not be watched: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A session that is open, with its payload launched and its directory watched.
pub struct Opened {
    session: Session,
    watch: Watch,
    /// What the platform recorded about where the container came from, carried
    /// onto the payload.
    pub mark: slpc::provenance::Mark,
    /// What concept 5.1's content check found, where it found anything.
    pub misrepresented: Option<content::Executable>,
    saw_payload_change: bool,
}

/// What closing a session did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    /// Written back where asked, and the session directory removed.
    Cleared,
    /// The target application still has things of its own in the payload
    /// directory, so the session was handed to recovery instead of being
    /// removed. Concept 6.2: the close is honoured, but deleting the directory
    /// underneath a running editor sends its next save nowhere this tool will
    /// ever look.
    LeftForRecovery,
}

/// Open a container: concept 5, steps 1 through 7.
///
/// # Errors
///
/// See [`Error`]. Nothing is left behind on any of them except
/// [`Error::Extract`] carrying [`extract::Error::Unmarked`] with
/// `payload_removed` false, which says so.
pub fn open(
    root: &Path,
    container_path: &Path,
    source: &impl policy::Source,
    launcher: &impl Launcher,
) -> Result<Opened, Error> {
    // Step 1. Opening is validating: `Container::open` applies SPEC 3 and the
    // limits SPEC 6 asks for before it will answer any question about the file.
    let mut container = slpc::Container::open(container_path).map_err(Error::Container)?;

    // Step 2, and step 3's refusals. Resolved here rather than passed in.
    let decision = policy::decide(source, container.payload_name()).map_err(Error::Policy)?;
    if !matches!(decision, Decision::Open { .. }) {
        return Err(Error::Refused(decision));
    }

    // Read before anything is written, so the warning is available whether or
    // not the extraction goes on to succeed. It reports and never refuses:
    // concept 5.1.
    let misrepresented = misrepresentation(&mut container, &decision);

    // Step 4.
    let session =
        session::create(root, container_path, container.payload_name()).map_err(Error::Session)?;

    // Steps 5 and 6. A failure here takes the session directory with it rather
    // than leaving a half-made one for recovery to ask about.
    let mark = match extract::extract(&mut container, &session) {
        Ok(m) => m,
        Err(e) => {
            let payload = session.payload_path();
            let _ = session.clone().remove();
            // `extract` reports whether *it* managed to take the ungated
            // payload back off disk, and then this removes the whole session
            // directory, which usually succeeds where the single unlink did
            // not. Left alone, the message tells somebody there is an ungated
            // executable on disk after the file has gone. Re-asked of the
            // filesystem, after the cleanup, so the sentence is true when it is
            // printed.
            return Err(Error::Extract(match e {
                extract::Error::Unmarked { cause, .. } => extract::Error::Unmarked {
                    cause,
                    payload_removed: !payload.exists(),
                },
                other => other,
            }));
        }
    };

    // Step 8 before step 7: the watch is registered before the application is
    // told the file exists, or a save that arrives quickly enough is a save
    // nothing was listening for.
    let watch = match Watch::on(&session.payload_dir(), &session.record().payload) {
        Ok(w) => w,
        Err(e) => {
            let _ = session.clone().remove();
            return Err(Error::Watch(e));
        }
    };

    if let Err(e) = launcher.launch(&session.payload_path()) {
        let _ = session.clone().remove();
        return Err(Error::Launch(e));
    }

    Ok(Opened {
        session,
        watch,
        mark,
        misrepresented,
        saw_payload_change: false,
    })
}

/// What concept 5.1's check makes of the payload, read out of the container
/// rather than off disk so the answer is available before anything is written.
fn misrepresentation<R: std::io::Read + std::io::Seek>(
    container: &mut slpc::Container<R>,
    decision: &Decision,
) -> Option<content::Executable> {
    let key = match decision {
        Decision::Open { key } => Some(key.as_str()),
        _ => None,
    };
    let mut head = [0u8; content::HEAD];
    let mut payload = container.payload().ok()?;
    let mut at = 0;
    while at < head.len() {
        match std::io::Read::read(&mut payload, &mut head[at..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => at += n,
        }
    }
    content::misrepresents(&head[..at], key)
}

impl Opened {
    /// The session on disk.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Where the payload was put.
    #[must_use]
    pub fn payload_path(&self) -> PathBuf {
        self.session.payload_path()
    }

    /// Whether the payload has been seen to change since the session opened.
    #[must_use]
    pub fn saw_a_change(&self) -> bool {
        self.saw_payload_change
    }

    /// Whether the target application has anything of its own in the payload
    /// directory (concept 6.1).
    ///
    /// # Errors
    ///
    /// Where the payload directory cannot be read.
    pub fn application_is_working(&self) -> std::io::Result<bool> {
        crate::watch::siblings_present(&self.session.payload_dir(), &self.session.record().payload)
    }

    /// Take whatever the watch has to say, and write back once if the payload
    /// was among it.
    ///
    /// Once, rather than once per event. A single save arrives as several
    /// events — a temporary sibling, a rename, a metadata touch — and repacking
    /// per event would rebuild the container three times to the same end.
    ///
    /// # Errors
    ///
    /// Where the write-back failed. The session stays open: concept 6.2 puts
    /// the close at the user's hand, and a failed save is a reason to tell them
    /// rather than to give up on the container.
    pub fn pump(&mut self) -> Result<bool, writeback::Error> {
        self.pump_including(None)
    }

    /// [`pump`](Self::pump), counting a change already taken off the channel.
    ///
    /// **A change that has been received is a change that has happened.**
    /// `wait_and_pump` blocks by taking one change off the channel, so passing
    /// it in here is what stops that one being dropped on the floor. No save is
    /// known to have been lost to the earlier version — every save measured
    /// emits more than one event, and the next drain collects the rest — but it
    /// relied on that being true of every application on three platforms, which
    /// is not a thing this code is in a position to know.
    fn pump_including(&mut self, first: Option<Change>) -> Result<bool, writeback::Error> {
        let mut payload_changed = first == Some(Change::Payload);
        for change in self.watch.drain() {
            if change == Change::Payload {
                payload_changed = true;
            }
        }
        if !payload_changed {
            return Ok(false);
        }
        self.saw_payload_change = true;
        self.save_if_changed()
    }

    /// Write the payload back, unless it already matches what the container
    /// holds.
    ///
    /// **Asked of the bytes rather than of the events.** One save arrives as
    /// several events — a temporary sibling, a rename, a metadata touch — and
    /// they do not reliably land in one drain, so counting events makes the
    /// number of repacks a function of how busy the machine is. A quiet period
    /// before repacking would trade that for latency on every save and still
    /// only make the guess better. `recover` answers the real question by
    /// comparing against the CRC-32 the container already records (concept
    /// 6.3), so a redundant event costs one comparison instead of one rebuild.
    ///
    /// **Only the two quiet states are silent.** An earlier version returned
    /// *nothing to do* for every state that was not `Edited`, which meant a
    /// container deleted or replaced underneath a live session stopped it
    /// saving without saying anything — the user edits, nothing is written, and
    /// no error appears. Those states go to the write-back to be refused and
    /// reported, which is where the refusal belongs anyway.
    ///
    /// # Errors
    ///
    /// Where the write-back failed, or cannot be attempted at all.
    pub fn save_if_changed(&mut self) -> Result<bool, writeback::Error> {
        match recover::state(&self.session) {
            // Nothing to write, and nothing wrong.
            recover::State::Unchanged | recover::State::NothingExtracted => Ok(false),
            // `Edited`, and every state that means this session can no longer
            // reach its container. `write_back` refuses the ones it must and
            // names the reason.
            _ => {
                writeback::write_back(&mut self.session)?;
                Ok(true)
            }
        }
    }

    /// Wait up to `within` for something to happen, then [`pump`](Self::pump).
    ///
    /// # Errors
    ///
    /// As [`pump`](Self::pump).
    pub fn wait_and_pump(&mut self, within: Duration) -> Result<bool, writeback::Error> {
        let first = self.watch.next_change(within);
        self.pump_including(first)
    }

    /// Close the session: catch up on the watch, then clean up.
    ///
    /// Concept 6.2's question — *write it back anyway?* — is the caller's, and
    /// so is the answer: it asks, and calls
    /// [`save_if_changed`](Self::save_if_changed) if the answer is yes. This
    /// used to take a `bool` and repack unconditionally on it, which rebuilt
    /// the container even when the payload matched it byte for byte, and
    /// rebuilt it twice when the final pump had just done so.
    ///
    /// # Errors
    ///
    /// Where the final catch-up write-back failed, in which case nothing is
    /// removed and the session stays recoverable.
    pub fn close(mut self) -> Result<Closed, writeback::Error> {
        // Anything the watch has not been asked about yet. A save arriving
        // between the last pump and the close is a save.
        self.pump()?;

        // Concept 6.2. The close is honoured either way; what changes is
        // whether the directory goes now or is handed to recovery, so that an
        // editor still holding the payload has somewhere for its next save to
        // land and the next launch asks about it.
        if self.application_is_working().unwrap_or(true) {
            return Ok(Closed::LeftForRecovery);
        }
        // A failure to remove leaves a session recovery will pick up, which is
        // the same outcome by another road and not worth a second error type.
        let _ = self.session.remove();
        Ok(Closed::Cleared)
    }
}

#[cfg(test)]
mod tests {
    use super::{open, Closed, Error};
    use crate::platform::testing::Recording;
    use crate::policy::{Layer, Origin, Source};
    use crate::writeback;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// Says nothing at every layer, so the shipped set answers.
    struct Default_;
    impl Source for Default_ {
        fn layer(&self, _o: Origin) -> crate::policy::Read {
            Ok(None)
        }
    }

    /// Denies everything, for the refusal arms.
    struct DenyAll;
    impl Source for DenyAll {
        fn layer(&self, o: Origin) -> crate::policy::Read {
            Ok((o == Origin::MachinePolicy).then(|| Layer {
                allowed: Some(Vec::new()),
                ..Layer::default()
            }))
        }
    }

    fn container(at: &Path, name: &str, payload: &[u8]) -> PathBuf {
        let doc: slpc::toml_edit::DocumentMut =
            format!("slipcase_version = \"1.0\"\n\n[payload]\nfile = \"{name}\"\n")
                .parse()
                .unwrap();
        let path = at.join(format!("{name}.slpc"));
        slpc::pack_reader(name, payload, doc, fs::File::create(&path).unwrap()).unwrap();
        path
    }

    #[test]
    fn opening_extracts_launches_and_watches() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"%PDF first");
        let launcher = Recording::default();

        let o = open(&root, &c, &Default_, &launcher).unwrap();
        assert_eq!(launcher.launched(), [o.payload_path()]);
        assert_eq!(fs::read(o.payload_path()).unwrap(), b"%PDF first");
        assert!(!o.saw_a_change());
    }

    #[test]
    fn a_save_reaches_the_container_without_anybody_closing_the_session() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        let mut o = open(&root, &c, &Default_, &launcher).unwrap();

        // The way a serious editor saves: a temporary sibling renamed over the
        // target, which is the case a watch on the file would miss.
        let scratch = o.payload_path().with_extension("pdf.tmp");
        fs::write(&scratch, b"edited").unwrap();
        fs::rename(&scratch, o.payload_path()).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !o.saw_a_change() {
            o.wait_and_pump(Duration::from_millis(250)).unwrap();
        }
        assert!(o.saw_a_change(), "the save never reached the session");

        let mut back = slpc::Container::open(&c).unwrap();
        let mut got = Vec::new();
        std::io::copy(&mut back.payload().unwrap(), &mut got).unwrap();
        assert_eq!(got, b"edited");
    }

    #[test]
    fn a_save_that_emits_one_event_still_reaches_the_container() {
        // Concept 6 is written about editors that save atomically, and the
        // tests followed it there. This is the other shape: a plain write in
        // place, which emits fewer events. It passes either side of the
        // `pump_including` change rather than pinning it — what it pins is that
        // the simple save works at all, which nothing else asserted.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let mut o = open(&root, &c, &Default_, &Recording::default()).unwrap();

        fs::write(o.payload_path(), b"edited in place").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !o.saw_a_change() {
            o.wait_and_pump(Duration::from_millis(250)).unwrap();
        }
        assert!(o.saw_a_change(), "a single-event save was never noticed");

        let mut back = slpc::Container::open(&c).unwrap();
        let mut got = Vec::new();
        std::io::copy(&mut back.payload().unwrap(), &mut got).unwrap();
        assert_eq!(got, b"edited in place");
    }

    #[test]
    fn one_save_is_one_write_back() {
        // A repack costs a full rebuild of the container, so the number of
        // them a session performs should follow the edits and not the event
        // traffic. Counting events cannot give that: one save arrives as
        // several, they do not reliably land in one drain, and this test was
        // flaky under a loaded suite for exactly that reason before `pump`
        // compared the bytes instead.
        //
        // Counted rather than inspected, because every redundant repack writes
        // the same bytes — a test asserting the container's contents passes
        // whatever the count is.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        let mut o = open(&root, &c, &Default_, &launcher).unwrap();

        let scratch = o.payload_path().with_extension("pdf.tmp");
        fs::write(&scratch, b"edited").unwrap();
        fs::rename(&scratch, o.payload_path()).unwrap();

        // Pump well past the point where the save has landed, so every event
        // it produced has arrived and been acted on.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            o.wait_and_pump(Duration::from_millis(100)).unwrap();
        }

        assert!(o.saw_a_change(), "the save never reached the session");
        assert_eq!(
            o.session().record().write_backs,
            1,
            "one save produced more than one write-back"
        );
    }

    #[test]
    fn policy_refuses_before_a_session_directory_exists() {
        // Concept 10 puts enforcement in the launch path, and a refusal that
        // had already written the payload somewhere would be a refusal in name.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        assert!(matches!(
            open(&root, &c, &DenyAll, &launcher),
            Err(Error::Refused(_))
        ));
        assert!(launcher.launched().is_empty());
        assert!(crate::session::scan(&root).unwrap().is_empty());
    }

    #[test]
    fn a_payload_with_no_usable_extension_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "README", b"hello");
        let launcher = Recording::default();

        match open(&root, &c, &Default_, &launcher) {
            Err(e) => assert!(e.to_string().contains("no usable extension"), "{e}"),
            Ok(_) => panic!("a payload with no usable extension was opened"),
        }
        assert!(crate::session::scan(&root).unwrap().is_empty());
    }

    #[test]
    fn an_executable_wearing_a_documents_name_is_reported_and_still_opened() {
        // Concept 5.1. The extension governs what runs, so a PDF reader handed
        // a PE image fails on it harmlessly; refusing would assert a control
        // this path does not carry. The person is told and decides.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "invoice.pdf", b"MZ\x90\x00 not a pdf");
        let launcher = Recording::default();

        let o = open(&root, &c, &Default_, &launcher).unwrap();
        assert_eq!(o.misrepresented, Some(crate::content::Executable::Pe));
        assert_eq!(launcher.launched().len(), 1);
    }

    #[test]
    fn a_desktop_that_will_not_open_it_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        assert!(matches!(
            open(&root, &c, &Default_, &Recording::refusing()),
            Err(Error::Launch(_))
        ));
        assert!(crate::session::scan(&root).unwrap().is_empty());
    }

    #[test]
    fn closing_without_a_change_can_still_write_back() {
        // The only available answer to Save As: no event fires when somebody
        // saves elsewhere, so a session that saw nothing may still have an edit
        // that belongs in the container.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        let mut o = open(&root, &c, &Default_, &launcher).unwrap();
        fs::write(o.payload_path(), b"edited quietly").unwrap();
        // Deliberately not pumped: this is the path where nothing was seen.
        assert!(o.save_if_changed().unwrap());
        assert_eq!(o.close().unwrap(), Closed::Cleared);

        let mut back = slpc::Container::open(&c).unwrap();
        let mut got = Vec::new();
        std::io::copy(&mut back.payload().unwrap(), &mut got).unwrap();
        assert_eq!(got, b"edited quietly");
        assert!(crate::session::scan(&root).unwrap().is_empty());
    }

    #[test]
    fn closing_while_the_application_is_working_hands_over_to_recovery() {
        // Concept 6.2: the close is honoured, but removing the directory under
        // a running editor sends its next save nowhere this tool will look.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        // No edit here, deliberately. `close` pumps before it decides, so a
        // save made in this test may or may not have reached the container by
        // the time the state is read — asserting on that state made this fail
        // about one run in six. What the handover rule guarantees is that the
        // directory survives, and that is what is checked.
        let o = open(&root, &c, &Default_, &launcher).unwrap();
        let payload = o.payload_path();
        fs::write(payload.with_file_name("~$report.pdf"), b"").unwrap();

        assert_eq!(o.close().unwrap(), Closed::LeftForRecovery);

        let left = crate::session::scan(&root).unwrap();
        assert_eq!(left.len(), 1);
        // Still there for the editor's next save to land in, which is the whole
        // point of not deleting it.
        assert!(payload.is_file());
    }

    #[test]
    fn a_container_deleted_under_a_live_session_is_reported_rather_than_ignored() {
        // Found in review, and it was a regression: once `pump` compared bytes,
        // every state that was not `Edited` returned *nothing to do*, so a
        // container removed underneath a session stopped it saving and said
        // nothing at all. The person keeps editing and no error ever appears.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let mut o = open(&root, &c, &Default_, &Recording::default()).unwrap();

        fs::write(o.payload_path(), b"edited").unwrap();
        fs::remove_file(&c).unwrap();

        assert!(matches!(
            o.save_if_changed(),
            Err(writeback::Error::Container(_))
        ));
    }

    #[test]
    fn a_different_container_at_the_recorded_path_refuses_the_write_back() {
        // The guard belongs on the acting side and not only in `recover`:
        // repacking here would rename the payload of a container this session
        // was never opened against.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let mut o = open(&root, &c, &Default_, &Recording::default()).unwrap();

        fs::write(o.payload_path(), b"edited").unwrap();
        let other = container(tmp.path(), "plan.dwg", b"unrelated");
        fs::rename(&other, &c).unwrap();

        match o.save_if_changed() {
            Err(writeback::Error::ContainerChanged { recorded, found }) => {
                assert_eq!(recorded, "report.pdf");
                assert_eq!(found, "plan.dwg");
            }
            other => panic!("{other:?}"),
        }
        // Untouched: still the other container, still its own payload name.
        assert_eq!(
            slpc::Container::open(&c).unwrap().payload_name(),
            "plan.dwg"
        );
    }

    #[test]
    fn saying_yes_to_an_unchanged_payload_rebuilds_nothing() {
        // `close` used to take the answer as a `bool` and repack on it without
        // asking whether anything had changed. That signature is gone, so this
        // cannot be made to fail by reverting the fix the way the two above
        // can; it pins the behaviour rather than the defect. What it is worth
        // is that rewriting the only copy of a container is not a free
        // operation, and answering *yes* to a question about a payload nobody
        // edited should cost nothing.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let mut o = open(&root, &c, &Default_, &Recording::default()).unwrap();

        assert!(!o.save_if_changed().unwrap());
        assert_eq!(o.session().record().write_backs, 0);
    }

    #[test]
    fn an_edit_is_written_back_once_however_many_times_it_is_asked_for() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let mut o = open(&root, &c, &Default_, &Recording::default()).unwrap();

        fs::write(o.payload_path(), b"edited").unwrap();
        assert!(o.save_if_changed().unwrap());
        assert!(!o.save_if_changed().unwrap());
        assert!(!o.save_if_changed().unwrap());
        assert_eq!(o.session().record().write_backs, 1);
    }

    #[test]
    fn a_clean_close_leaves_nothing_for_recovery_to_ask_about() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        let o = open(&root, &c, &Default_, &launcher).unwrap();
        assert_eq!(o.close().unwrap(), Closed::Cleared);
        assert!(crate::session::scan(&root).unwrap().is_empty());
    }
}
