//! The instance that holds the sessions, and the loop that serves it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8. One process holds every open session; every other invocation
//! reaches it through the front door and exits.
//!
//! **Requests are served on the loop's own thread and the accepting is not.** A
//! blocking `accept` would starve the watchers, and the watchers are the whole
//! reason this process exists. So a thread does nothing but accept and hand
//! connections over, and the loop alternates between answering one, pumping the
//! sessions, and collecting whatever concept 9's channel has been told.
//!
//! ## Three lists rather than one
//!
//! A session is open, or it is closed and the application has not finished with
//! it (concept 6.2), or a question has been asked about it and nothing may
//! happen until somebody answers (concept 6.3). Concept 8 says the process
//! lives while any of the three exists and stops when none does, so they are
//! three lists and `is_idle` is all of them being empty.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::flow::{self, Lingering, Opened};
use crate::ipc::{Request, Response, Voice};
use crate::outside::Outside;
use crate::present::{Answer, Choice, Question, Report};
use crate::recover;
use crate::session::{self, Session};
use crate::table::Table;

/// How long the loop waits for a request before going round to pump.
const TICK: Duration = Duration::from_millis(250);

/// How long a closed session's payload directory must go untouched before the
/// application is taken to have finished with it (concept 8).
///
/// Short, because it is the second of two conditions rather than the whole
/// test: the siblings have to be gone as well, and an application that has
/// cleaned up after itself is not about to write again. What this guards is the
/// gap between the last write and the last unlink.
const SETTLED: Duration = Duration::from_secs(2);

/// How long the instance stays alive holding a question nobody has answered.
///
/// **A judgement, and worth naming as one.** Concept 8 says to bound the linger
/// and does not say where, because there is no measurement that settles it:
/// what it trades is a resident process against a question that can still be
/// acted on. Five minutes is long enough that somebody who saw the notification
/// and finished a sentence first still finds the buttons live, and short enough
/// that a process nobody is talking to does not sit there for the afternoon.
/// When it expires the question is taken back rather than left standing, and
/// what replaces it says how to reach the same decision from the command line —
/// a button that does nothing is worse than no button.
const HELD: Duration = Duration::from_secs(300);

/// How long the icon shows that a save is going back into its container.
///
/// Long enough to be seen and short enough not to be a state. A repack of an
/// ordinary document is faster than this, so the wait is not the work — it is
/// the acknowledgement, and it exists because pressing Save and seeing nothing
/// change anywhere is what makes somebody doubt the tool is running.
const PULSE: Duration = Duration::from_millis(900);

/// A question the instance has asked and is holding open so that it can act on
/// the answer.
struct Pending {
    /// The session it is about, as `sessions` names it.
    about: String,
    /// What the answer acts on.
    session: Session,
    /// A container to open once this is settled, where the question was raised
    /// by an `open` that could not proceed until it was (concept 8).
    then_open: Option<PathBuf>,
    /// When it was last put in front of somebody. Reset by a `Reveal`, which is
    /// somebody saying *not yet* rather than declining to answer.
    asked: Instant,
}

/// The sessions this instance is holding.
pub struct Resident {
    root: PathBuf,
    sessions: Table<Opened>,
    lingering: Vec<Lingering>,
    pending: Vec<Pending>,
    /// [`SETTLED`] and [`HELD`], held rather than read, so that the rules they
    /// express can be tested instead of waited out. Nothing outside the tests
    /// changes them: a five-minute hold is not a setting concept 10 asks for,
    /// and making it one would put a second answer to *how long* in a file.
    settles_after: Duration,
    holds_for: Duration,
    /// What the standing list is coloured for, until somebody puts it down.
    ///
    /// **Held here rather than only spoken**, because a notification is a
    /// moment and most of these are not. A container that would not open leaves
    /// no session behind to re-read the trouble from, so the toast saying so
    /// was the only record there was, and a toast that was missed is a trouble
    /// that never happened.
    troubles: Vec<crate::present::Trouble>,
    /// When a save last went back into a container, for the moment of colour
    /// that says so. `None` until one has.
    wrote_back: Option<Instant>,
}

