//! Where the front door is, and who is allowed through it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8: where no instance is running, the invocation starts one and hands
//! over; where one is, it hands over and exits.
//!
//! **The endpoint is restricted to its owner by the directory it sits in**,
//! which is the platform's own mechanism and, §8 says, a requirement rather
//! than a hardening measure. A socket's own permission bits are not portable —
//! some kernels ignore them on connect — so the guarantee is the traversal bit
//! on a directory nobody else can enter, set before the socket is bound.
//!
//! **It is runtime state and not saved state**, which is the opposite of the
//! choice §6.4 made for sessions. A stale socket from a crashed instance is
//! debris to be cleared, where a stale session directory holds somebody's edit;
//! so this goes in `$XDG_RUNTIME_DIR` where a platform offers one — cleared at
//! logout, which is exactly right for this and exactly wrong for a session.

use std::io;
use std::path::PathBuf;

/// Where this user's endpoint lives.
///
/// `$XDG_RUNTIME_DIR` where the platform sets one, which is already private to
/// its owner and cleared at logout. Otherwise the session state directory's own
/// parent, which is private for the same reason and is at least on a filesystem
/// this user can write.
///
/// # Errors
///
/// Where no per-user directory can be named at all.
pub fn path() -> io::Result<PathBuf> {
    if let Some(dir) = runtime_dir() {
        return Ok(dir.join("slipcase-open").join("front-door"));
    }
    let sessions = crate::session::default_root()?;
    let base = sessions.parent().unwrap_or(&sessions).to_path_buf();
    Ok(base.join("front-door"))
}

#[cfg(unix)]
fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(unix))]
fn runtime_dir() -> Option<PathBuf> {
    None
}

/// Make the endpoint's directory, private to its owner.
///
/// Set after creation rather than left to the umask, which is the user's: a
/// permissive one would leave the front door reachable by every account on the
/// machine, and §8 puts that among the requirements rather than the
/// improvements.
///
/// # Errors
///
/// Where the directory cannot be made or narrowed.
pub fn prepare(at: &std::path::Path) -> io::Result<()> {
    let dir = at.parent().unwrap_or(at);
    std::fs::create_dir_all(dir)?;
    private(dir)
}

#[cfg(unix)]
fn private(dir: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn private(_dir: &std::path::Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub use unix::{bind, connect, Incoming, Listener, Stream};

#[cfg(unix)]
mod unix {
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    /// One connection.
    pub type Stream = UnixStream;

    /// The bound endpoint. Unlinks the socket when it goes.
    #[derive(Debug)]
    pub struct Listener {
        inner: UnixListener,
        path: std::path::PathBuf,
    }

    /// Connections as they arrive.
    pub type Incoming<'a> = std::os::unix::net::Incoming<'a>;

    impl Listener {
        /// Connections as they arrive.
        pub fn incoming(&self) -> Incoming<'_> {
            self.inner.incoming()
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Best effort. A socket left behind is cleared by the next `bind`,
            // which is written for that case because a crash cannot run this.
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Speak to the instance already running, if there is one.
    ///
    /// # Errors
    ///
    /// Where there is nothing listening, or the connection fails.
    pub fn connect(at: &Path) -> io::Result<Stream> {
        UnixStream::connect(at)
    }

    /// Become the instance.
    ///
    /// **A refused connection means the socket is debris, not a rival.** A
    /// crashed instance leaves the file behind and nothing listening on it, so
    /// binding fails with *address in use* forever until somebody removes it.
    /// Removing it is only safe after a connection has been refused, which is
    /// what says nobody is on the other end — deleting an endpoint somebody is
    /// serving would take the running instance's front door away and leave two
    /// processes holding sessions.
    ///
    /// # Errors
    ///
    /// Where the endpoint cannot be bound, including where another instance
    /// bound it first. A caller that loses that race connects instead.
    pub fn bind(at: &Path) -> io::Result<Listener> {
        super::prepare(at)?;
        match UnixListener::bind(at) {
            Ok(inner) => Ok(Listener {
                inner,
                path: at.to_owned(),
            }),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                // Ask before clearing. Anything but a refusal means somebody is
                // there, and the caller should be talking to them instead.
                if UnixStream::connect(at).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "another instance is listening",
                    ));
                }
                std::fs::remove_file(at)?;
                Ok(Listener {
                    inner: UnixListener::bind(at)?,
                    path: at.to_owned(),
                })
            }
            Err(e) => Err(e),
        }
    }
}

