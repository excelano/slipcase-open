//! The parts that differ by platform, behind one small trait.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 12 structures the differences as a trait with three implementations
//! rather than treating cross-platform as a yes-or-no decision. This is the
//! launch half of it; the policy sources and the presentation join it in
//! PLAN.md Phases 3 and 4.
//!
//! **Launching does not wait, and cannot.** Concept 6 starts from the
//! observation that handing a document to the desktop frequently returns at
//! once, because the file goes to an already-running instance of the target
//! application and there is no child process to wait on. Roughly half of real
//! applications behave that way, so a launch that waited would be right about
//! half the time and silently wrong about the rest. The session model exists
//! because of this, and the launcher's job stops at handing the file over.

use std::io;
use std::path::Path;

/// Handing a payload to whatever the desktop says opens it.
pub trait Launcher {
    /// Open `payload` with the platform's own handler.
    ///
    /// # Errors
    ///
    /// Where the platform's launcher cannot be run, or refuses.
    fn launch(&self, payload: &Path) -> io::Result<()>;
}

/// This machine.
pub struct Host;

/// `xdg-open`, which every desktop on this platform provides or is expected to.
///
/// There is no trust-zone marking to apply here and concept 12 says so out
/// loud: Linux keeps provenance as a note rather than as a gate, so
/// `slpc::provenance` records where a payload came from and nothing consults it.
/// That is the platform's shape rather than an omission in this code, and the
/// administrator documentation states it rather than leaving it to be
/// discovered.
#[cfg(target_os = "linux")]
impl Launcher for Host {
    fn launch(&self, payload: &Path) -> io::Result<()> {
        spawn_detached("xdg-open", payload)
    }
}

/// `open`, which consults `com.apple.quarantine` on the way, so the mark
/// carried onto the payload at extraction is what raises the warning.
#[cfg(target_os = "macos")]
impl Launcher for Host {
    fn launch(&self, payload: &Path) -> io::Result<()> {
        spawn_detached("open", payload)
    }
}

/// `ShellExecuteEx` with the default verb, which is what a double-click runs.
///
/// The zone check is the point of this arm, and it is switched on by *not*
/// switching it off — see [`shell`], which holds the measurements.
#[cfg(target_os = "windows")]
impl Launcher for Host {
    fn launch(&self, payload: &Path) -> io::Result<()> {
        shell::hand_over(payload)
    }
}

/// Give up this process's claim on the foreground before handing a request to
/// the instance that will act on it.
///
/// The seam is here rather than in `shell` because `main` is the caller and
/// `shell` is this module's own. What it is for is in `shell`.
#[cfg(target_os = "windows")]
pub fn hand_the_foreground_on() {
    shell::hand_the_foreground_on();
}

#[cfg(target_os = "windows")]
mod shell {
    //! Handing a payload to the shell, with Mark of the Web still consulted.
    //!
    //! ## `IAttachmentExecute` is not what reads the mark, and this was measured
    //!
    //! Concept 12 names `IAttachmentExecute` for both the launch and the trust
    //! zone, and the stub this replaced repeated it. It is the wrong instrument
    //! for both halves of what this tool does, which a probe settled on
    //! 2026-09-01 by asking `CheckPolicy` about ten files — marked and unmarked,
    //! across five extensions — and reading the raw `HRESULT` rather than the
    //! `Result<()>` the bindings collapse it into:
    //!
    //! | | no source | internet source | `file://` source |
    //! |---|---|---|---|
    //! | `.txt`, marked or not | `S_FALSE` | `S_FALSE` | `S_OK` |
    //! | `.pdf`, marked or not | `S_FALSE` | `S_FALSE` | `S_OK` |
    //! | `.exe`, marked or not | `0x800C000E` | `S_FALSE` | `S_OK` |
    //!
    //! The marked and unmarked rows are identical in every column. The answer
    //! moves with `SetSource` and with the extension, and never with the
    //! `Zone.Identifier` stream on the file. That is the interface working as
    //! intended rather than failing: it is for a client that has *received* an
    //! attachment and is deciding whether to save and run it, so the zone comes
    //! from the source it is told about. This tool arrives after that: the
    //! payload is on disk and already carries its mark, put there by
    //! `slpc::provenance` as `extract` placed it.
    //!
    //! ## What does read it, and the requirement that is therefore a negative
    //!
    //! `ShellExecuteEx` performs the zone check itself, and the evidence is that
    //! `SEE_MASK_NOZONECHECKS` exists to turn it off — a flag documented as
    //! bypassing "zone checking put into place by `IAttachmentExecute`", which
    //! would have nothing to bypass if the check were opt-in. `SEE_MASK_FLAG_NO_UI`
    //! is the other way to lose it, by suppressing the dialog it would raise.
    //!
    //! So the security-relevant instruction here is not a call to make but two
    //! flags never to set, which is a weaker thing to rely on than a call and is
    //! why [`MASK`] is a named constant with a test over it rather than a literal
    //! at the call site. A future flag added for an unrelated reason is exactly
    //! how this would be lost.
    //!
    //! **Not measured, and it needs a person at a desktop:** that the warning is
    //! actually shown for a marked payload. Nothing automated can watch a modal
    //! dialog, and the suite never reaches this function: the recording launcher
    //! in `platform::testing` is what every test launches through, on all three
    //! platforms.
    //! `packaging/README.md` is where a run of it belongs once there is one.
    //!
    //! ## Why a thread
    //!
    //! The dialogs this call may raise are modal and unbounded: the zone warning
    //! waits for an answer, and so does the *how do you want to open this*
    //! picker when nothing is registered. `resident::run` is what pumps the
    //! watchers, and concept 8 makes those the reason the process exists, so it
    //! is the one thread that may not wait on a person.
    //!
    //! What that costs is a failure after the handover going unreported, and the
    //! Unix arm already pays it: `spawn_detached` starts `xdg-open` and drops
    //! the result, so a missing handler is silent there too. The parity is
    //! deliberate rather than convenient. Reporting it would need concept 9's
    //! channel reachable from another thread, which it is not.