impl Resident {
    /// An instance holding nothing, keeping its sessions under `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            sessions: Table::new(),
            lingering: Vec::new(),
            pending: Vec::new(),
            settles_after: SETTLED,
            holds_for: HELD,
            troubles: Vec::new(),
            wrote_back: None,
        }
    }

    /// Set the two waits, for the tests of what happens on either side of
    /// them. A five-minute hold has no other way of being tested, and a
    /// two-second settle has no other way of being tested quickly.
    #[cfg(test)]
    fn waiting(mut self, settles_after: Duration, holds_for: Duration) -> Self {
        self.settles_after = settles_after;
        self.holds_for = holds_for;
        self
    }

    /// Whether there is anything left to hold, which is concept 8's exit rule.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.sessions.is_empty() && self.lingering.is_empty() && self.pending.is_empty()
    }

    /// Take on a trouble, for the icon to carry and the menu to explain.
    ///
    /// The same one twice is one trouble. A container that will not open is a
    /// container somebody is likely to double-click again, and three identical
    /// lines in a menu say nothing the first did not.
    fn note(&mut self, mood: crate::present::Mood, id: impl Into<String>, summary: String) {
        let id = id.into();
        if let Some(had) = self.troubles.iter_mut().find(|t| t.id == id) {
            had.mood = mood;
            had.summary = summary;
            return;
        }
        self.troubles
            .push(crate::present::Trouble { id, mood, summary });
    }

    /// Put down a trouble that has been read.
    fn dismiss(&mut self, id: &str) {
        self.troubles.retain(|t| t.id != id);
    }

    /// What the standing list is coloured for.
    #[must_use]
    pub fn troubles(&self) -> &[crate::present::Trouble] {
        &self.troubles
    }

    /// What colour the icon is, which is the worst thing currently true.
    ///
    /// **Two sources, and they are different in kind.** A trouble is remembered
    /// until somebody puts it down, because it is a moment that would otherwise
    /// be gone. A session waiting on a decision is not remembered at all — it
    /// is read off the sessions every time, so answering the question is what
    /// clears the colour, with nothing to dismiss and nothing that can fall out
    /// of step with what is on disk.
    #[must_use]
    pub fn mood(&self) -> crate::present::Mood {
        use crate::present::Mood;
        let worst = self
            .troubles
            .iter()
            .map(|t| t.mood)
            .max()
            .unwrap_or(Mood::Settled);
        let waiting = if self.pending.is_empty() {
            Mood::Settled
        } else {
            Mood::Look
        };
        let saving = match self.wrote_back {
            Some(at) if at.elapsed() < PULSE => Mood::Working,
            _ => Mood::Settled,
        };
        worst.max(waiting).max(saving)
    }

    /// Answer one request.
    pub fn handle(&mut self, request: Request, outside: &Outside<'_>) -> Response {
        match request {
            Request::Ping => Response::Ok(Vec::new()),
            Request::List => self.list(),
            Request::Open { container, voice } => self.open(&container, voice, outside),
            Request::Close(id) => self.close(&id),
        }
    }

    /// Open a container, or bring forward the session that already has it.
    fn open(&mut self, container: &Path, voice: Voice, outside: &Outside<'_>) -> Response {
        // Concept 8: a container that already has a live session is not opened
        // twice. Two sessions would both repack it and the second write-back
        // would overwrite the first with nothing said. Re-launching is what a
        // second double-click on an open document does everywhere else.
        if let Some(open) = self.sessions.find_mut(container) {
            return match outside.launcher.launch(&open.payload_path()) {
                Ok(()) => say(
                    voice,
                    outside,
                    Report::ordinary(format!(
                        "{} is already open; brought forward.",
                        slpc::display_name(&open.session().record().payload)
                    )),
                ),
                Err(e) => refuse(voice, outside, format!("could not bring it forward: {e}")),
            };
        }

        // Concept 8: a pending recovery item is resolved first. A session left
        // by a crash is not in the live table, so nothing refuses it — but
        // opening a fresh one would extract the container's current payload and
        // leave the recovered edit with nowhere to go.
        match self.ask_about_what_was_left(container, outside) {
            Err(e) => return refuse(voice, outside, e),
            Ok(true) => {
                return refuse(
                    voice,
                    outside,
                    "a session on this container was left behind, and what to do with it \
                     comes first"
                        .to_string(),
                )
            }
            Ok(false) => {}
        }

        match flow::open(&self.root, container, outside) {
            Err(e) => {
                let named = container.file_name().map_or_else(
                    || container.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                // **The one refusal that is spoken twice.** Concept 5.1's check
                // fires close to never and means one thing when it does, and
                // the person is holding a file somebody sent them believing it
                // is a document. A notification they may not look at is not
                // enough for that, so it is insisted on as well as recorded —
                // and the icon goes red, which is the only thing red is for.
                if let flow::Error::Misrepresented(what) = &e {
                    outside.channel.insist(
                        &Report::interrupt(format!("{named} was not opened."))
                            .and(format!(
                                "Its payload is {}, not a document.",
                                what.describes()
                            ))
                            .and("That is the shape of a phishing attachment.")
                            .and("Nothing was extracted and nothing was run."),
                    );
                    self.note(
                        crate::present::Mood::Danger,
                        format!("content:{}", container.display()),
                        format!("{named} - is {}, not a document", what.describes()),
                    );
                    // `refuse` deliberately skipped: it would say "Not opened"
                    // through the same channel that has just been insisted at,
                    // which on Windows is a box and two toasts for one event.
                    // What was insisted on is the better sentence of the two,
                    // and the command line still gets this one back.
                    return Response::Err(e.to_string());
                }
                // Everything else: nothing was extracted and nothing is at
                // risk, so this is a look rather than a warning — but it is
                // still the case the standing list exists for. A double-click
                // that produces no document and no window has to say something
                // that outlasts a banner, or the tool appears not to work.
                self.note(
                    crate::present::Mood::Look,
                    format!("open:{}", container.display()),
                    format!("{named} - did not open: {e}"),
                );
                refuse(voice, outside, e.to_string())
            }
            Ok(opened) => {
                let name = slpc::display_name(&opened.session().record().payload).into_owned();
                let mut report = Report::routine(format!("{name} is open."))
                    .and(format!("Session {}", id_of(opened.session())));
                if opened.mark != slpc::provenance::Mark::Silent {
                    report = report.and("It came from somewhere else, and the copy says so.");
                }
                if let Err(e) = self.sessions.insert(container, opened) {
                    return refuse(
                        voice,
                        outside,
                        format!("the session could not be tracked: {e}"),
                    );
                }
                say(voice, outside, report)
            }
        }
    }

    /// Put concept 6.3's question about a session left behind on this
    /// container, where there is one worth asking about, and hold it.
    ///
    /// Answers `true` where the open must wait for it. Concept 8: *the recovery
    /// question comes first, and the new session follows the answer* — so the
    /// container is remembered against the question and opened once it is
    /// settled, rather than the person having to double-click a second time.
    fn ask_about_what_was_left(
        &mut self,
        container: &Path,
        outside: &Outside<'_>,
    ) -> Result<bool, String> {
        let want = crate::identity::of(container).map_err(|e| e.to_string())?;
        let sessions = session::scan(&self.root).map_err(|e| e.to_string())?;
        for left in sessions {
            // A container the record names that has since gone cannot be the
            // one being opened, so it is not this invocation's business.
            if !crate::identity::of(&left.record().container).is_ok_and(|is| is == want) {
                continue;
            }
            let state = recover::state(&left);
            if !state.needs_a_person() {
                continue;
            }
            let about = id_of(&left);
            // Already asked, and still waiting. Asking again would put a second
            // copy of the same question in the message list, and answering
            // either would leave the other one behind.
            if self.pending.iter().any(|p| p.about == about) {
                return Ok(true);
            }
            outside.channel.ask(&Question {
                about: about.clone(),
                summary: format!(
                    "{} was left behind.",
                    slpc::display_name(&left.record().payload)
                ),
                // Short, because a notification body is one paragraph however
                // it is written. The container is named because that is what
                // the decision is about; the session directory is not, because
                // `Reveal` is the button that opens it.
                detail: vec![
                    format!("It is {state}."),
                    format!("From {}.", slpc::display_path(&left.record().container)),
                    "It will open once you have decided.".into(),
                ],
                choices: vec![Choice::WriteBack, Choice::Discard, Choice::Reveal],
            });
            self.pending.push(Pending {
                about,
                session: left,
                then_open: Some(container.to_path_buf()),
                asked: Instant::now(),
            });
            return Ok(true);
        }
        Ok(false)
    }

    /// Every session, with the id apart from the words.
    ///
    /// One source for both surfaces: `list` puts the id back in front and hands
    /// the lines to the command line, and concept 12's standing list takes the
    /// pieces, because a menu needs the id to act on and no room to show it.
    pub(crate) fn listed(&self) -> Vec<crate::present::Listed> {
        let mut out: Vec<crate::present::Listed> = self
            .sessions
            .iter()
            .map(|o| crate::present::Listed {
                id: id_of(o.session()),
                payload: slpc::display_name(&o.session().record().payload).into_owned(),
                label: format!(
                    "{}  open, {} write-back(s)",
                    slpc::display_name(&o.session().record().payload),
                    o.session().record().write_backs
                ),
                live: true,
                needs_a_person: false,
                write_backs: Some(o.session().record().write_backs),
            })
            .collect();
        out.extend(self.lingering.iter().map(|l| crate::present::Listed {
            id: id_of(l.session()),
            payload: slpc::display_name(&l.session().record().payload).into_owned(),
            label: format!(
                "{}  closed, waiting for the application to finish",
                slpc::display_name(&l.session().record().payload)
            ),
            live: true,
            needs_a_person: false,
            write_backs: Some(l.session().record().write_backs),
        }));
        out.extend(self.pending.iter().map(|p| crate::present::Listed {
            id: p.about.clone(),
            payload: slpc::display_name(&p.session.record().payload).into_owned(),
            label: format!(
                "{}  {}, waiting for you",
                slpc::display_name(&p.session.record().payload),
                recover::state(&p.session)
            ),
            live: false,
            needs_a_person: true,
            write_backs: None,
        }));

        // Everything above is also a directory under the root, so a scan that
        // did not know what was held would report each of them twice.
        let held = self.held_directories();
        if let Ok(left) = session::scan(&self.root) {
            for s in left
                .iter()
                .filter(|s| !held.contains(&s.dir().to_path_buf()))
            {
                out.push(crate::present::Listed {
                    id: id_of(s),
                    payload: slpc::display_name(&s.record().payload).into_owned(),
                    label: format!(
                        "{}  {}",
                        slpc::display_name(&s.record().payload),
                        recover::state(s)
                    ),
                    live: false,
                    // Concept 6.3: what needs nobody is swept and never spoken
                    // of. Listing it in a standing surface is furniture.
                    needs_a_person: recover::state(s).needs_a_person(),
                    write_backs: None,
                });
            }
        }
        out
    }

    pub(crate) fn list(&self) -> Response {
        let mut lines: Vec<String> = self
            .listed()
            .into_iter()
            .map(|e| format!("{}  {}", e.id, e.label))
            .collect();
        if lines.is_empty() {
            lines.push("No sessions.".into());
        }
        Response::Ok(lines)
    }

    /// Every session directory this instance is holding, in any of its three
    /// lists.
    fn held_directories(&self) -> Vec<PathBuf> {
        self.sessions
            .iter()
            .map(|o| o.session().dir().to_path_buf())
            .chain(
                self.lingering
                    .iter()
                    .map(|l| l.session().dir().to_path_buf()),
            )
            .chain(self.pending.iter().map(|p| p.session.dir().to_path_buf()))
            .collect()
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
            // Concept 6.2 and 8. The close is honoured; the directory is not
            // removed, and the watch stays on it so that the application's last
            // save is noticed when it happens rather than at the next launch.
            Ok(flow::Closed::LeftForRecovery(lingering)) => {
                self.lingering.push(lingering);
                Response::Ok(vec![
                    "Session closed, and the application still has the payload open.".into(),
                    "It is being watched until the application finishes.".into(),
                ])
            }
            Err(e) => Response::Err(e.to_string()),
        }
    }

    /// One turn: pump the open sessions, move on whatever has settled, and act
    /// on whatever has been answered.
    pub fn turn(&mut self, outside: &Outside<'_>) {
        self.pump_all(outside);
        self.ask_about_what_has_settled(outside);
        self.act_on_answers(outside);
        self.let_go_of_the_unanswered(outside);
    }

    /// Give every open session's watch a turn, and report what came of it.
    fn pump_all(&mut self, outside: &Outside<'_>) {
        let mut wrote_back = Vec::new();
        let mut landed = Vec::new();
        let mut failed = Vec::new();
        for open in self.sessions.iter_mut() {
            match open.pump() {
                Ok(true) => {
                    let s = open.session();
                    // **The first of a session and not each one.** A write-back
                    // fires on every save, so an hour's editing is dozens of
                    // notifications for the same fact. The first is worth
                    // saying, because it is how somebody learns the loop works
                    // at all; the rest are the tool congratulating itself.
                    // Concept 6.2 wants a session visible and wants somewhere
                    // to look when an edit is expected to have landed, and
                    // `sessions` answers that better than a stream of banners.
                    if s.record().write_backs <= 1 {
                        outside.report(
                            &Report::routine(format!(
                                "{} written back.",
                                slpc::display_name(&s.record().payload)
                            ))
                            .and("Saves from here on are written back quietly."),
                        );
                    }
                    landed.push(id_of(s));
                    wrote_back.push(s.record().container.clone());
                }
                Ok(false) => {}
                // One failing container is not a reason to stop watching the
                // rest, and it is a reason to say so: concept 6.2 puts the
                // close at the user's hand, and a save that did not land is the
                // thing they most need to know did not.
                Err(e) => {
                    let name = slpc::display_name(&open.session().record().payload).into_owned();
                    outside.report(
                        &Report::interrupt(format!("{name} could not be written back."))
                            .and(e.to_string()),
                    );
                    failed.push((id_of(open.session()), name));
                }
            }
        }
        // A write-back renamed a new file over the container, so the identity
        // recorded when the session opened is stale. See `table::refresh`.
        if !wrote_back.is_empty() {
            // The moment of colour that says a save went home. Set from a
            // write-back having happened rather than from an event having
            // arrived, which is the same rule `pump` itself decides by.
            self.wrote_back = Some(Instant::now());
        }
        for container in wrote_back {
            self.sessions.refresh(&container);
        }
        // The promise this tool makes is that a save reaches the container, and
        // this is that promise outstanding. It stays on the icon until somebody
        // puts it down, because the alternative is a banner that flashed past
        // while they were typing into the very document that did not save.
        for (id, name) in failed {
            self.note(
                crate::present::Mood::AtRisk,
                format!("writeback:{id}"),
                format!("{name} - a save did not reach its container"),
            );
        }
        // And it goes when a later save from the same session lands, without
        // anybody dismissing anything. The colour is a claim about right now,
        // so a claim the next save disproves has to answer to it.
        for id in landed {
            self.dismiss(&format!("writeback:{id}"));
        }
    }

    /// Ask about every lingering session the application has finished with.
    fn ask_about_what_has_settled(&mut self, outside: &Outside<'_>) {
        let mut still_waiting = Vec::new();
        for mut lingering in std::mem::take(&mut self.lingering) {
            if !lingering.has_settled(self.settles_after) {
                still_waiting.push(lingering);
                continue;
            }
            let about = id_of(lingering.session());
            let session = lingering.into_session();
            let state = recover::state(&session);
            let name = slpc::display_name(&session.record().payload).into_owned();
            // Concept 6.3: equal to what the container holds means nothing was
            // lost, so clean up and say nothing. This is that case arriving
            // while the process is still alive to see it, which is what concept
            // 8 keeps it alive for.
            if !state.needs_a_person() {
                let _ = session.remove();
                continue;
            }
            outside.channel.ask(&Question {
                about: about.clone(),
                summary: format!("{name} was saved after you closed the session."),
                detail: vec![
                    format!("It is {state}."),
                    format!("Into {}.", slpc::display_path(&session.record().container)),
                ],
                choices: vec![Choice::WriteBack, Choice::Discard, Choice::Reveal],
            });
            self.pending.push(Pending {
                about,
                session,
                then_open: None,
                asked: Instant::now(),
            });
        }
        self.lingering = still_waiting;
    }

    /// Do what somebody chose.
    fn act_on_answers(&mut self, outside: &Outside<'_>) {
        for answer in outside.channel.answers() {
            self.settle(&answer, outside);
        }
    }

    fn settle(&mut self, answer: &Answer, outside: &Outside<'_>) {
        let Some(at) = self.pending.iter().position(|p| p.about == answer.about) else {
            // A button from a question this instance is no longer holding: the
            // same decision taken at the command line in the meantime, or a
            // notification that outlived the process that asked. Saying so
            // beats a click that appears to do nothing.
            outside.report(&Report::ordinary(format!(
                "{} has already been dealt with.",
                answer.about
            )));
            return;
        };

        // Reveal is *not yet* rather than an answer, so the question stays and
        // is put again. A service that closes a notification when one of its
        // actions is invoked — which GNOME Shell does — would otherwise leave
        // somebody looking at the folder with no way back to the decision.
        if answer.choice == Choice::Reveal {
            let pending = &mut self.pending[at];
            pending.asked = Instant::now();
            let dir = pending.session.payload_dir();
            let question = Question {
                about: pending.about.clone(),
                summary: format!(
                    "{} is still waiting.",
                    slpc::display_name(&pending.session.record().payload)
                ),
                detail: vec![format!("The payload is in {}", dir.display())],
                choices: vec![Choice::WriteBack, Choice::Discard, Choice::Reveal],
            };
            if let Err(e) = outside.launcher.launch(&dir) {
                outside.report(&Report::ordinary(format!(
                    "{} could not be shown: {e}",
                    dir.display()
                )));
            }
            outside.channel.ask(&question);
            return;
        }

        let mut pending = self.pending.remove(at);
        outside.channel.withdraw(&pending.about);
        // Owned, because what follows moves the session out from under it.
        let name = slpc::display_name(&pending.session.record().payload).into_owned();
        match answer.choice {
            Choice::WriteBack => match crate::writeback::write_back(&mut pending.session) {
                Ok(()) => {
                    outside.report(&Report::ordinary(format!(
                        "{name} written back to {}.",
                        slpc::display_path(&pending.session.record().container)
                    )));
                    let _ = pending.session.remove();
                }
                Err(e) => {
                    // Nothing is removed. The session stays where it
                    // was, which is what makes a second attempt possible, and
                    // the question goes back so there is something to make it
                    // with.
                    outside.report(
                        &Report::interrupt(format!("{name} could not be written back."))
                            .and(e.to_string()),
                    );
                    pending.asked = Instant::now();
                    self.pending.push(pending);
                    return;
                }
            },
            Choice::Discard => {
                let _ = pending.session.remove();
                outside.report(&Report::ordinary(format!("{name} discarded.")));
            }
            Choice::Reveal => unreachable!("answered above"),
        }

        // Concept 8: the new session follows the answer.
        if let Some(container) = pending.then_open {
            if let Response::Err(why) = self.open(&container, Voice::Instance, outside) {
                outside.report(&Report::interrupt(format!("{name} did not open: {why}")));
            }
        }
    }

    /// Take back the questions nobody has answered inside [`HELD`], and say
    /// where the same decision still lives.
    fn let_go_of_the_unanswered(&mut self, outside: &Outside<'_>) {
        let (gone, kept) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition::<Vec<_>, _>(|p| p.asked.elapsed() >= self.holds_for);
        self.pending = kept;
        for p in &gone {
            Self::stop_asking(p, outside);
        }
    }

    /// Withdraw one question and leave the command line in its place.
    fn stop_asking(pending: &Pending, outside: &Outside<'_>) {
        outside.channel.withdraw(&pending.about);
        outside.report(
            &Report::ordinary(format!(
                "{} is still undecided.",
                slpc::display_name(&pending.session.record().payload)
            ))
            .and(format!(
                "slipcase-open recover {} --write-back",
                pending.about
            ))
            .and(format!("slipcase-open recover {} --discard", pending.about)),
        );
    }

    /// Close every open session, and put down every question, for a shutdown
    /// that is not a crash.
    ///
    /// A question left standing when this process goes is a button with nobody
    /// behind it, so each is withdrawn and replaced by the two commands that
    /// reach the same decision. The session directories stay: concept 6.3
    /// carries them to the next launch, which is where they were always going
    /// to be answered if nobody answered here.
    pub fn stand_down(&mut self, outside: &Outside<'_>) {
        for open in self.sessions.drain().collect::<Vec<_>>() {
            match open.close() {
                Ok(flow::Closed::Cleared) => {}
                Ok(flow::Closed::LeftForRecovery(lingering)) => self.lingering.push(lingering),
                Err(e) => {
                    outside.report(&Report::interrupt(format!("a session did not close: {e}")));
                }
            }
        }
        for lingering in std::mem::take(&mut self.lingering) {
            let session = lingering.into_session();
            if recover::state(&session).needs_a_person() {
                outside.report(
                    &Report::ordinary(format!(
                        "{} was closed while its application was still working.",
                        slpc::display_name(&session.record().payload)
                    ))
                    .and("It is left for recovery: run `slipcase-open sessions`."),
                );
            } else {
                let _ = session.remove();
            }
        }
        for pending in &std::mem::take(&mut self.pending) {
            Self::stop_asking(pending, outside);
        }
    }
}

