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
use crate::{content, extract, writeback};

/// Why a container did not open.
#[derive(Debug)]
pub enum Error {
    /// It is not a container, or not one this build can read.
    Container(slpc::Error),
    /// Policy will not have it opened. Carries the decision, so the refusal can
    /// say which of the several reasons applies.
    Refused(Decision),
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
    let decision = policy::decide(source, container.payload_name());
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
            let _ = session.clone().remove();
            return Err(Error::Extract(e));
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
        let mut payload_changed = false;
        for change in self.watch.drain() {
            if change == Change::Payload {
                payload_changed = true;
            }
        }
        if !payload_changed {
            return Ok(false);
        }
        self.saw_payload_change = true;
        writeback::write_back(&mut self.session)?;
        Ok(true)
    }

    /// Wait up to `within` for something to happen, then [`pump`](Self::pump).
    ///
    /// # Errors
    ///
    /// As [`pump`](Self::pump).
    pub fn wait_and_pump(&mut self, within: Duration) -> Result<bool, writeback::Error> {
        let _ = self.watch.next_change(within);
        self.pump()
    }

    /// Close the session: a final write-back where asked, then clean up.
    ///
    /// `write_back` is the caller's answer to concept 6.2's question. Where the
    /// session saw no change, asking is the only available answer to Save As —
    /// no event fires when somebody saves to a different location, so a session
    /// that saw nothing may still have an edit that belongs in the container.
    /// Asking is not detection and must not be reported as though it were.
    ///
    /// # Errors
    ///
    /// Where the final write-back failed, in which case nothing is removed and
    /// the session stays recoverable.
    pub fn close(mut self, write_back: bool) -> Result<Closed, writeback::Error> {
        // Anything the watch has not been asked about yet, first. A save
        // arriving between the last pump and the close is a save.
        self.pump()?;

        if write_back {
            writeback::write_back(&mut self.session)?;
        }

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
    use crate::recover;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// Says nothing at every layer, so the shipped set answers.
    struct Default_;
    impl Source for Default_ {
        fn layer(&self, _o: Origin) -> Option<Layer> {
            None
        }
    }

    /// Denies everything, for the refusal arms.
    struct DenyAll;
    impl Source for DenyAll {
        fn layer(&self, o: Origin) -> Option<Layer> {
            (o == Origin::MachinePolicy).then(|| Layer {
                allowed: Some(Vec::new()),
                ..Layer::default()
            })
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

        let o = open(&root, &c, &Default_, &launcher).unwrap();
        fs::write(o.payload_path(), b"edited quietly").unwrap();
        // Deliberately not pumped: this is the path where nothing was seen.
        assert_eq!(o.close(true).unwrap(), Closed::Cleared);

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

        let o = open(&root, &c, &Default_, &launcher).unwrap();
        fs::write(o.payload_path().with_file_name("~$report.pdf"), b"").unwrap();
        fs::write(o.payload_path(), b"edited").unwrap();

        assert_eq!(o.close(false).unwrap(), Closed::LeftForRecovery);

        let left = crate::session::scan(&root).unwrap();
        assert_eq!(left.len(), 1);
        assert!(matches!(recover::state(&left[0]), recover::State::Edited));
    }

    #[test]
    fn a_clean_close_leaves_nothing_for_recovery_to_ask_about() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();

        let o = open(&root, &c, &Default_, &launcher).unwrap();
        assert_eq!(o.close(false).unwrap(), Closed::Cleared);
        assert!(crate::session::scan(&root).unwrap().is_empty());
    }
}
