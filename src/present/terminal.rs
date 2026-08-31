//! The floor beneath the notifications: lines on the terminal.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 9 keeps the command line always available, and this is what the
//! instance narrates through when there is nothing better — no notification
//! service, no session bus, or a build for a platform whose arm is not written
//! yet.
//!
//! **It writes to the error stream, and the reason is the first invocation.**
//! Where nobody is holding the front door, the `open` that starts the instance
//! is also its own client, so one process is both narrating and answering a
//! request. The answer to the request goes to the output stream and belongs to
//! whoever ran the command; the narration that follows it belongs to the
//! session and continues for as long as the session lives. Separating them
//! keeps a shell pipeline reading the first without collecting the second.

use std::io::Write as _;

use super::{Answer, Channel, Choice, Question, Report, Weight};

/// Reports and questions as lines.
#[derive(Debug, Default, Clone, Copy)]
pub struct Terminal;

impl Channel for Terminal {
    fn report(&self, report: &Report) {
        let mut err = std::io::stderr().lock();
        // Concept 9's weights have no terminal equivalent and none is invented:
        // a bell or a colour would be this tool deciding it knows more about
        // somebody's terminal than their terminal does. The weight is carried
        // for the channels that can act on it.
        let _ = writeln!(err, "{}", report.summary);
        for line in &report.detail {
            let _ = writeln!(err, "  {line}");
        }
        if report.weight == Weight::Interrupt {
            let _ = err.flush();
        }
    }

    fn ask(&self, question: &Question) {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "{}", question.summary);
        for line in &question.detail {
            let _ = writeln!(err, "  {line}");
        }
        for choice in &question.choices {
            // Reveal has no verb to name, and inventing one to fill out the
            // list would be adding a command to the interface so that a
            // rendering could be symmetrical. The question's own detail names
            // the directory, which is what reveal-the-folder is for.
            if let Some(flag) = flag_for(*choice) {
                let _ = writeln!(
                    err,
                    "  slipcase-open recover {} {flag}",
                    question.about.as_str()
                );
            }
        }
        let _ = err.flush();
    }

    fn withdraw(&self, _about: &str) {
        // A line already read cannot be taken back.
    }

    fn answers(&self) -> Vec<Answer> {
        // A terminal has no way to deliver an answer to a question asked
        // minutes ago. It is answered by running the verb the question named,
        // which reaches the instance through the front door as a request rather
        // than arriving here as an answer.
        Vec::new()
    }
}

/// The `recover` flag that performs a choice, where one does.
fn flag_for(choice: Choice) -> Option<&'static str> {
    match choice {
        Choice::WriteBack => Some("--write-back"),
        Choice::Discard => Some("--discard"),
        Choice::Reveal => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{flag_for, Terminal};
    use crate::present::{Answer, Channel, Choice};

    #[test]
    fn every_choice_is_either_a_verb_or_deliberately_not_one() {
        // The point of the assertion is that adding a choice makes somebody
        // decide, rather than letting it fall silently off the terminal.
        assert_eq!(flag_for(Choice::WriteBack), Some("--write-back"));
        assert_eq!(flag_for(Choice::Discard), Some("--discard"));
        assert_eq!(flag_for(Choice::Reveal), None);
    }

    #[test]
    fn a_terminal_never_answers() {
        assert_eq!(Terminal.answers(), Vec::<Answer>::new());
    }
}
