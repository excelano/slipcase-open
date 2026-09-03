//! What the tool says to the person using it, and how it asks.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 9. A notification carrying actions is the baseline and exists on all
//! three platforms; the command line is the floor beneath it. This is the trait
//! both sit behind, so the engine can narrate and ask without knowing which of
//! the two is listening, and so the tray — an enhancement on the two platforms
//! that have one — joins later without the engine noticing.
//!
//! **An answer arrives long after the question, or never at all.** A
//! notification with buttons sits in the message list until somebody comes back
//! to it, so [`Channel::ask`] hands the question over and returns, and answers
//! are collected on the loop's own turn by [`Channel::answers`]. Waiting on one
//! would starve the watchers, which are the reason the process is resident
//! (concept 8).
//!
//! **This is the boundary concept 8 draws rather than a breach of it.** The
//! engine holds the session model and calls the trait; the implementations sit
//! beside it and are chosen by `main`. Nothing in `flow`, `writeback` or
//! `recover` knows a notification exists.

use std::fmt;

pub mod terminal;

#[cfg(target_os = "linux")]
pub mod freedesktop;

#[cfg(windows)]
pub mod toast;

#[cfg(windows)]
pub mod tray;

/// One session, as the standing list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// What `sessions` prints and `close` takes.
    pub id: String,
    /// The payload and what has become of it, without the id in front. The
    /// command line puts the id back; a menu has no room for one.
    pub label: String,
    /// The payload's name alone, for a menu item that is an action rather than
    /// a line of a report: *Close report.pdf* says what pressing it does, where
    /// the whole label repeats what the tooltip already counted.
    pub payload: String,
    /// Live: open, or closed and waiting for the application to finish. What
    /// the standing list is reassuring somebody about.
    pub live: bool,
    /// A decision only a person can make — a session left behind with an edit
    /// in it. Concept 6.3's three choices are what it needs.
    pub needs_a_person: bool,
    /// Saves written back so far, where the session is one that counts them.
    ///
    /// The one number that changes while somebody works, which is the whole
    /// reason a standing list is worth more than a notification: a toast says
    /// what happened once, and this says what is true now.
    pub write_backs: Option<u64>,
}

/// What the icon is saying, which for most people is the whole of this tool's
/// interface.
///
/// **One question, answered continuously: is my work safe.** Somebody who never
/// opens the menu and never reads a notification should still be able to glance
/// at the clock and know the answer, and a person who learns to ignore it
/// entirely is the success case rather than a failure.
///
/// Ordered, because the icon can only be one colour and what it shows is the
/// worst thing currently true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Mood {
    /// Watching, and everything that was saved has landed.
    #[default]
    Settled,
    /// A save is on its way back into its container. Seen for a moment and
    /// gone.
    ///
    /// **The least important of these and the one most worth having.** Pressing
    /// Save and seeing nothing happen anywhere is what makes a person doubt the
    /// tool is running at all, and it is the first thing that was noticed
    /// missing. It sits below the warnings because a flicker must never hide
    /// one.
    Working,
    /// Worth a look, with nothing at risk: a decision waiting, or a container
    /// that did not open because the desktop had nothing to open it with.
    Look,
    /// Work that is not in its container — a write-back that failed, or a
    /// container that moved out from under a session. The one thing this tool
    /// promises is that a save reaches the container, and this is that promise
    /// outstanding.
    AtRisk,
    /// Danger, and nothing else may use it or it stops meaning anything. Today
    /// that is one thing: a payload that is a program wearing a document's
    /// name.
    Danger,
}

/// Something the icon has taken on a colour for, in words.
///
/// The icon says *something is wrong*; the menu says *what*, in one line, about
/// a file the person recognises. Held rather than worked out from the sessions,
/// because most of these are moments — a container that would not open leaves
/// no session to re-read the trouble from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trouble {
    /// What it is called when it is put down.
    pub id: String,
    /// How much colour it is worth.
    pub mood: Mood,
    /// One line, naming the file.
    pub summary: String,
}

