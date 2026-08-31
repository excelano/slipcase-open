//! The instance that holds the sessions, and the loop that serves it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8. One process holds every open session; every other invocation
//! reaches it through the front door and exits.
//!
//! **Requests are served on the loop's own thread and the accepting is not.** A
//! blocking `accept` would starve the watchers for as long as nobody connected,
//! and the watchers are the whole reason this process exists. So a thread does
//! nothing but accept and hand connections over, and the loop alternates
//! between answering one and pumping the sessions.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::flow::{self, Opened};
use crate::ipc::{Request, Response};
use crate::platform::Launcher;
use crate::policy;
use crate::recover;
use crate::session::{self, Session};
use crate::table::Table;

/// How long the loop waits for a request before going round to pump.
const TICK: Duration = Duration::from_millis(250);

/// The sessions this instance is holding.
pub struct Resident {
    root: PathBuf,
    sessions: Table<Opened>,
}

impl Resident {
    /// An instance holding nothing, keeping its sessions under `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            sessions: Table::new(),
        }
    }

    /// Whether there is anything left to hold, which is concept 8's exit rule.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Answer one request.
    pub fn handle(
        &mut self,
        request: Request,
        source: &impl policy::Source,
        launcher: &impl Launcher,
    ) -> Response {
        match request {
            Request::Ping => Response::Ok(Vec::new()),
            Request::List => self.list(),
            Request::Open(path) => self.open(&path, source, launcher),
            Request::Close(id) => self.close(&id),
        }
    }

    /// Open a container, or bring forward the session that already has it.
    fn open(
        &mut self,
        container: &Path,
        source: &impl policy::Source,
        launcher: &impl Launcher,
    ) -> Response {
        // Concept 8: a container that already has a live session is not opened
        // twice. Two sessions would both repack it and the second write-back
        // would overwrite the first with nothing said. Re-launching is what a
        // second double-click on an open document does everywhere else.
        if let Some(open) = self.sessions.find_mut(container) {
            return match launcher.launch(&open.payload_path()) {
                Ok(()) => Response::Ok(vec![format!(
                    "{} is already open; brought forward.",
                    slpc::display_name(&open.session().record().payload)
                )]),
                Err(e) => Response::Err(format!("could not bring it forward: {e}")),
            };
        }

        // Concept 8: a pending recovery item is resolved first. A session left
        // by a crash is not in the live table, so nothing refuses it — but
        // opening a fresh one would extract the container's current payload and
        // leave the recovered edit with nowhere to go.
        match self.pending_recovery(container) {
            Err(e) => return Response::Err(e),
            Ok(Some(response)) => return response,
            Ok(None) => {}
        }

        match flow::open(&self.root, container, source, launcher) {
            Err(e) => Response::Err(e.to_string()),
            Ok(opened) => {
                let mut lines = vec![format!(
                    "{} is open.",
                    slpc::display_name(&opened.session().record().payload)
                )];
                if opened.mark != slpc::provenance::Mark::Silent {
                    lines.push("  It came from somewhere else, and the copy says so.".into());
                }
                if let Some(what) = opened.misrepresented {
                    lines.push(format!("  Warning: the payload is {}.", what.describes()));
                }
                lines.push(format!("  Session {}", id_of(opened.session())));
                if let Err(e) = self.sessions.insert(container, opened) {
                    return Response::Err(format!("the session could not be tracked: {e}"));
                }
                Response::Ok(lines)
            }
        }
    }

    /// A session left behind on this same container, where there is one worth
    /// asking about.
    ///
    /// Answered with a refusal rather than a prompt, because Phase 2 has no
    /// channel to ask through: §9's notifications carry the question and arrive
    /// in Phase 3. Naming the commands is the honest form of *the recovery
    /// question comes first* until then.
    fn pending_recovery(&self, container: &Path) -> Result<Option<Response>, String> {
        let want = crate::identity::of(container).map_err(|e| e.to_string())?;
        let sessions = session::scan(&self.root).map_err(|e| e.to_string())?;
        for left in sessions {
            // A container the record names that has since gone cannot be the
            // one being opened, so it is not this invocation's business.
            if !crate::identity::of(&left.record().container).is_ok_and(|is| is == want) {
                continue;
            }
            let state = recover::state(&left);
            if state.needs_a_person() {
                let id = id_of(&left);
                return Ok(Some(Response::Err(format!(
                    "a session on this container was left behind and is {state}.\n\
                     Resolve it first:\n  \
                     slipcase-open recover {id} --write-back\n  \
                     slipcase-open recover {id} --discard"
                ))));
            }
        }
        Ok(None)
    }

    fn list(&self) -> Response {
        let mut lines: Vec<String> = self
            .sessions
            .iter()
            .map(|o| {
                format!(
                    "{}  {}  open, {} write-back(s)",
                    id_of(o.session()),
                    slpc::display_name(&o.session().record().payload),
                    o.session().record().write_backs
                )
            })
            .collect();

        let live: Vec<PathBuf> = self
            .sessions
            .iter()
            .map(|o| o.session().dir().to_path_buf())
            .collect();
        if let Ok(left) = session::scan(&self.root) {
            for s in left
                .iter()
                .filter(|s| !live.contains(&s.dir().to_path_buf()))
            {
                lines.push(format!(
                    "{}  {}  {}",
                    id_of(s),
                    slpc::display_name(&s.record().payload),
                    recover::state(s)
                ));
            }
        }
        if lines.is_empty() {
            lines.push("No sessions.".into());
        }
        Response::Ok(lines)
    }

    fn close(&mut self, id: &str) -> Response {
        let Some(container) = self
            .sessions
            .iter()
            .find(|o| id_of(o.session()) == id)
            .map(|o| o.session().record().container.clone())
        else {
            return Response::Err(format!("no open session {id}"));
        };
        let Some(opened) = self.sessions.remove(&container) else {
            return Response::Err(format!("no open session {id}"));
        };
        match opened.close() {
            Ok(flow::Closed::Cleared) => Response::Ok(vec!["Session closed.".into()]),
            Ok(flow::Closed::LeftForRecovery) => Response::Ok(vec![
                "Session closed, and the application still has the payload open.".into(),
                "It has been left for recovery: run `slipcase-open sessions`.".into(),
            ]),
            Err(e) => Response::Err(e.to_string()),
        }
    }

    /// Give every session's watch a turn, and report what came of it.
    ///
    /// # Errors
    ///
    /// Never as a whole: a session that could not write back is reported and
    /// the others still run, because one failing container is not a reason to
    /// stop watching the rest.
    pub fn pump_all(&mut self) -> Vec<String> {
        let mut notices = Vec::new();
        let mut wrote_back = Vec::new();
        for open in self.sessions.iter_mut() {
            match open.pump() {
                Ok(true) => {
                    let s = open.session();
                    notices.push(format!(
                        "{}: written back ({}).",
                        slpc::display_name(&s.record().payload),
                        s.record().write_backs
                    ));
                    wrote_back.push(s.record().container.clone());
                }
                Ok(false) => {}
                Err(e) => notices.push(format!("could not write back: {e}")),
            }
        }
        // A write-back renamed a new file over the container, so the identity
        // recorded when the session opened is stale. See `table::refresh`.
        for container in wrote_back {
            self.sessions.refresh(&container);
        }
        notices
    }

    /// Close every session, for a shutdown that is not a crash.
    pub fn close_all(&mut self) -> Vec<String> {
        self.sessions
            .drain()
            .filter_map(|open| match open.close() {
                Ok(_) => None,
                Err(e) => Some(format!("could not close a session: {e}")),
            })
            .collect()
    }
}