/// The name `list` prints and `close` takes back.
fn id_of(s: &Session) -> String {
    s.dir()
        .file_name()
        .map_or_else(|| "?".to_string(), |n| n.to_string_lossy().into_owned())
}

/// A report said once: through the channel where the client has nowhere to show
/// it, and back to the client where it has.
fn say(voice: Voice, outside: &Outside<'_>, report: Report) -> Response {
    if voice == Voice::Instance {
        outside.report(&report);
    }
    Response::Ok(
        std::iter::once(report.summary)
            .chain(report.detail.into_iter().map(|d| format!("  {d}")))
            .collect(),
    )
}

/// A refusal said once, on the same rule.
///
/// Weighted as an interrupt through the channel. A refusal answers something
/// somebody just double-clicked, and concept 5.1's extensionless case is one of
/// the things it can be: a quiet refusal there reads as the document simply not
/// opening.
fn refuse(voice: Voice, outside: &Outside<'_>, why: String) -> Response {
    if voice == Voice::Instance {
        outside.report(&Report::interrupt("Not opened.").and(why.clone()));
    }
    Response::Err(why)
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
pub fn run(
    listener: crate::endpoint::Listener,
    resident: &mut Resident,
    outside: &Outside<'_>,
    standing: &dyn crate::present::Standing,
) -> io::Result<()> {
    let mut shown: Vec<crate::present::Listed> = Vec::new();
    let mut carried: Vec<crate::present::Trouble> = Vec::new();
    let mut wearing = crate::present::Mood::Settled;
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
                    Ok(request) => resident.handle(request, outside),
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

        resident.turn(outside);

        // Concept 12's standing list, told what to say and asked what was
        // said back. Only when it changed: this is every 250 milliseconds, and
        // all three are the same on almost all of them.
        //
        // The mood is in that comparison and it is the one that moves on its
        // own — [`PULSE`] expires with nothing else happening — which is what
        // takes the icon back to blue after a save without needing a timer of
        // its own anywhere.
        let listed = resident.listed();
        let troubles = resident.troubles().to_vec();
        let mood = resident.mood();
        if (&listed, &troubles, mood) != (&shown, &carried, wearing) {
            standing.show(&listed, &troubles, mood);
            shown = listed;
            carried = troubles;
            wearing = mood;
        }
        let mut leaving = false;
        for chosen in standing.taken() {
            match chosen {
                // Somebody has read it. Nothing else happens: the trouble was
                // the record that it happened at all, and putting it down is
                // the person saying they have the record now.
                crate::present::Chosen::Dismiss(id) => resident.dismiss(&id),
                // The same ending as interrupting the command line, and the
                // menu item says so: every session stays where it is and stays
                // recoverable.
                crate::present::Chosen::Quit => leaving = true,
            }
        }
        if leaving {
            break;
        }

        // Concept 8's exit rule: nothing open, nothing lingering, nothing
        // waiting on somebody. Staying resident does nothing for the crash
        // case, where this process is dead by definition.
        //
        // **Where there is a standing list the rule does not apply**, and that
        // is a change rather than an exception — see
        // [`crate::present::Standing::holding`]. The rule was written for a
        // process with no face: nothing for it to be, so no reason for it to
        // be. An icon gives warnings somewhere to live, and a warning raised by
        // a process on its way out has nowhere to go.
        if resident.is_idle() && !standing.holding() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{sweep, Resident};
    use crate::ipc::{Request, Response, Voice};
    use crate::outside::Outside;
    use crate::platform::testing::Recording;
    use crate::policy::{Origin, Read, Source};
    use crate::present::testing::Recording as Told;
    use crate::present::Choice;
    use crate::{extract, recover, session};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    struct Default_;
    impl Source for Default_ {
        fn layer(&self, _o: Origin) -> Read {
            Ok(None)
        }
    }

    /// The three things the engine works against, kept together so a test can
    /// hand them over as one and then ask each of them what it saw.
    struct World {
        policy: Default_,
        launcher: Recording,
        channel: Told,
    }

    impl World {
        fn new() -> Self {
            Self {
                policy: Default_,
                launcher: Recording::default(),
                channel: Told::default(),
            }
        }

        fn outside(&self) -> Outside<'_> {
            Outside::new(&self.policy, &self.launcher, &self.channel)
        }

        /// The same, with the routine reports let through.
        fn loud(&self) -> Outside<'_> {
            self.outside().saying(crate::policy::Notify::Everything)
        }
    }

    /// An `open` from somewhere with a terminal, which is what most of these
    /// are: the response is the assertion.
    fn opening(container: PathBuf) -> Request {
        Request::Open {
            container,
            voice: Voice::Client,
        }
    }

    /// An `open` from a double-click, where the instance has to speak.
    fn announcing(container: PathBuf) -> Request {
        Request::Open {
            container,
            voice: Voice::Instance,
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

    /// The session name the `open` response gives back.
    fn session_named_in(lines: &[String]) -> String {
        lines
            .iter()
            .find_map(|l| l.strip_prefix("  Session "))
            .unwrap_or_else(|| panic!("no session named in {lines:?}"))
            .to_string()
    }

    /// A session left behind by a process that died with an edit in it.
    fn a_crashed_session(root: &Path, c: &Path, name: &str, edit: &[u8]) -> session::Session {
        let left = session::create(root, c, name).unwrap();
        extract::extract(&mut slpc::Container::open(c).unwrap(), &left).unwrap();
        fs::write(left.payload_path(), edit).unwrap();
        left
    }

    #[test]
    fn opening_a_container_twice_brings_the_session_forward() {
        // Concept 8. Two sessions would both repack it and the second
        // write-back would overwrite the first with nothing said.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let w = World::new();
        let mut r = Resident::new(&root);

        ok(r.handle(opening(c.clone()), &w.outside()));
        let again = ok(r.handle(opening(c.clone()), &w.outside()));

        assert!(again[0].contains("already open"), "{again:?}");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
        // Brought forward means launched again, which is what a second
        // double-click does everywhere else.
        assert_eq!(w.launcher.launched().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn the_same_container_under_another_hard_link_is_the_same_session() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let link = tmp.path().join("other-name.slpc");
        fs::hard_link(&c, &link).unwrap();
        let w = World::new();
        let mut r = Resident::new(&root);

        ok(r.handle(opening(c), &w.outside()));
        let again = ok(r.handle(opening(link), &w.outside()));
        assert!(again[0].contains("already open"), "{again:?}");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn two_different_containers_get_two_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let a = container(tmp.path(), "report.pdf", b"a");
        let b = container(tmp.path(), "notes.txt", b"b");
        let w = World::new();
        let mut r = Resident::new(&root);

        ok(r.handle(opening(a), &w.outside()));
        ok(r.handle(opening(b), &w.outside()));
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
        let w = World::new();
        let mut r = Resident::new(&root);

        ok(r.handle(opening(c.clone()), &w.outside()));
        let payload = session::scan(&root).unwrap()[0].payload_path();
        fs::write(&payload, b"edited").unwrap();

        // Watched through the record rather than through what was said. The
        // write-back count is what `pump` guarantees; a notification is a
        // presentation choice that a threshold may now drop.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let saved = || session::scan(&root).unwrap()[0].record().write_backs;
        while std::time::Instant::now() < deadline && saved() == 0 {
            r.turn(&w.outside());
        }
        assert!(saved() >= 1, "nothing was written back");

        let again = ok(r.handle(opening(c), &w.outside()));
        assert!(again[0].contains("already open"), "{again:?}");
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn a_recovery_item_on_the_same_container_is_asked_about_first() {
        // Concept 8. Opening a fresh session would extract the container's
        // current payload and leave the recovered edit with nowhere to go.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let left = a_crashed_session(&root, &c, "report.pdf", b"edited then the process died");

        let w = World::new();
        let mut r = Resident::new(&root);
        let refused = err(r.handle(opening(c), &w.outside()));

        assert!(refused.contains("left behind"), "{refused}");
        assert!(
            w.launcher.launched().is_empty(),
            "nothing should have opened"
        );
        assert_eq!(session::scan(&root).unwrap().len(), 1);

        // Concept 9 turns the Phase 2 refusal into a question, and it has to
        // carry all three of concept 6.3's answers.
        let asked = w.channel.questions();
        assert_eq!(asked.len(), 1);
        assert!(asked[0]
            .about
            .starts_with(left.dir().file_name().unwrap().to_str().unwrap()));
        assert_eq!(
            asked[0].choices,
            vec![Choice::WriteBack, Choice::Discard, Choice::Reveal]
        );
        // And the instance is holding it, which is what makes an answer
        // actionable rather than a button with nobody behind it.
        assert!(!r.is_idle());
    }

    #[test]
    fn one_question_is_asked_however_many_times_the_container_is_double_clicked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        a_crashed_session(&root, &c, "report.pdf", b"edited");

        let w = World::new();
        let mut r = Resident::new(&root);
        err(r.handle(opening(c.clone()), &w.outside()));
        err(r.handle(opening(c), &w.outside()));
        assert_eq!(w.channel.questions().len(), 1);
    }

    #[test]
    fn writing_back_a_recovered_session_opens_the_one_that_was_waiting() {
        // Concept 8: the new session follows the answer, so nobody has to
        // double-click the container a second time.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        a_crashed_session(&root, &c, "report.pdf", b"the edit that never landed");

        let w = World::new();
        let mut r = Resident::new(&root);
        err(r.handle(opening(c.clone()), &w.outside()));
        let about = w.channel.questions()[0].about.clone();

        w.channel.answer(&about, Choice::WriteBack);
        r.turn(&w.outside());

        // The edit is in the container, the question has been taken back, and
        // the session that was waiting on the answer is open.
        let mut held = slpc::Container::open(&c).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut held.payload().unwrap(), &mut bytes).unwrap();
        assert_eq!(bytes, b"the edit that never landed");
        assert_eq!(w.channel.withdrawn(), vec![about]);
        assert_eq!(w.launcher.launched().len(), 1);
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn discarding_a_recovered_session_opens_the_one_that_was_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        a_crashed_session(&root, &c, "report.pdf", b"edited");

        let w = World::new();
        let mut r = Resident::new(&root);
        err(r.handle(opening(c.clone()), &w.outside()));
        let about = w.channel.questions()[0].about.clone();

        w.channel.answer(&about, Choice::Discard);
        r.turn(&w.outside());

        // Not asserted by the directory being gone: a session is named for the
        // second it started in, so the one that follows the answer takes the
        // same name back. What says the edit was discarded is that the session
        // now on disk holds the container's payload rather than the edit.
        let now = session::scan(&root).unwrap();
        assert_eq!(now.len(), 1);
        assert_eq!(fs::read(now[0].payload_path()).unwrap(), b"first");
        let mut held = slpc::Container::open(&c).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut held.payload().unwrap(), &mut bytes).unwrap();
        assert_eq!(bytes, b"first", "discard must not touch the container");
        assert_eq!(w.launcher.launched().len(), 1);
    }

    #[test]
    fn revealing_shows_the_folder_and_puts_the_question_again() {
        // Reveal is *not yet* rather than an answer. A service that closes a
        // notification when one of its actions is invoked would otherwise leave
        // somebody looking at a folder with no way back to the decision.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let left = a_crashed_session(&root, &c, "report.pdf", b"edited");

        let w = World::new();
        let mut r = Resident::new(&root);
        err(r.handle(opening(c), &w.outside()));
        let about = w.channel.questions()[0].about.clone();

        w.channel.answer(&about, Choice::Reveal);
        r.turn(&w.outside());

        assert_eq!(w.launcher.launched(), vec![left.payload_dir()]);
        assert_eq!(w.channel.questions().len(), 2);
        assert!(w.channel.withdrawn().is_empty());
        assert!(left.dir().exists());
        assert!(!r.is_idle());
    }

    #[test]
    fn an_answer_about_a_session_nobody_is_holding_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let mut r = Resident::new(tmp.path().join("sessions"));
        w.channel.answer("gone-0", Choice::WriteBack);
        r.turn(&w.outside());
        assert!(
            w.channel.said().contains("already been dealt with"),
            "{}",
            w.channel.said()
        );
    }

    #[test]
    fn a_question_nobody_answers_is_taken_back_and_replaced_by_the_commands() {
        // Concept 8 says to bound the linger. What it costs is that the buttons
        // stop working, so they are removed rather than left to do nothing, and
        // what replaces them reaches the same decision.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let left = a_crashed_session(&root, &c, "report.pdf", b"edited");

        let w = World::new();
        let mut r = Resident::new(&root).waiting(Duration::ZERO, Duration::ZERO);
        err(r.handle(opening(c), &w.outside()));
        let about = w.channel.questions()[0].about.clone();

        r.turn(&w.outside());

        assert_eq!(w.channel.withdrawn(), vec![about.clone()]);
        assert!(w
            .channel
            .said()
            .contains(&format!("recover {about} --write-back")));
        assert!(w
            .channel
            .said()
            .contains(&format!("recover {about} --discard")));
        // The session itself stays. Concept 6.3 carries it to the next launch.
        assert!(left.dir().exists());
        assert!(r.is_idle());
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

        let w = World::new();
        let mut r = Resident::new(&root);
        ok(r.handle(opening(c), &w.outside()));
        assert_eq!(w.launcher.launched().len(), 1);
        assert!(w.channel.questions().is_empty());
    }

    #[test]
    fn closing_by_name_closes_that_session() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let w = World::new();
        let mut r = Resident::new(&root);

        let opened = ok(r.handle(opening(c), &w.outside()));
        ok(r.handle(Request::Close(session_named_in(&opened)), &w.outside()));
        assert!(r.is_idle());
        assert!(session::scan(&root).unwrap().is_empty());
    }

    #[test]
    fn closing_while_the_application_is_working_keeps_the_watch_on_it() {
        // Concept 6.2 and 8. The close is honoured, the directory stays, and
        // the process keeps watching so the application's last save is noticed
        // when it happens rather than at the next launch.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let w = World::new();
        let mut r = Resident::new(&root);

        let opened = ok(r.handle(opening(c), &w.outside()));
        let dir = session::scan(&root).unwrap()[0].payload_dir();
        fs::write(dir.join(".~lock.report.pdf#"), b"still working").unwrap();

        let closed = ok(r.handle(Request::Close(session_named_in(&opened)), &w.outside()));
        assert!(
            closed
                .iter()
                .any(|l| l.contains("still has the payload open")),
            "{closed:?}"
        );
        assert!(
            !r.is_idle(),
            "the process has to stay for the watch to be worth anything"
        );
        assert_eq!(session::scan(&root).unwrap().len(), 1);
    }

    #[test]
    fn a_lingering_session_saved_after_the_close_becomes_a_question() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let w = World::new();
        let mut r = Resident::new(&root).waiting(Duration::ZERO, Duration::from_secs(300));

        let opened = ok(r.handle(opening(c), &w.outside()));
        let dir = session::scan(&root).unwrap()[0].payload_dir();
        let sibling = dir.join(".~lock.report.pdf#");
        fs::write(&sibling, b"still working").unwrap();
        ok(r.handle(Request::Close(session_named_in(&opened)), &w.outside()));

        // The application's last save, and then it tidies up after itself.
        fs::write(dir.join("report.pdf"), b"the last save").unwrap();
        fs::remove_file(&sibling).unwrap();
        r.turn(&w.outside());

        let asked = w.channel.questions();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(asked[0].summary.contains("after you closed"), "{asked:?}");
        // Nothing was written back on its own. Concept 6.3: this tool was not
        // watching when the user said they were done, so it cannot tell a
        // complete save from a half-written one.
        let mut held =
            slpc::Container::open(&session::scan(&root).unwrap()[0].record().container).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut held.payload().unwrap(), &mut bytes).unwrap();
        assert_eq!(bytes, b"first");
    }

    #[test]
    fn a_lingering_session_that_matches_its_container_goes_quietly() {
        // Concept 6.3: equal means nothing was lost, so clean up and say
        // nothing. This is that case arriving while the process is still alive
        // to see it, rather than at the next launch.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let w = World::new();
        let mut r = Resident::new(&root).waiting(Duration::ZERO, Duration::from_secs(300));

        let opened = ok(r.handle(opening(c), &w.outside()));
        let dir = session::scan(&root).unwrap()[0].payload_dir();
        let sibling = dir.join(".~lock.report.pdf#");
        fs::write(&sibling, b"still working").unwrap();
        ok(r.handle(Request::Close(session_named_in(&opened)), &w.outside()));

        fs::remove_file(&sibling).unwrap();
        r.turn(&w.outside());

        assert!(
            w.channel.questions().is_empty(),
            "{:?}",
            w.channel.questions()
        );
        assert!(session::scan(&root).unwrap().is_empty());
        assert!(r.is_idle());
    }

    #[test]
    fn closing_a_session_that_is_not_open_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let mut r = Resident::new(tmp.path().join("sessions"));
        let refused = err(r.handle(Request::Close("nothing-0".into()), &w.outside()));
        assert!(refused.contains("no open session"), "{refused}");
    }

    #[test]
    fn a_double_click_is_spoken_for_and_a_terminal_is_not() {
        // Concept 9. An invocation with nowhere to print is why the instance
        // has a channel at all, and one with a terminal of its own would
        // otherwise hear everything twice.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let quiet = container(tmp.path(), "quiet.pdf", b"a");
        let loud = container(tmp.path(), "loud.pdf", b"b");
        let w = World::new();
        let mut r = Resident::new(&root);

        ok(r.handle(opening(quiet), &w.loud()));
        assert!(w.channel.reports().is_empty(), "{:?}", w.channel.reports());

        ok(r.handle(announcing(loud), &w.loud()));
        assert!(
            w.channel.said().contains("loud.pdf is open"),
            "{}",
            w.channel.said()
        );
    }

    #[test]
    fn listing_shows_what_is_open_and_what_was_left() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let open_one = container(tmp.path(), "report.pdf", b"a");
        let crashed = container(tmp.path(), "notes.txt", b"b");
        a_crashed_session(&root, &crashed, "notes.txt", b"edited");

        let w = World::new();
        let mut r = Resident::new(&root);
        ok(r.handle(opening(open_one), &w.outside()));

        let lines = ok(r.handle(Request::List, &w.outside()));
        assert!(lines
            .iter()
            .any(|l| l.contains("report.pdf") && l.contains("open")));
        assert!(lines
            .iter()
            .any(|l| l.contains("notes.txt") && l.contains("edited")));
        // And each of them once. A session the instance is holding is also on
        // disk, so a list that read both without checking would double it.
        assert_eq!(lines.len(), 2, "{lines:?}");
    }

    #[test]
    fn the_sweep_takes_the_quiet_ones_and_leaves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let a = container(tmp.path(), "quiet.pdf", b"a");
        let b = container(tmp.path(), "edited.pdf", b"b");

        let quiet = session::create(&root, &a, "quiet.pdf").unwrap();
        extract::extract(&mut slpc::Container::open(&a).unwrap(), &quiet).unwrap();

        let edited = a_crashed_session(&root, &b, "edited.pdf", b"an edit that never landed");
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
        let w = World::new();
        let mut r = Resident::new(&root);
        ok(r.handle(opening(c), &w.outside()));

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
    fn standing_down_takes_back_every_question_it_was_holding() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = container(tmp.path(), "report.pdf", b"first");
        let left = a_crashed_session(&root, &c, "report.pdf", b"edited");

        let w = World::new();
        let mut r = Resident::new(&root);
        err(r.handle(opening(c), &w.outside()));
        let about = w.channel.questions()[0].about.clone();

        r.stand_down(&w.outside());

        assert_eq!(w.channel.withdrawn(), vec![about.clone()]);
        assert!(w
            .channel
            .said()
            .contains(&format!("recover {about} --write-back")));
        assert!(
            left.dir().exists(),
            "the session carries the question to the next launch"
        );
        assert!(r.is_idle());
    }

    #[test]
    fn a_ping_is_answered_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let mut r = Resident::new(tmp.path().join("sessions"));
        assert_eq!(
            r.handle(Request::Ping, &w.outside()),
            Response::Ok(Vec::new())
        );
        assert!(r.is_idle());
    }

    #[test]
    fn nothing_wrong_is_the_ordinary_colour() {
        let tmp = tempfile::tempdir().unwrap();
        let r = Resident::new(tmp.path().join("sessions"));
        assert_eq!(r.mood(), crate::present::Mood::Settled);
        assert!(r.troubles().is_empty());
    }

    #[test]
    fn the_icon_wears_the_worst_thing_currently_true() {
        use crate::present::Mood;
        let tmp = tempfile::tempdir().unwrap();
        let mut r = Resident::new(tmp.path().join("sessions"));

        r.note(Mood::Look, "a", "one did not open".into());
        assert_eq!(r.mood(), Mood::Look);
        // Worse arrives and wins, whatever order they were taken on in.
        r.note(Mood::Danger, "b", "one is a program".into());
        assert_eq!(r.mood(), Mood::Danger);
        r.note(Mood::AtRisk, "c", "one did not save".into());
        assert_eq!(
            r.mood(),
            Mood::Danger,
            "a lesser trouble does not talk the icon down"
        );

        // And it comes back down as they are put down, rather than sticking at
        // the worst thing that ever happened.
        r.dismiss("b");
        assert_eq!(r.mood(), Mood::AtRisk);
        r.dismiss("c");
        assert_eq!(r.mood(), Mood::Look);
        r.dismiss("a");
        assert_eq!(r.mood(), Mood::Settled);
    }

    #[test]
    fn the_same_trouble_twice_is_one_trouble() {
        use crate::present::Mood;
        let tmp = tempfile::tempdir().unwrap();
        let mut r = Resident::new(tmp.path().join("sessions"));
        r.note(
            Mood::Look,
            "open:report",
            "report.slpc - did not open".into(),
        );
        r.note(
            Mood::Look,
            "open:report",
            "report.slpc - did not open".into(),
        );
        r.note(Mood::Look, "open:report", "report.slpc - still not".into());
        assert_eq!(r.troubles().len(), 1, "{:?}", r.troubles());
        assert_eq!(r.troubles()[0].summary, "report.slpc - still not");
    }

    #[test]
    fn a_container_that_will_not_open_leaves_something_behind() {
        // The case the standing list exists for: a double-click that produces
        // no document and no window. The notification saying so is a moment,
        // and a moment that was missed is indistinguishable from the tool being
        // broken.
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let mut r = Resident::new(tmp.path().join("sessions"));
        let nowhere = tmp.path().join("not-a-container.slpc");
        fs::write(&nowhere, b"this is not a container").unwrap();

        let _ = err(r.handle(opening(nowhere), &w.outside()));
        assert_eq!(r.mood(), crate::present::Mood::Look);
        let said = &r.troubles()[0].summary;
        assert!(
            said.starts_with("not-a-container.slpc"),
            "it names the file the person clicked, not the session: {said}"
        );
    }

    #[test]
    fn a_payload_that_is_a_program_is_the_one_thing_red_is_for() {
        // Concept 5.1's check, which fires close to never and means one thing
        // when it does. Nothing else in this file may reach `Danger`.
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let mut r = Resident::new(tmp.path().join("sessions"));
        let c = container(tmp.path(), "invoice.txt", b"MZ\x90\x00 not a document");

        let why = err(r.handle(opening(c), &w.outside()));
        assert!(why.contains("was not opened"), "{why}");
        assert!(r.is_idle(), "nothing opened, so nothing is being held");
        assert!(
            w.launcher.launched().is_empty(),
            "nothing was handed to the desktop"
        );
        assert!(
            session::scan(&tmp.path().join("sessions"))
                .unwrap_or_default()
                .is_empty(),
            "the refusal is before the session, so nothing reached the disk"
        );

        // Insisted on rather than reported. A notification can be missed, and
        // this is the one refusal where being missed matters: the person is
        // holding a file somebody sent them believing it is a document.
        let insisted = w.channel.insisted();
        assert_eq!(insisted.len(), 1, "{insisted:?}");
        assert!(insisted[0].summary.contains("invoice.txt"), "{insisted:?}");

        assert_eq!(r.mood(), crate::present::Mood::Danger);
        let said = &r.troubles()[0].summary;
        assert!(
            said.contains("invoice.txt") && said.contains("Windows executable"),
            "{said}"
        );
    }

    #[test]
    fn a_trouble_stays_until_it_is_put_down() {
        // The property that makes the colour worth reading: it is not a banner
        // that expires while somebody is looking the other way. Turning the
        // loop over changes nothing about it.
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let mut r = Resident::new(tmp.path().join("sessions"));
        let c = container(tmp.path(), "invoice.txt", b"MZ\x90\x00 not a document");
        let _ = err(r.handle(opening(c), &w.outside()));

        for _ in 0..5 {
            r.turn(&w.outside());
        }
        assert_eq!(r.mood(), crate::present::Mood::Danger);

        let id = r.troubles()[0].id.clone();
        r.dismiss(&id);
        assert!(r.troubles().is_empty());
        assert_eq!(r.mood(), crate::present::Mood::Settled);
    }

    #[test]
    fn a_question_waiting_colours_the_icon_and_answering_clears_it() {
        // Read off the sessions rather than remembered, so there is nothing to
        // dismiss and nothing that can disagree with what is on disk.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let w = World::new();
        let c = container(tmp.path(), "report.txt", b"a report\n");

        a_crashed_session(&root, &c, "report.txt", b"an edit nobody wrote back\n");

        let mut r = Resident::new(&root);
        assert_eq!(r.mood(), crate::present::Mood::Settled);
        // Opening the same container is what raises concept 6.3's question.
        let _ = r.handle(announcing(c), &w.loud());
        assert_eq!(
            r.mood(),
            crate::present::Mood::Look,
            "a decision waiting is worth a look and nothing more"
        );
        assert!(
            r.troubles().is_empty(),
            "and it is not a trouble: it is on the sessions, so answering ends it"
        );

        let about = w.channel.questions()[0].about.clone();
        w.channel.answer(&about, Choice::Discard);
        r.turn(&w.outside());
        assert_eq!(r.mood(), crate::present::Mood::Settled);
    }
}
