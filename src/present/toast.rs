//! Windows' half of concept 9: a toast, with the buttons a question needs.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! **This needs package identity, and that is why it can fail.** A toast is
//! addressed by an `AppUserModelID`, and a packaged application has one without
//! asking. Unpackaged, it would need a Start Menu shortcut carrying an
//! identity and `SetCurrentProcessExplicitAppUserModelID` agreeing with it —
//! which `slipcase-desktop` looked at and declined, because a shortcut and a
//! process declaring different identities is worse than neither declaring one.
//! Concept 15 saw this coming and took the Store partly for it.
//!
//! So [`Toast::connect`] failing is ordinary rather than exceptional: it is
//! what happens when the binary is run from a checkout instead of from its
//! package, and the answer is concept 9's floor. That is the same shape as the
//! Linux arm, where no session bus means the terminal takes over.
//!
//! **Multi-threaded apartment, and the reason is the resident loop.** A `WinRT`
//! event handler on a single-threaded apartment is delivered through the
//! thread's message queue, and `resident::run` has no message pump — it is
//! pumping watchers, which concept 8 makes the reason the process exists. In an
//! MTA the callback arrives on a pool thread instead, which is what lets an
//! answer be pushed into a queue the loop drains on its own turn. It is the
//! same arrangement the Linux arm reaches by putting the bus signals on a
//! thread of their own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use windows::core::{Interface as _, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier,
};

use super::{Answer, Channel, Choice, Question, Report, Weight};

/// Toasts, and the answers that come back from their buttons.
pub struct Toast {
    notifier: ToastNotifier,
    /// What is still on screen, so a question can be taken back. Keyed the way
    /// an [`Answer`] is, because that is what the engine withdraws by.
    outstanding: Mutex<HashMap<String, Vec<ToastNotification>>>,
    answers: Arc<Mutex<Vec<Answer>>>,
    /// The boxes raised and not yet closed, so that the process can be kept
    /// from ending underneath one. See [`Dialogs`].
    dialogs: Dialogs,
}

impl Toast {
    /// Reach the notification manager for this package.
    ///
    /// # Errors
    ///
    /// Where the process has no package identity, which is every run that is
    /// not from the installed package. Concept 9's floor is the answer.
    pub fn connect() -> windows::core::Result<Self> {
        apartment();
        // The call that needs the identity. Unpackaged it refuses, and that
        // refusal is this function's whole error case.
        let notifier = ToastNotificationManager::CreateToastNotifier()?;
        Ok(Self {
            notifier,
            outstanding: Mutex::default(),
            answers: Arc::default(),
            dialogs: Dialogs::default(),
        })
    }

    /// Show one toast, and where it carries buttons, remember it by `about` so
    /// that it can be taken back.
    fn show(&self, xml: &str, about: Option<&str>) -> windows::core::Result<()> {
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(xml))?;
        let notification = ToastNotification::CreateToastNotification(&document)?;

        if let Some(about) = about {
            let answers = Arc::clone(&self.answers);
            let about = about.to_owned();
            notification.Activated(&TypedEventHandler::new(
                move |_sender: windows::core::Ref<'_, ToastNotification>,
                      args: windows::core::Ref<'_, windows::core::IInspectable>| {
                    // The argument is the choice's key rather than its label,
                    // for the reason `Choice::key` gives: a notification
                    // service may hold a question across an upgrade of this
                    // binary, so what comes back has to be stable.
                    if let Some(pressed) = args
                        .as_ref()
                        .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
                        .and_then(|a| a.Arguments().ok())
                        .and_then(|k| Choice::from_key(&k.to_string()))
                    {
                        if let Ok(mut answers) = answers.lock() {
                            answers.push(Answer {
                                about: about.clone(),
                                choice: pressed,
                            });
                        }
                    }
                    Ok(())
                },
            ))?;
        }

        self.notifier.Show(&notification)?;
        if let Some(about) = about {
            if let Ok(mut outstanding) = self.outstanding.lock() {
                outstanding
                    .entry(about.to_owned())
                    .or_default()
                    .push(notification);
            }
        }
        Ok(())
    }
}

impl Channel for Toast {
    fn report(&self, report: &Report) {
        // A toast that will not show is not worth an error path of its own, for
        // the reason the Linux arm gives: the record on disk and
        // `slipcase-open sessions` are the answer to a channel that has gone,
        // as they are to a crash.
        let _ = self.show(
            &body(&report.summary, &report.detail, &[], report.weight),
            None,
        );
    }

    fn ask(&self, question: &Question) {
        // Interrupt weight whether or not anything renders it, because it is a
        // question: nothing happens to the session until somebody answers, and
        // one nobody sees is a session that sits there.
        let xml = body(
            &question.summary,
            &question.detail,
            &question.choices,
            Weight::Interrupt,
        );
        let _ = self.show(&xml, Some(&question.about));
    }

