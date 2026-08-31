//! Concept 9's channel on Linux: `org.freedesktop.Notifications`.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! A notification carrying actions is what the design needs, and this interface
//! has carried an actions parameter for as long as it has existed. GNOME Shell
//! renders them as buttons and keeps the notification in its message list,
//! which is concept 6.2's *somewhere to look when they expect an edit to have
//! landed* without a tray — and Linux has no dependable tray, which is the
//! reason concept 9 inverted its own layering.
//!
//! **A D-Bus client rather than a shell out to `notify-send`.** That tool can
//! pass an action and print back the key that was pressed, so the objection is
//! not that it cannot hear the answer. It is the shape: one blocked process per
//! outstanding notification, no way to withdraw one, and no guarantee it is
//! installed. The bus interface is what a desktop provides.
//!
//! ## How an answer gets back
//!
//! `Notify` returns an identifier, and the service emits `ActionInvoked` with
//! that identifier and the key of the button. So the identifier is recorded
//! against the session the question was about, one thread does nothing but
//! read signals, and the resident loop collects what that thread has put down
//! (concept 8, which will not have the loop waiting on anything but its own
//! tick).
//!
//! ## Where a service has no actions
//!
//! The capability is optional and a few implementations do without it. A
//! question shown by such a service would appear with no buttons and no way to
//! answer, so it is asked as a statement instead, with the two commands that
//! reach the same decision in the body. That is the same fallback the terminal
//! makes, arriving by a different road.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::Value;

use super::{Answer, Channel, Choice, Question, Report, Weight};

const SERVICE: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";

/// The desktop entry the service attributes a notification to, which is how a
/// shell finds the name and icon to draw beside it. The installed file's
/// basename, from `packaging/linux/slipcase-open.desktop`; a hint naming an
/// entry that is not there is ignored rather than refused, so a mismatch here
/// shows up as an unattributed notification and nothing else.
const DESKTOP_ENTRY: &str = "slipcase-open";

/// Until an icon of this project's own ships with the package. A stock name
/// rather than nothing, because a missing icon renders as a blank square and
/// reads as a broken notification.
const ICON: &str = "document-open";

/// What each outstanding notification is about, so a signal naming a number can
/// be turned into an answer naming a session.
type Outstanding = Arc<Mutex<HashMap<u32, String>>>;

/// Notifications, and the answers that come back from them.
pub struct Desktop {
    connection: Connection,
    outstanding: Outstanding,
    answers: Arc<Mutex<Vec<Answer>>>,
    /// Whether the service will render buttons.
    actions: bool,
    /// Whether the service reads the body as markup, in which case what goes
    /// into it has to be escaped.
    markup: bool,
}

impl Desktop {
    /// Reach the session bus and the notification service.
    ///
    /// # Errors
    ///
    /// Where there is no session bus, or nothing implements the interface —
    /// both of which are ordinary rather than exceptional. A session over SSH
    /// has neither, and the answer is concept 9's floor rather than a failure.
    pub fn connect() -> Result<Self, zbus::Error> {
        let connection = Connection::session()?;
        let proxy = notifications(&connection)?;
        // Asked once rather than per notification. A round trip before every
        // message would put a blocking call on the resident loop's own thread,
        // and a service that gained or lost actions mid-session is not a case
        // the specification contemplates.
        let capabilities: Vec<String> = proxy.call("GetCapabilities", &())?;
        let actions = capabilities.iter().any(|c| c == "actions");
        let markup = capabilities.iter().any(|c| c == "body-markup");

        let desktop = Self {
            connection,
            outstanding: Outstanding::default(),
            answers: Arc::default(),
            actions,
            markup,
        };
        desktop.listen()?;
        Ok(desktop)
    }