/// Not yet, and deliberately not approximated.
///
/// Concept 8 wants a named pipe with an ACL naming the invoking user's SID.
/// That is the platform's own mechanism, and it is what makes the front door
/// this user's rather than the machine's. A pipe with a default descriptor is a
/// different thing wearing the same name. PLAN.md Phase 4.
#[cfg(not(unix))]
pub fn connect(_at: &std::path::Path) -> io::Result<std::net::TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the front door is not implemented on this platform yet",
    ))
}

#[cfg(test)]
mod tests {
    use super::{bind, connect, path, prepare};
    use crate::ipc::{answer, ask, take, Request, Response};

    #[test]
    fn the_endpoint_is_under_a_per_user_directory() {
        let at = path().unwrap();
        assert!(at.is_absolute());
        assert_eq!(at.file_name().unwrap(), "front-door");
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_is_owner_only_whatever_the_umask_says() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let at = tmp.path().join("run/slipcase-open/front-door");
        prepare(&at).unwrap();
        let mode = std::fs::metadata(at.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_request_reaches_the_instance_and_the_answer_comes_back() {
        let tmp = tempfile::tempdir().unwrap();
        let at = tmp.path().join("front-door");
        let listener = bind(&at).unwrap();

        let serving = std::thread::spawn(move || {
            let mut stream = listener.incoming().next().unwrap().unwrap();
            let request = take(&mut stream).unwrap();
            answer(&mut stream, &Response::Ok(vec![format!("{request:?}")])).unwrap();
        });

        let mut client = connect(&at).unwrap();
        let got = ask(&mut client, &Request::Ping).unwrap();
        serving.join().unwrap();
        assert_eq!(got, Response::Ok(vec!["Ping".to_string()]));
    }

    #[cfg(unix)]
    #[test]
    fn nothing_listening_is_a_connection_that_fails_rather_than_a_hang() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(connect(&tmp.path().join("front-door")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_a_crash_left_behind_is_cleared_rather_than_blocking_forever() {
        // What a crash leaves: the file on disk and no descriptor behind it.
        // `std::os::unix::net::UnixListener` does not unlink on drop, so
        // dropping one is exactly that state — where `mem::forget` would keep
        // the descriptor open in this process and still be listening, which is
        // the opposite of the case.
        //
        // Without the clearing rule, binding fails with *address in use* until
        // somebody deletes the file by hand: a tool that stops working after
        // one crash.
        let tmp = tempfile::tempdir().unwrap();
        let at = tmp.path().join("front-door");
        drop(std::os::unix::net::UnixListener::bind(&at).unwrap());
        assert!(at.exists(), "the debris should still be there");
        assert!(connect(&at).is_err(), "nothing should be listening on it");

        let listener = bind(&at);
        assert!(listener.is_ok(), "{:?}", listener.err());
        assert!(connect(&at).is_ok(), "the new instance should answer");
    }

    #[cfg(unix)]
    #[test]
    fn an_endpoint_somebody_is_serving_is_not_taken_from_them() {
        // The other half of the rule. Clearing a live socket would take the
        // running instance's front door away and leave two processes holding
        // sessions on the same containers.
        let tmp = tempfile::tempdir().unwrap();
        let at = tmp.path().join("front-door");
        let _live = bind(&at).unwrap();

        let second = bind(&at);
        assert!(
            second.is_err(),
            "the endpoint was taken from a live instance"
        );
        // And the live one still answers.
        assert!(connect(&at).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_listener_takes_the_socket_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let at = tmp.path().join("front-door");
        {
            let _listener = bind(&at).unwrap();
            assert!(at.exists());
        }
        assert!(!at.exists());
    }
}