    fn withdraw(&self, about: &str) {
        let Ok(mut outstanding) = self.outstanding.lock() else {
            return;
        };
        for notification in outstanding.remove(about).unwrap_or_default() {
            let _ = self.notifier.Hide(&notification);
        }
    }

    fn answers(&self) -> Vec<Answer> {
        self.answers
            .lock()
            .map_or_else(|_| Vec::new(), |mut a| std::mem::take(&mut *a))
    }

    fn insist(&self, report: &Report) {
        // The toast as well, so that what was refused is still in the
        // notification centre an hour later. The box is the part that cannot be
        // missed; the toast is the part that can be gone back to.
        self.report(report);
        self.dialogs.raise(message_box(report));
    }

    fn stay_until_seen(&self) {
        self.dialogs.settle();
    }
}

/// The boxes this channel has raised, and the wait that keeps the process
/// alive while one of them is up.
///
/// **A thread and a join, rather than either on its own.** The thread is
/// [`message_box`]'s reason, which the resident loop needs; the join is
/// [`Channel::stay_until_seen`]'s, which the invocation that refused and held
/// nothing needs. Neither is optional and they are in tension, so the box goes
/// on a thread that nobody waits for *until the process is leaving*.
///
/// A dialog whose thread has already finished is a box somebody closed. The
/// list is drained by [`settle`](Self::settle) and not pruned as they go: there
/// is at most one of these in any run this tool has, and a reaper for a vector
/// that reaches a length of one would be the more complicated thing.
#[derive(Default)]
struct Dialogs(Mutex<Vec<std::thread::JoinHandle<()>>>);

impl Dialogs {
    /// Put one up on a thread of its own and remember it.
    ///
    /// A name, so that a thread panicking here is attributable. Failure to
    /// spawn is the case where the toast is the whole of what was said, which
    /// is the same outcome as this platform having no dialog at all — and
    /// nothing is remembered, so nothing is waited for.
    fn raise(&self, show: impl FnOnce() + Send + 'static) {
        let spawned = std::thread::Builder::new()
            .name("slipcase-open dialog".to_owned())
            .spawn(show);
        if let (Ok(handle), Ok(mut held)) = (spawned, self.0.lock()) {
            held.push(handle);
        }
    }

    /// Wait for every box still up.
    ///
    /// A poisoned lock is treated as nothing to wait for. The alternative is a
    /// process that will not end because the thread bookkeeping went wrong,
    /// which is worse than a box that closes with its process.
    fn settle(&self) {
        let waiting = match self.0.lock() {
            Ok(mut held) => std::mem::take(&mut *held),
            Err(_) => return,
        };
        for handle in waiting {
            let _ = handle.join();
        }
    }
}

/// Concept 12's native dialog, which this is the first thing to need.
///
/// **Returns the showing rather than doing it**, because where it runs is
/// [`Dialogs`]'s decision and not this function's. The two callers want
/// opposite things — the resident loop must not block for as long as somebody
/// leaves a box up, and an invocation with nothing left to hold must not end
/// while one is up — and both are served by a thread that is joined only on the
/// way out.
///
/// `MB_SYSTEMMODAL` puts it in front of whatever is focused, which is the
/// window the person just double-clicked in. Without it the box can open behind
/// Explorer, which is the same defect the launcher had and the same fix.
fn message_box(report: &Report) -> impl FnOnce() + Send + 'static {
    use windows::core::HSTRING;

    let mut body = report.summary.clone();
    for line in &report.detail {
        body.push_str("\n\n");
        body.push_str(line);
    }
    // Built here and moved, so that nothing borrowed from `report` crosses on
    // to the thread.
    let text = HSTRING::from(body);
    let title = HSTRING::from("Slipcase Open");
    move || show_box(&text, &title)
}

#[allow(unsafe_code)]
fn show_box(text: &windows::core::HSTRING, title: &windows::core::HSTRING) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_SYSTEMMODAL,
    };

    // SAFETY: both strings are null-terminated and owned by the caller for the
    // life of the call, and the box has no parent window — this process has
    // none to give it.
    unsafe {
        MessageBoxW(
            None,
            windows::core::PCWSTR(text.as_ptr()),
            windows::core::PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_SYSTEMMODAL,
        );
    }
}

/// The apartment this thread speaks `WinRT` in.
///
/// Multi-threaded, for the reason in the module documentation. A thread that
/// already chose an apartment keeps it — `RPC_E_CHANGED_MODE` is somebody
/// else's decision and not this code's to undo — and the notifier will simply
/// refuse afterwards, which is the same failure as having no identity and takes
/// the same route out.
#[allow(unsafe_code)]
fn apartment() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    // SAFETY: the documented entry point, with no reserved parameter. Nothing
    // is uninitialized to match it: this is the process's own thread and the
    // apartment lasts as long as the instance does.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