    /// Start the thread that turns signals into answers.
    fn listen(&self) -> Result<(), zbus::Error> {
        let connection = self.connection.clone();
        let outstanding = Arc::clone(&self.outstanding);
        let answers = Arc::clone(&self.answers);
        let proxy = notifications(&connection)?;
        std::thread::spawn(move || {
            let Ok(signals) = proxy.receive_all_signals() else {
                return;
            };
            for message in signals {
                let header = message.header();
                match header.member().map(zbus::names::MemberName::as_str) {
                    Some("ActionInvoked") => {
                        let Ok((id, key)) = message.body().deserialize::<(u32, String)>() else {
                            continue;
                        };
                        // The record stays: the engine withdraws a question
                        // when it has acted on it, and `NotificationClosed`
                        // takes care of the rest.
                        let Some(about) = outstanding.lock().map_or(None, |o| o.get(&id).cloned())
                        else {
                            continue;
                        };
                        // A key this build does not know is a button somebody
                        // else's service invented, or one from a version of
                        // this tool that has since been replaced.
                        if let Some(choice) = Choice::from_key(&key) {
                            if let Ok(mut answers) = answers.lock() {
                                answers.push(Answer { about, choice });
                            }
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _reason)) = message.body().deserialize::<(u32, u32)>() {
                            if let Ok(mut outstanding) = outstanding.lock() {
                                outstanding.remove(&id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(())
    }

    /// Send one, and answer with the identifier the service gave it.
    fn notify(
        &self,
        summary: &str,
        body: &str,
        actions: &[&str],
        weight: Weight,
    ) -> Result<u32, zbus::Error> {
        let proxy = notifications(&self.connection)?;
        // A payload name is attacker-controlled — SPEC 2.3 constrains it only
        // to being a plain filename — and a service advertising `body-markup`
        // parses the body as Pango. A name carrying `<b>` would then style the
        // sentence somebody is being asked to judge, and one carrying a stray
        // `&` would break the parse and take the whole body with it. SPEC 3
        // already makes a refusal message a display path; this is the same
        // rule at the channel that has a markup parser behind it.
        let body = if self.markup {
            escape(body)
        } else {
            body.to_string()
        };
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(DESKTOP_ENTRY));
        hints.insert(
            "urgency",
            Value::from(match weight {
                Weight::Ordinary => 1u8,
                Weight::Interrupt => 2u8,
            }),
        );
        // Never expiring for anything that has to be come back to, which is
        // concept 9's *persists somewhere the user can return to*: a question,
        // and a warning that concept 5.1 says earns an interrupt. Everything
        // else takes the service's own timeout, because a write-back notice
        // that had to be dismissed would make the ordinary case the noisy one.
        let timeout: i32 = if actions.is_empty() && weight == Weight::Ordinary {
            -1
        } else {
            0
        };
        proxy.call(
            "Notify",
            &(
                "slipcase-open",
                0u32,
                ICON,
                summary,
                body.as_str(),
                actions,
                hints,
                timeout,
            ),
        )
    }

    /// The notifications this instance has open about `about`.
    fn identifiers_for(&self, about: &str) -> Vec<u32> {
        self.outstanding.lock().map_or_else(
            |_| Vec::new(),
            |o| {
                o.iter()
                    .filter(|(_, held)| held.as_str() == about)
                    .map(|(id, _)| *id)
                    .collect()
            },
        )
    }
}

impl Channel for Desktop {
    fn report(&self, report: &Report) {
        // A failed notification is not worth an error path of its own. The
        // service may have gone away mid-session, and the record on disk plus
        // `slipcase-open sessions` is the answer to that, as it is to a crash.
        let _ = self.notify(
            &report.summary,
            &report.detail.join("\n"),
            &[],
            report.weight,
        );
    }

    fn ask(&self, question: &Question) {
        let mut body = question.detail.clone();
        let mut actions: Vec<&str> = Vec::new();
        if self.actions {
            for choice in &question.choices {
                actions.push(choice.key());
                actions.push(choice.label());
            }
        } else {
            body.push(String::new());
            body.push(format!(
                "slipcase-open recover {} --write-back",
                question.about
            ));
            body.push(format!(
                "slipcase-open recover {} --discard",
                question.about
            ));
        }
        // Asked at interrupt weight whether or not the buttons render, because
        // it is asked at all: nothing happens to the session until somebody
        // answers, and a question nobody sees is a session that sits there.
        if let Ok(id) = self.notify(
            &question.summary,
            &body.join("\n"),
            &actions,
            Weight::Interrupt,
        ) {
            if let Ok(mut outstanding) = self.outstanding.lock() {
                outstanding.insert(id, question.about.clone());
            }
        }
    }

    fn withdraw(&self, about: &str) {
        let Ok(proxy) = notifications(&self.connection) else {
            return;
        };
        for id in self.identifiers_for(about) {
            let _: Result<(), _> = proxy.call("CloseNotification", &(id,));
            if let Ok(mut outstanding) = self.outstanding.lock() {
                outstanding.remove(&id);
            }
        }
    }

    fn answers(&self) -> Vec<Answer> {
        self.answers
            .lock()
            .map_or_else(|_| Vec::new(), |mut a| std::mem::take(&mut *a))
    }
}

/// The three characters Pango reads as markup.
///
/// The summary is not markup by the specification and is left alone; the body
/// is, wherever the service says so.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// A proxy onto the notification service.
fn notifications(connection: &Connection) -> Result<Proxy<'static>, zbus::Error> {
    Proxy::new(connection, SERVICE, OBJECT, SERVICE)
}

#[cfg(test)]
mod tests {
    use super::{escape, Desktop};
    use crate::present::{Channel, Choice, Question, Report};

    #[test]
    fn a_payload_name_cannot_put_markup_in_the_body() {
        // `payload.file` is attacker-controlled and the body is parsed as Pango
        // wherever the service says `body-markup`. A name carrying a tag would
        // otherwise style the sentence somebody is being asked to judge, and a
        // bare ampersand would break the parse and lose the body with it.
        assert_eq!(
            escape("<b>invoice</b> & <i>co</i>.pdf"),
            "&lt;b&gt;invoice&lt;/b&gt; &amp; &lt;i&gt;co&lt;/i&gt;.pdf"
        );
        // And an ordinary name is left exactly as it was.
        assert_eq!(escape("quarterly report.pdf"), "quarterly report.pdf");
    }

    /// Talks to the real session bus, so it is not part of the suite.
    ///
    /// What it checks is the one thing no unit test can: that the arguments
    /// this builds match the signature `Notify` actually takes. Everything
    /// else here is a `HashMap` and a `Vec`; a wrong type in that tuple is a
    /// runtime error on a machine with a desktop and nothing at all on the
    /// build server.
    ///
    /// `cargo test --lib -- --ignored notifications` and watch the screen.
    #[test]
    #[ignore = "needs a session bus and a notification service"]
    fn notifications_reach_a_real_service() {
        let desktop = Desktop::connect().expect("no notification service");
        desktop.report(&Report::ordinary("slipcase-open: a report").and("with a line under it"));
        desktop.ask(&Question {
            about: "test-0".into(),
            summary: "slipcase-open: a question".into(),
            detail: vec!["It should carry three buttons.".into()],
            choices: vec![Choice::WriteBack, Choice::Discard, Choice::Reveal],
        });
        assert!(
            !desktop.identifiers_for("test-0").is_empty(),
            "the question was not given an identifier"
        );
        desktop.withdraw("test-0");
        assert!(desktop.identifiers_for("test-0").is_empty());
    }

    /// Concept 9 rests on a notification that persists somewhere the person can
    /// return to: a recovery question is worth nothing if it goes while they
    /// are finishing a sentence. That is the desktop's behaviour rather than
    /// this code's, so it is put on a real one and looked at.
    ///
    /// **Measured on GNOME Shell 48.7: a notification does not outlive the
    /// process that sent it.** While the sender is alive the question is on
    /// screen and in the message list, with its buttons; once the sender's bus
    /// connection goes, the shell removes it. So concept 9's *persists
    /// somewhere the user can return to* holds only for as long as the instance
    /// does, and concept 8's fallback is the record on disk rather than the
    /// notification. Amended in both places.
    ///
    /// Holds for a minute so there is time to look, and leaves the question
    /// standing. Dismiss it by hand.
    ///
    /// `cargo test --lib -- --ignored persists`
    #[test]
    #[ignore = "needs a session bus, and leaves a notification behind"]
    fn a_question_persists_in_the_message_list() {
        let desktop = Desktop::connect().expect("no notification service");
        desktop.ask(&Question {
            about: "persistence-0".into(),
            summary: "slipcase-open: does this stay?".into(),
            detail: vec![
                "It should still be in the message list a minute from now.".into(),
                "Dismiss it by hand when you have looked.".into(),
            ],
            choices: vec![Choice::WriteBack, Choice::Discard, Choice::Reveal],
        });
        let held = desktop.identifiers_for("persistence-0");
        assert_eq!(held.len(), 1);
        println!("notification {} sent. Holding for 60 seconds.", held[0]);
        // Alive on purpose. GNOME Shell watches the sending bus name, and what
        // this is asking is whether a notification outlives the process that
        // sent it — which is what concept 9 assumes and concept 8's exit rule
        // depends on.
        std::thread::sleep(std::time::Duration::from_secs(60));
        println!("exiting now; watch whether it goes with me");
    }
}