/// What somebody chose from the standing list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// Put down a trouble that has been read.
    ///
    /// **The one thing the standing list asks of anybody**, and it is there
    /// because a list of things that cannot be acted on is furniture. It is
    /// also the only way an icon that has gone red gets to go back to blue,
    /// which is what makes the colour a statement rather than a decoration.
    Dismiss(String),
    /// Leave, keeping every session recoverable, which is what interrupting
    /// the command line already does.
    Quit,
}

/// The standing list of what is open.
///
/// Concept 12 gives this a tray icon on Windows, a menu bar item on macOS and
/// the command line on Linux — which is why doing nothing is a complete
/// implementation and [`Nowhere`] is one. Concept 9 says the tray joins later
/// without the engine noticing, and not noticing is what this trait is for: the
/// loop hands it lines it was going to format anyway and asks what came back.
pub trait Standing {
    /// What is open, what is wrong, and the colour those add up to, whenever
    /// any of the three changes.
    fn show(&self, sessions: &[Listed], troubles: &[Trouble], mood: Mood);

    /// What has been chosen since this was last asked. Does not block.
    fn taken(&self) -> Vec<Chosen>;

    /// Whether this surface is itself a reason for the instance to stay.
    ///
    /// **A standing list stands until it is dismissed, and that supersedes
    /// concept 8's exit rule wherever there is one.** The rule ends an instance
    /// when no session, no lingering session and no unanswered question remain,
    /// which was right while the process had no face: there was nothing for it
    /// to be, so there was no reason for it to be. An icon changes that. It is
    /// where a warning lives, and a warning that appears in a process already
    /// on its way out has nowhere to go but a notification somebody may never
    /// see — which is the whole reason the colours exist.
    ///
    /// The cost, said plainly: open one container and a background process
    /// stays until it is asked to leave. That is the bargain every sync client
    /// makes, and the icon is what turns it from a surprise into a bargain.
    ///
    /// Where a terminal started this there is no icon, by [`crate::present`]'s
    /// own rule that the command line is the floor — so that invocation keeps
    /// concept 8 exactly as written, and `open` at a prompt still returns.
    fn holding(&self) -> bool {
        false
    }
}

/// No standing list, which is every platform without one and every test.
pub struct Nowhere;

impl Standing for Nowhere {
    fn show(&self, _sessions: &[Listed], _troubles: &[Trouble], _mood: Mood) {}
    fn taken(&self) -> Vec<Chosen> {
        Vec::new()
    }
}

/// How much of the person's attention something is worth.
///
/// **The axis is whether the person asked for it, not how loud it is.** That is
/// what [`crate::policy::Notify`] thresholds on, and it is the distinction a
/// setting called *how much the tool says without being asked* has to be able
/// to make. A confirmation of something somebody just clicked is not chatter
/// however routine it looks, and silencing it would mean pressing a button and
/// getting nothing back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Happens on its own, with nobody waiting on it. A write-back is this.
    /// The only weight a threshold may drop.
    Routine,
    /// The answer to something the person just did — a button pressed, a verb
    /// run — or a standing fact about the machine they should know once, like
    /// concept 10's *settings here are administered*.
    Ordinary,
    /// Worth being interrupted for. Concept 5.1's content check earns this and
    /// says why: it fires close to never, and when it fires it means the
    /// payload is an executable wearing a document's name.
    Interrupt,
}

/// Something said that needs no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// One line, and the only line a notification is certain to show.
    pub summary: String,
    /// What follows it, where there is room.
    pub detail: Vec<String>,
    pub weight: Weight,
}

impl Report {
    /// Something that happened on its own. Droppable.
    #[must_use]
    pub fn routine(summary: impl Into<String>) -> Self {
        Self {
            weight: Weight::Routine,
            ..Self::ordinary(summary)
        }
    }

