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

/// Not yet, and deliberately not approximated.
///
/// Concept 5 step 6 wants the platform's *attachment-aware* execution path:
/// `ShellExecuteEx` with `IAttachmentExecute`, which is what consults Mark of
/// the Web and shows the warning a downloaded file has earned. `cmd /c start`
/// is not that. It is a different launcher with different quoting rules and no
/// attachment handling at all, and shipping it would mean the one platform
/// where the trust zone is enforced is the one where this tool quietly bypasses
/// it. PLAN.md Phase 4.
#[cfg(target_os = "windows")]
impl Launcher for Host {
    fn launch(&self, _payload: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "launching is not implemented on Windows yet: it needs ShellExecuteEx with \
             IAttachmentExecute, and an approximation would bypass Mark of the Web",
        ))
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
