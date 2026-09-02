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
    }

    impl Recording {
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
    }
}