    /// The answer to something somebody did, or a fact worth knowing once.
    #[must_use]
    pub fn ordinary(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            detail: Vec::new(),
            weight: Weight::Ordinary,
        }
    }

    /// A report worth interrupting for.
    #[must_use]
    pub fn interrupt(summary: impl Into<String>) -> Self {
        Self {
            weight: Weight::Interrupt,
            ..Self::ordinary(summary)
        }
    }

    /// Another line of detail.
    #[must_use]
    pub fn and(mut self, line: impl Into<String>) -> Self {
        self.detail.push(line.into());
        self
    }
}

/// What can be done about a session, as a button.
///
/// The set is concept 6.3's: write-back, discard, and reveal-the-folder. It is
/// deliberately small, because every member has to be expressible as a button
/// on all three platforms and as a verb on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Put the payload back into its container.
    WriteBack,
    /// Throw the payload away and remove the session.
    Discard,
    /// Show the payload directory, and decide later.
    Reveal,
}

impl Choice {
    /// The name this travels under, which a notification hands back rather than
    /// the label. Stable, because a running notification service may hold a
    /// question across an upgrade of this binary.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::WriteBack => "write-back",
            Self::Discard => "discard",
            Self::Reveal => "reveal",
        }
    }

    /// What the button says.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::WriteBack => "Write it back",
            Self::Discard => "Discard it",
            Self::Reveal => "Show me",
        }
    }

    /// The choice a notification service handed back, where it is one this
    /// build knows.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        [Self::WriteBack, Self::Discard, Self::Reveal]
            .into_iter()
            .find(|c| c.key() == key)
    }
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Something asked, which nothing acts on until it is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The session the answer applies to, spelled as `sessions` names it. An
    /// answer carries this back rather than a handle, because the question may
    /// outlive the process that asked it.
    pub about: String,
    pub summary: String,
    pub detail: Vec<String>,
    /// In the order they should be offered. The first is the default where a
    /// platform distinguishes one.
    pub choices: Vec<Choice>,
}

/// What somebody chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub about: String,
    pub choice: Choice,
}

/// How the tool speaks, and how it is answered.
///
/// Object-safe on purpose: `main` picks one implementation at startup from what
/// the machine turns out to have, and the engine holds it as a reference in
/// [`crate::outside::Outside`].
pub trait Channel {
    /// Say something that needs no answer.
    fn report(&self, report: &Report);

    /// Ask something, and return. The answer, if one comes, arrives through
    /// [`answers`](Self::answers).
    fn ask(&self, question: &Question);

    /// Take back a question that no longer applies, named by its `about`.
    ///
    /// The case is a session that resolved itself while its question was still
    /// sitting in somebody's message list — a lingering editor's last save
    /// landing (concept 8), or the same question answered at the command line.
    /// A button that would act on a session that has gone is answerable and
    /// pointless, and the answer would have to be a refusal.
    fn withdraw(&self, about: &str);

    /// The answers that have arrived since this was last asked.
    ///
    /// Does not block. The resident loop calls this each turn between pumping
    /// the watchers.
    fn answers(&self) -> Vec<Answer>;

    /// Say something in a way that cannot be missed, because something was
    /// refused and the person is owed an explanation for it.
    ///
    /// **Not a question, and it does not block the caller.** By the time this
    /// is called the refusal has already happened — nothing is waiting on an
    /// answer, and there is nothing to decide. What it buys over
    /// [`report`](Self::report) is that a notification can be missed and this
    /// one must not be: a double-click that produced no document and no
    /// message is the tool looking broken at the exact moment it worked.
    ///
    /// The default is [`report`](Self::report), which is right wherever the
    /// channel has nothing louder. Concept 12 gives Windows a native message
    /// box and that is where this becomes a dialog.
    fn insist(&self, report: &Report) {
        self.report(report);
    }
}