/// One toast's XML.
///
/// `ToastGeneric` rather than one of the numbered legacy templates, because
/// those fix how many lines of text there are and a report's detail does not.
fn body(summary: &str, detail: &[String], choices: &[Choice], weight: Weight) -> String {
    let mut xml = String::from("<toast");
    // A question stays on screen until it is answered or taken back; a report
    // goes when the person stops looking at it. Concept 9's weights are about
    // whether somebody asked for it, and this is the nearest thing the platform
    // offers to that distinction.
    if weight == Weight::Interrupt {
        xml.push_str(" duration=\"long\" scenario=\"reminder\"");
    }
    xml.push_str("><visual><binding template=\"ToastGeneric\"><text>");
    xml.push_str(&escape(summary));
    xml.push_str("</text>");
    for line in detail {
        xml.push_str("<text>");
        xml.push_str(&escape(line));
        xml.push_str("</text>");
    }
    xml.push_str("</binding></visual>");
    if !choices.is_empty() {
        xml.push_str("<actions>");
        for choice in choices {
            xml.push_str("<action activationType=\"foreground\" content=\"");
            xml.push_str(&escape(choice.label()));
            xml.push_str("\" arguments=\"");
            xml.push_str(&escape(choice.key()));
            xml.push_str("\"/>");
        }
        xml.push_str("</actions>");
    }
    xml.push_str("</toast>");
    xml
}

/// The five characters XML will not take raw.
///
/// A payload's name reaches here, and concept 9 already learned this on the
/// other platform: a name carrying an ampersand or a quote would otherwise be
/// markup, and the toast would either render wrongly or not parse at all.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{body, escape, Dialogs};
    use crate::present::{Choice, Weight};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// The guarantee the lost refusal needed, on the type that carries it.
    ///
    /// Not a message box: a box needs somebody to close it, so a test that
    /// raised one would hang until a person did. What is being asserted is the
    /// thing that failed, which is not the drawing — it is that the wait
    /// returns *after* the work on the thread has finished rather than before.
    #[test]
    fn nothing_is_left_running_once_the_dialogs_have_settled() {
        let dialogs = Dialogs::default();
        let closed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&closed);
        dialogs.raise(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            flag.store(true, Ordering::SeqCst);
        });
        // The condition before the fix: the caller had returned by here.
        dialogs.settle();
        assert!(
            closed.load(Ordering::SeqCst),
            "settle returned while a box was still up, which is the defect"
        );
    }

    #[test]
    fn settling_with_no_dialog_up_returns_rather_than_waits() {
        // Every ordinary run. `stay_until_seen` is called on the way out of
        // invocations that never insisted on anything, and it must cost them
        // nothing.
        let dialogs = Dialogs::default();
        let started = std::time::Instant::now();
        dialogs.settle();
        dialogs.settle();
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_payload_name_with_markup_in_it_is_escaped() {
        // The case the Linux arm met first: a name is somebody else's text and
        // reaches the channel unaltered.
        let xml = body(
            "R&D <draft>.txt is open",
            &["It came from \"somewhere else\"".to_string()],
            &[],
            Weight::Routine,
        );
        assert!(xml.contains("R&amp;D &lt;draft&gt;.txt"), "{xml}");
        assert!(xml.contains("&quot;somewhere else&quot;"), "{xml}");
        assert!(!xml.contains("<draft>"), "{xml}");
    }

    #[test]
    fn a_question_carries_its_choices_as_keys_not_labels() {
        // What comes back from a button is the key, because a service may hold
        // a question across an upgrade of this binary.
        let xml = body(
            "A session was left behind",
            &[],
            &[Choice::WriteBack, Choice::Discard],
            Weight::Interrupt,
        );
        assert!(
            xml.contains(&format!("arguments=\"{}\"", Choice::WriteBack.key())),
            "{xml}"
        );
        assert!(
            xml.contains(&format!("content=\"{}\"", Choice::Discard.label())),
            "{xml}"
        );
        // And it stays up rather than sliding away unanswered.
        assert!(xml.contains("scenario=\"reminder\""), "{xml}");
    }

    #[test]
    fn a_report_has_no_actions_and_does_not_linger() {
        let xml = body("Written back", &[], &[], Weight::Routine);
        assert!(!xml.contains("<actions>"), "{xml}");
        assert!(!xml.contains("scenario="), "{xml}");
    }

    #[test]
    fn every_key_survives_the_round_trip_a_button_makes() {
        // The button carries `key` and the answer is rebuilt with `from_key`,
        // so a choice whose key did not round-trip would be a button that does
        // nothing.
        for choice in [Choice::WriteBack, Choice::Discard, Choice::Reveal] {
            assert_eq!(Choice::from_key(choice.key()), Some(choice));
            assert_eq!(escape(choice.key()), choice.key(), "a key needs escaping");
        }
    }
}