    use std::io;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL,
    };

    /// What is asked of `ShellExecuteEx`, and more to the point what is not.
    ///
    /// `SEE_MASK_NOASYNC` because this thread has no message loop and the call
    /// has to finish its association work before the thread ends. Neither
    /// `SEE_MASK_NOZONECHECKS` nor `SEE_MASK_FLAG_NO_UI` is here, and the module
    /// documentation says why that absence is the whole trust-zone story.
    pub(super) const MASK: u32 = SEE_MASK_NOASYNC;

    /// Give up this process's claim on the foreground, so that whoever acts
    /// next may take it.
    ///
    /// **Which process is holding the right is not the one doing the launching,
    /// and that is the whole of why this exists.** Concept 8 makes every
    /// invocation a client of a resident instance, so a double-click starts a
    /// process that hands its request over and exits. The shell activated
    /// *that* process, so it is the one Windows will let change the foreground
    /// — and the instance, which is what actually calls `ShellExecuteEx`, has
    /// been sitting in the background since the first container was opened and
    /// may not.
    ///
    /// Measured on 2026-09-02, and it is exactly this shape: the first
    /// double-click put the payload in front, because that invocation *was* the
    /// instance; every one after it opened the payload behind the window the
    /// person was looking at, because the instance by then was somebody else's
    /// old process.
    ///
    /// `ASFW_ANY` rather than naming the instance: the client would have to ask
    /// the pipe who is serving it, and the answer would still be wrong half the
    /// time — concept 6 says a payload frequently goes to an application that
    /// is already running, so the process which ends up in front is neither the
    /// client nor the instance.
    #[allow(unsafe_code)]
    pub(super) fn hand_the_foreground_on() {
        // SAFETY: gives away a right this process holds, takes no pointer, and
        // returns a bool that means nothing here — a client which never had the
        // right has none to lose.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }

    /// Hand `payload` to whatever the shell says opens it, and stop caring.
    ///
    /// # Errors
    ///
    /// Where the thread that does the handing cannot be started. Anything the
    /// shell itself refuses is not reported, for the reason in the module
    /// documentation.
    pub(super) fn hand_over(payload: &Path) -> io::Result<()> {
        // Widened here rather than in the thread, so that a path this process
        // can see is what gets sent rather than one resolved later.
        let path: Vec<u16> = payload
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        std::thread::Builder::new()
            .name("slipcase-open launch".to_owned())
            .spawn(move || {
                let _ = execute(&path);
            })
            .map(drop)
    }

    /// The call itself, on a thread of its own.
    #[allow(unsafe_code)]
    fn execute(path: &[u16]) -> io::Result<()> {
        let _apartment = Apartment::enter();

        // Hand our right to the foreground to whatever is about to be
        // started. Windows refuses a foreground change from a process that does
        // not have it, and the refusal is silent: the document opens *behind*
        // whatever the person was looking at. Measured on 2026-09-02 — a
        // container double-clicked in Explorer opened its payload behind the
        // Explorer window — and this is the documented way to pass the right
        // on, since this process was itself activated by that double-click.
        //
        // `ASFW_ANY` rather than a process id, because there is none to name:
        // concept 6 says half of real applications hand the file to an instance
        // that is already running, so the process that ends up with the
        // foreground is frequently not the one this call starts.
        //
        // SAFETY: a permission handed to the shell for the length of this
        // call, taking no pointer and returning a bool this code ignores by
        // design — a refusal leaves the window where it would have been.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };

        let mut how = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
            fMask: MASK,
            // Null is the default verb, which is what a double-click invokes.
            // Naming `open` would be narrower and would refuse the types whose
            // registration calls its default something else.
            lpVerb: PCWSTR::null(),
            lpFile: PCWSTR(path.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: `how` is a live, correctly sized structure this call fills in,
        // and `path` is null-terminated by `hand_over` and outlives the call
        // because `SEE_MASK_NOASYNC` means it does not return early.
        unsafe { ShellExecuteExW(&raw mut how) }.map_err(|_| io::Error::last_os_error())
    }

    /// COM for the length of one launch.
    ///
    /// `ShellExecuteEx` hands work to shell extensions and some of them require
    /// a single-threaded apartment, so this thread enters one. Whether it leaves
    /// again is not the same question: `S_FALSE` means somebody had already
    /// entered on this thread and still owes a matching exit, where
    /// `RPC_E_CHANGED_MODE` means they chose another model and this code must
    /// not undo it.
    struct Apartment(bool);

    impl Apartment {
        #[allow(unsafe_code)]
        fn enter() -> Self {
            // SAFETY: the documented entry point, with no reserved parameter.
            let how = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            Self(how.is_ok())
        }
    }

    impl Drop for Apartment {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: paired with the successful `CoInitializeEx` above, on
                // the same thread, which is what this call requires.
                unsafe { CoUninitialize() };
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::MASK;
        use windows::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOZONECHECKS};

        #[test]
        fn the_zone_check_is_never_opted_out_of() {
            // The one thing about this arm that can be tested without a person
            // watching a dialog, and the one worth a regression test: both of
            // these are ways to lose Mark of the Web, and neither is loud when
            // it happens. A flag added later for an unrelated reason is how the
            // trust zone would go quiet.
            assert_eq!(
                MASK & SEE_MASK_NOZONECHECKS,
                0,
                "the zone check has been opted out of"
            );
            assert_eq!(
                MASK & SEE_MASK_FLAG_NO_UI,
                0,
                "the warning the zone check raises has been suppressed"
            );
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl Launcher for Host {
    fn launch(&self, _payload: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no launcher for this platform",
        ))
    }
}

/// Start the launcher and stop caring about it.
///
/// The child's own streams go nowhere: `xdg-open` and `open` both write
/// diagnostics that belong to them rather than to this tool, and inheriting
/// them would put another program's complaints in the middle of a session
/// report.
///
/// Waited on only far enough to reap it. These launchers exit immediately
/// whether or not the document opened, so the exit status says the launcher ran
/// and nothing about the application — which is the same reason concept 6 will
/// not take process exit as a save signal.
#[cfg(unix)]
fn spawn_detached(program: &str, payload: &Path) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .arg(payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Not `wait`: `xdg-open` may block for as long as the application it
    // started, on the desktops where it execs rather than forks.
    let _ = child.try_wait();
    Ok(())
}

#[cfg(test)]
pub mod testing {
    //! A launcher that records rather than launches, so the flow can be tested
    //! without a desktop.

    use super::Launcher;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// Remembers what it was asked to open.
    #[derive(Default)]
    pub struct Recording {
        launched: Mutex<Vec<PathBuf>>,
        /// What to answer with, for the arm where the desktop refuses.
        refuse: bool,
    }

    impl Recording {
        /// A launcher that refuses everything, for the arm where the platform
        /// has no handler or will not run one.
        #[must_use]
        pub fn refusing() -> Self {
            Self {
                refuse: true,
                ..Self::default()
            }
        }

        /// Everything it was handed, in order.
        ///
        /// # Panics
        ///
        /// If a previous caller panicked while holding the lock, which in a
        /// test means the test that did so has already failed.
        #[must_use]
        pub fn launched(&self) -> Vec<PathBuf> {
            self.launched.lock().unwrap().clone()
        }
    }

    impl Launcher for Recording {
        fn launch(&self, payload: &Path) -> io::Result<()> {
            if self.refuse {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no handler"));
            }
            self.launched.lock().unwrap().push(payload.to_owned());
            Ok(())
        }
    }
}
