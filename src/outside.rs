//! The three things the engine works against that are not its own state.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Where policy is read from, what opens a payload, and how the person is
//! spoken to. They travel together because they are chosen together — once, by
//! `main`, from what the machine turns out to be — and because the engine
//! passes all three down the same path: concept 10 puts the policy decision
//! immediately before the launch, and concept 5.1's warning is said about the
//! same payload in the same breath.
//!
//! Held as trait objects rather than as type parameters. The alternative
//! threads three generics through every signature from the front door down to
//! the session, so that a program which picks its implementations at startup can
//! pretend it knew them at compile time.

use crate::platform::Launcher;
use crate::policy::{self, Notify};
use crate::present::{Channel, Report, Weight};

/// What the engine has been given to work with.
#[derive(Clone, Copy)]
pub struct Outside<'a> {
    /// Concept 10's layers, in whatever form this platform keeps them.
    pub policy: &'a dyn policy::Source,
    /// Concept 5 step 7, which hands the payload to the desktop.
    pub launcher: &'a dyn Launcher,
    /// Concept 9, which narrates and asks.
    pub channel: &'a dyn Channel,
    /// How much to say without being asked, resolved once when the instance
    /// started. See [`Notify`].
    pub notify: Notify,
}

impl<'a> Outside<'a> {
    /// Gather the three, saying as much as [`Notify`]'s default allows.
    #[must_use]
    pub fn new(
        policy: &'a dyn policy::Source,
        launcher: &'a dyn Launcher,
        channel: &'a dyn Channel,
    ) -> Self {
        Self {
            policy,
            launcher,
            channel,
            notify: Notify::default(),
        }
    }

    /// The same, at the volume a resolved policy asked for.
    #[must_use]
    pub fn saying(mut self, notify: Notify) -> Self {
        self.notify = notify;
        self
    }

    /// Report, unless the threshold says this one is not worth an interruption.
    ///
    /// **Every routine report goes through here and no question does.** A
    /// question is [`Channel::ask`], which this cannot reach, so no setting can
    /// silence a session into stranding its payload.
    pub fn report(&self, report: &Report) {
        if report.weight != Weight::Routine || self.notify == Notify::Everything {
            self.channel.report(report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Outside;
    use crate::platform::testing::Recording as Launching;
    use crate::policy::{Notify, Origin, Read, Source};
    use crate::present::testing::Recording as Told;
    use crate::present::{Choice, Question, Report};

    struct Default_;
    impl Source for Default_ {
        fn layer(&self, _o: Origin) -> Read {
            Ok(None)
        }
    }

    fn three() -> [Report; 3] {
        [
            Report::routine("a save landed"),
            Report::ordinary("you asked and here is the answer"),
            Report::interrupt("this one is a warning"),
        ]
    }

    #[test]
    fn the_default_drops_what_happened_on_its_own_and_nothing_else() {
        let launcher = Launching::default();
        let told = Told::default();
        let outside = Outside::new(&Default_, &launcher, &told);
        assert_eq!(outside.notify, Notify::Important);
        for r in &three() {
            outside.report(r);
        }
        let said = told.said();
        assert!(!said.contains("a save landed"), "{said}");
        assert!(said.contains("you asked"), "{said}");
        assert!(said.contains("a warning"), "{said}");
    }

    #[test]
    fn saying_everything_lets_the_routine_ones_through() {
        let launcher = Launching::default();
        let told = Told::default();
        let outside = Outside::new(&Default_, &launcher, &told).saying(Notify::Everything);
        for r in &three() {
            outside.report(r);
        }
        assert_eq!(told.reports().len(), 3);
    }

    #[test]
    fn no_setting_can_silence_a_question() {
        // The property the whole design rests on. A question is `ask`, which
        // `Outside::report` cannot reach, so a session waiting on a decision
        // cannot be quietened into stranding its payload — and that is
        // structural rather than a rule somebody has to keep in mind.
        let launcher = Launching::default();
        let told = Told::default();
        let outside = Outside::new(&Default_, &launcher, &told);
        outside.channel.ask(&Question {
            about: "abc-0".into(),
            summary: "report.pdf was left behind.".into(),
            detail: Vec::new(),
            choices: vec![Choice::WriteBack, Choice::Discard],
        });
        assert_eq!(told.questions().len(), 1);
    }
}