#[cfg(test)]
pub mod testing {
    //! A channel that remembers rather than shows, and can be answered by hand.

    use super::{Answer, Channel, Choice, Question, Report};
    use std::sync::Mutex;

    /// A channel that discards everything, for the tests of code that does not
    /// speak. `flow` is all of it: concept 8 keeps the narration out of the
    /// engine, so a flow test that had to build a recorder would be asserting
    /// the absence of something by carrying it around.
    pub struct Silent;

    impl Channel for Silent {
        fn report(&self, _report: &Report) {}
        fn ask(&self, _question: &Question) {}
        fn withdraw(&self, _about: &str) {}
        fn answers(&self) -> Vec<Answer> {
            Vec::new()
        }
    }

    /// Records what it was told and what it was asked, and hands back whatever
    /// answers a test has put in it.
    #[derive(Default)]
    pub struct Recording {
        reports: Mutex<Vec<Report>>,
        questions: Mutex<Vec<Question>>,
        withdrawn: Mutex<Vec<String>>,
        pending: Mutex<Vec<Answer>>,
        insisted: Mutex<Vec<Report>>,
    }

    impl Recording {
        /// Everything insisted on, in order. Not included in
        /// [`reports`](Self::reports).
        ///
        /// # Panics
        ///
        /// As [`reports`](Self::reports).
        #[must_use]
        pub fn insisted(&self) -> Vec<Report> {
            self.insisted.lock().unwrap().clone()
        }

        /// Everything reported, in order.
        ///
        /// # Panics
        ///
        /// If a previous caller panicked while holding the lock, which in a
        /// test means the test that did so has already failed.
        #[must_use]
        pub fn reports(&self) -> Vec<Report> {
            self.reports.lock().unwrap().clone()
        }

        /// Every summary and detail line reported, joined, for the tests that
        /// only care that something was said.
        ///
        /// # Panics
        ///
        /// As [`reports`](Self::reports).
        #[must_use]
        pub fn said(&self) -> String {
            self.reports
                .lock()
                .unwrap()
                .iter()
                .fold(String::new(), |mut all, r| {
                    all.push_str(&r.summary);
                    all.push('\n');
                    all.push_str(&r.detail.join("\n"));
                    all.push('\n');
                    all
                })
        }

        /// Everything asked, in order.
        ///
        /// # Panics
        ///
        /// As [`reports`](Self::reports).
        #[must_use]
        pub fn questions(&self) -> Vec<Question> {
            self.questions.lock().unwrap().clone()
        }

        /// The questions taken back, in order.
        ///
        /// # Panics
        ///
        /// As [`reports`](Self::reports).
        #[must_use]
        pub fn withdrawn(&self) -> Vec<String> {
            self.withdrawn.lock().unwrap().clone()
        }

        /// Answer as a person would, to be picked up on the next
        /// [`answers`](Channel::answers).
        ///
        /// # Panics
        ///
        /// As [`reports`](Self::reports).
        pub fn answer(&self, about: &str, choice: Choice) {
            self.pending.lock().unwrap().push(Answer {
                about: about.to_owned(),
                choice,
            });
        }
    }

    impl Channel for Recording {
        fn report(&self, report: &Report) {
            self.reports.lock().unwrap().push(report.clone());
        }

        fn ask(&self, question: &Question) {
            self.questions.lock().unwrap().push(question.clone());
        }

        fn withdraw(&self, about: &str) {
            self.withdrawn.lock().unwrap().push(about.to_owned());
        }

        fn answers(&self) -> Vec<Answer> {
            std::mem::take(&mut *self.pending.lock().unwrap())
        }

        fn insist(&self, report: &Report) {
            // Recorded apart from `report`, because what is being asserted is
            // that this one could not be missed rather than that it was said.
            // The default implementation forwards to `report`; this one does
            // not, so a test cannot pass by mistaking the two.
            self.insisted.lock().unwrap().push(report.clone());
        }
    }
}