/// The name `list` prints and `close` takes back.
fn id_of(s: &Session) -> String {
    s.dir()
        .file_name()
        .map_or_else(|| "?".to_string(), |n| n.to_string_lossy().into_owned())
}

/// Remove the sessions left behind that have nothing to say.
///
/// Concept 6.3: a recovered payload matching its container means nothing was
/// lost, so clean up and say nothing.
///
/// **This could not be done before Phase 2 and that is why it was not.** A
/// session that is open and not yet edited reads as unchanged, and no process
/// could tell a live session from a dead one — a sweep run from a second
/// terminal would have deleted a directory out from under a running editor.
/// `live` is what the resident instance knows and nothing else did.
///
/// # Errors
///
/// Where the session root cannot be read. A session that will not go is left
/// rather than reported: it is debris, the next sweep will try again, and
/// failing a launch over it would be the tail wagging the dog.
pub fn sweep(root: &Path, live: &[PathBuf]) -> io::Result<usize> {
    let mut removed = 0;
    for s in session::scan(root)? {
        if live.iter().any(|d| d == s.dir()) {
            continue;
        }
        if recover::state(&s).needs_a_person() {
            continue;
        }
        if s.remove().is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Hold the sessions and serve the front door until nothing is left.
///
/// # Errors
///
/// Where the endpoint cannot be served.
#[cfg(unix)]
pub fn run(
    listener: crate::endpoint::Listener,
    resident: &mut Resident,
    source: &impl policy::Source,
    launcher: &impl Launcher,
    notice: &mut impl FnMut(&str),
) -> io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // A caller that went away between connecting and being read is not an
        // event: `flatten` drops it and takes the next.
        for stream in listener.incoming().flatten() {
            if tx.send(stream).is_err() {
                return;
            }
        }
    });

    loop {
        match rx.recv_timeout(TICK) {
            Ok(mut stream) => {
                let response = match crate::ipc::take(&mut stream) {
                    Ok(request) => resident.handle(request, source, launcher),
                    // A request this build cannot read is answered rather than
                    // dropped, so a client waiting on the front door is not
                    // left waiting on it.
                    Err(e) => Response::Err(e.to_string()),
                };
                let _ = crate::ipc::answer(&mut stream, &response);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            // The accepting thread has gone, which means the listener has.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        for line in resident.pump_all() {
            notice(&line);
        }

        // Concept 8's exit rule. Staying resident does nothing for the crash
        // case, where this process is dead by definition.
        if resident.is_idle() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{sweep, Resident};
    use crate::ipc::{Request, Response};
    use crate::platform::testing::Recording;
    use crate::policy::{Origin, Read, Source};
    use crate::{extract, recover, session};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct Default_;
    impl Source for Default_ {
        fn layer(&self, _o: Origin) -> Read {
            Ok(None)
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

    fn ok(r: Response) -> Vec<String> {
        match r {
            Response::Ok(lines) => lines,
            Response::Err(e) => panic!("{e}"),
        }
    }

    fn err(r: Response) -> String {
        match r {
            Response::Err(e) => e,
            Response::Ok(lines) => panic!("expected a refusal, got {lines:?}"),
        }
    }

    #[test]
    fn opening_a_container_twice_brings_the_session_forward() {
        // Concept 8. Two sessions would both repack it and the second
        // write-back would overwrite the first with nothing said.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();
        let mut r = Resident::new(&root);

        ok(r.handle(Request::Open(c.clone()), &Default_, &launcher));
        let again = ok(r.handle(Request::Open(c.clone()), &Default_, &launcher));

        assert!(again[0].contains("already open"), "{again:?}");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
        // Brought forward means launched again, which is what a second
        // double-click does everywhere else.
        assert_eq!(launcher.launched().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn the_same_container_under_another_hard_link_is_the_same_session() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let link = tmp.path().join("other-name.slpc");
        fs::hard_link(&c, &link).unwrap();
        let launcher = Recording::default();
        let mut r = Resident::new(&root);

        ok(r.handle(Request::Open(c), &Default_, &launcher));
        let again = ok(r.handle(Request::Open(link), &Default_, &launcher));
        assert!(again[0].contains("already open"), "{again:?}");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn two_different_containers_get_two_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let a = container(tmp.path(), "report.pdf", b"a");
        let b = container(tmp.path(), "notes.txt", b"b");
        let launcher = Recording::default();
        let mut r = Resident::new(&root);

        ok(r.handle(Request::Open(a), &Default_, &launcher));
        ok(r.handle(Request::Open(b), &Default_, &launcher));
        assert_eq!(session::scan(&root).unwrap().len(), 2);
        assert!(!r.is_idle());
    }

    #[test]
    fn a_session_survives_a_write_back_still_being_the_same_container() {
        // A session is still the same session after it has saved, even though
        // the write-back renamed a new file over the container and so gave it a
        // new inode. This passes on the path arm alone — checked by reverting
        // `refresh` and watching it stay green — so what it pins is that a save
        // does not lose a session, not that `refresh` works. The identity arm
        // is covered where it can be seen: `table::refreshing_keeps_the_
        // identity_arm_working_after_a_save`, which reaches the container
        // through a hard link made after the save and does fail without it.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();
        let mut r = Resident::new(&root);

        ok(r.handle(Request::Open(c.clone()), &Default_, &launcher));
        let payload = session::scan(&root).unwrap()[0].payload_path();
        fs::write(&payload, b"edited").unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline
            && !r.pump_all().iter().any(|n| n.contains("written back"))
        {}

        let again = ok(r.handle(Request::Open(c), &Default_, &launcher));
        assert!(again[0].contains("already open"), "{again:?}");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn a_recovery_item_on_the_same_container_is_resolved_first() {
        // Concept 8. Opening a fresh session would extract the container's
        // current payload and leave the recovered edit with nowhere to go.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");

        // What a crash leaves: a session on disk, with an edit that never
        // landed and no process holding it.
        let left = session::create(&root, &c, "report.pdf").unwrap();
        extract::extract(&mut slpc::Container::open(&c).unwrap(), &left).unwrap();
        fs::write(left.payload_path(), b"edited then the process died").unwrap();

        let launcher = Recording::default();
        let mut r = Resident::new(&root);
        let refused = err(r.handle(Request::Open(c), &Default_, &launcher));

        assert!(refused.contains("left behind"), "{refused}");
        assert!(refused.contains("--write-back"), "{refused}");
        assert!(launcher.launched().is_empty(), "nothing should have opened");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn a_quiet_leftover_does_not_stand_in_the_way() {
        // Only a recovery item worth asking about blocks. One that matches its
        // container has nothing to lose and should not stop somebody working.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let left = session::create(&root, &c, "report.pdf").unwrap();
        extract::extract(&mut slpc::Container::open(&c).unwrap(), &left).unwrap();
        assert!(matches!(recover::state(&left), recover::State::Unchanged));

        let launcher = Recording::default();
        let mut r = Resident::new(&root);
        ok(r.handle(Request::Open(c), &Default_, &launcher));
        assert_eq!(launcher.launched().len(), 1);
    }

    #[test]
    fn closing_by_name_closes_that_session() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();
        let mut r = Resident::new(&root);

        let opened = ok(r.handle(Request::Open(c), &Default_, &launcher));
        let id = opened
            .iter()
            .find_map(|l| l.strip_prefix("  Session "))
            .unwrap()
            .to_string();

        ok(r.handle(Request::Close(id), &Default_, &launcher));
        assert!(r.is_idle());
        assert!(session::scan(&root).unwrap().is_empty());
    }

    #[test]
    fn closing_a_session_that_is_not_open_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = Resident::new(tmp.path().join("sessions"));
        let refused = err(r.handle(
            Request::Close("nothing-0".into()),
            &Default_,
            &Recording::default(),
        ));
        assert!(refused.contains("no open session"), "{refused}");
    }

    #[test]
    fn listing_shows_what_is_open_and_what_was_left() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let open_one = container(tmp.path(), "report.pdf", b"a");
        let crashed = container(tmp.path(), "notes.txt", b"b");

        let left = session::create(&root, &crashed, "notes.txt").unwrap();
        extract::extract(&mut slpc::Container::open(&crashed).unwrap(), &left).unwrap();
        fs::write(left.payload_path(), b"edited").unwrap();

        let launcher = Recording::default();
        let mut r = Resident::new(&root);
        ok(r.handle(Request::Open(open_one), &Default_, &launcher));

        let lines = ok(r.handle(Request::List, &Default_, &launcher));
        assert!(lines
            .iter()
            .any(|l| l.contains("report.pdf") && l.contains("open")));
        assert!(lines
            .iter()
            .any(|l| l.contains("notes.txt") && l.contains("edited")));
    }

    #[test]
    fn the_sweep_takes_the_quiet_ones_and_leaves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let a = container(tmp.path(), "quiet.pdf", b"a");
        let b = container(tmp.path(), "edited.pdf", b"b");

        let quiet = session::create(&root, &a, "quiet.pdf").unwrap();
        extract::extract(&mut slpc::Container::open(&a).unwrap(), &quiet).unwrap();

        let edited = session::create(&root, &b, "edited.pdf").unwrap();
        extract::extract(&mut slpc::Container::open(&b).unwrap(), &edited).unwrap();
        fs::write(edited.payload_path(), b"an edit that never landed").unwrap();

        let half_made = session::create(&root, &a, "quiet.pdf").unwrap();

        assert_eq!(sweep(&root, &[]).unwrap(), 2);
        let left = session::scan(&root).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].dir(), edited.dir());
        assert!(!half_made.dir().exists());
    }

    #[test]
    fn the_sweep_will_not_touch_a_live_session() {
        // The reason this could not be written before Phase 2. A session that
        // is open and not yet edited reads as unchanged, and deleting it would
        // take the directory out from under a running editor.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let launcher = Recording::default();
        let mut r = Resident::new(&root);
        ok(r.handle(Request::Open(c), &Default_, &launcher));

        let live: Vec<_> = session::scan(&root)
            .unwrap()
            .iter()
            .map(|s| s.dir().to_path_buf())
            .collect();
        assert!(matches!(
            recover::state(&session::scan(&root).unwrap()[0]),
            recover::State::Unchanged
        ));

        assert_eq!(sweep(&root, &live).unwrap(), 0);
        assert_eq!(session::scan(&root).unwrap().len(), 1);
        // And it would have gone, had the sweep not been told.
        assert_eq!(sweep(&root, &[]).unwrap(), 1);
    }

    #[test]
    fn a_ping_is_answered_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = Resident::new(tmp.path().join("sessions"));
        assert_eq!(
            r.handle(Request::Ping, &Default_, &Recording::default()),
            Response::Ok(Vec::new())
        );
        assert!(r.is_idle());
    }
}
