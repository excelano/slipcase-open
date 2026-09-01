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
    #[cfg(windows)]
    {
        // Not a filesystem path at all, and `main` never treats it as one: the
        // door is only ever handed to `bind` and `connect`, and printed once in
        // the refusal that names it.
        //
        // Two things name it. The SID, because the pipe namespace belongs to
        // the machine rather than to this account, so a fixed name would be one
        // door for everybody logged in. And the state directory, because the
        // door belongs to the sessions it serves — which is the same rule the
        // arm below follows by putting the socket beside them, and is what
        // gives a redirected world a front door of its own instead of reaching
        // into whatever this account already has running.
        let root = crate::session::default_root()?;
        let mut sum = crc32fast::Hasher::new();
        sum.update(root.as_os_str().as_encoded_bytes());
        Ok(PathBuf::from(format!(
            r"\\.\pipe\slipcase-open.{}.{:08x}",
            pipe::own_sid()?,
            sum.finalize()
        )))
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = runtime_dir() {
            return Ok(dir.join("slipcase-open").join("front-door"));
        }
        let sessions = crate::session::default_root()?;
        let base = sessions.parent().unwrap_or(&sessions).to_path_buf();
        Ok(base.join("front-door"))
    }
}

#[cfg(unix)]
fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(all(not(unix), not(windows)))]
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

/// Nothing to narrow. Windows scopes this by the inherited ACL on the directory
/// above rather than by a mode, so this is the shape of the platform and not a
/// stub waiting to be filled. `Result` because the Unix arm has one to give.
#[allow(clippy::unnecessary_wraps)]
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

/// Concept 8's named pipe, and the ACL that makes it this user's front door
/// rather than the machine's.
#[cfg(windows)]
pub use pipe::{bind, connect, Incoming, Listener, Stream};

#[cfg(windows)]
mod pipe {
    //! The front door on Windows.
    //!
    //! **A pipe leaves no debris, so there is no clearing rule here.** The Unix
    //! arm has to reason about a socket a crashed instance left behind, because
    //! the file outlives the process that held it. A named pipe does not exist
    //! apart from its instances: when the last handle closes, the name is gone.
    //! Measured 2026-09-01 — after the listener drops, a connect answers
    //! `NotFound` — and it is why `bind` below is shorter than its counterpart
    //! rather than for having skipped something it should have done.
    //!
    //! **`FILE_FLAG_FIRST_PIPE_INSTANCE` is the exclusion.** It refuses with
    //! `ERROR_ACCESS_DENIED` where any instance of the name exists already,
    //! which is precisely *another instance is listening*, and is answered as
    //! `AddrInUse` so that `main` hands over rather than failing. There is no
    //! race to lose between asking and binding, where the Unix arm has to
    //! connect first to tell a rival from debris.
    //!
    //! **The name carries the SID and the descriptor enforces it.** The pipe
    //! prefix is one namespace for the whole machine, so a fixed name would be
    //! the machine's door and two accounts would collide on it. The SID in the
    //! name keeps them apart; the ACL is what keeps them out, and concept 8 puts
    //! that among the requirements rather than the hardening.
    //!
    //! **Only the server half reaches past `std`.** A pipe opens as a file, so
    //! `connect` is `OpenOptions` and nothing else, and `Stream` is
    //! `std::fs::File` with the `Read` and `Write` that `ipc` asks for.

    use std::cell::Cell;
    use std::io;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::path::Path;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        LocalFree, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// What one instance holds in each direction. A request is a path and a verb
    /// and a response is a few lines, so this is room to spare rather than a
    /// budget; the pipe blocks and does not truncate when it is reached.
    const BUFFER: u32 = 4096;

    /// Every instance busy serving somebody. `connect` waits rather than
    /// failing: the door is answered in the time it takes to read one request,
    /// and the caller has nowhere else to go.
    const BUSY_PAUSE: Duration = Duration::from_millis(20);
    const BUSY_TRIES: u32 = 50;

    /// One connection. A connected instance is a byte stream and `std` already
    /// gives `Read` and `Write` over a handle, which is all `ipc` wants.
    pub type Stream = std::fs::File;

    /// The bound endpoint.
    pub struct Listener {
        name: Vec<u16>,
        security: Vec<u16>,
        /// The instance waiting for the next client.
        ///
        /// One is always outstanding while the listener lives, and that is what
        /// holds the name: closing every instance would release it to whoever
        /// asked next. So [`Incoming::next`] makes the replacement *before* it
        /// hands the connected one over, rather than after.
        ///
        /// `Cell` rather than `&mut self`, so that `incoming` takes `&self` and
        /// reads exactly like the Unix arm — which leaves `resident::run` one
        /// body for both platforms with nothing to configure out.
        pending: Cell<Option<OwnedHandle>>,
    }

    impl std::fmt::Debug for Listener {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Listener").finish_non_exhaustive()
        }
    }

    /// Connections as they arrive.
    pub struct Incoming<'a> {
        listener: &'a Listener,
    }

    impl Listener {
        /// Connections as they arrive.
        #[must_use]
        pub fn incoming(&self) -> Incoming<'_> {
            Incoming { listener: self }
        }
    }

    impl Iterator for Incoming<'_> {
        type Item = io::Result<Stream>;

        fn next(&mut self) -> Option<Self::Item> {
            // No instance outstanding means an earlier turn could not make one,
            // so the door is shut. `run` reads the end of this iterator as the
            // listener having gone, and stands the instance down.
            let pending = self.listener.pending.take()?;
            if let Err(why) = wait_for_client(&pending) {
                return Some(Err(why));
            }
            // Before this one is handed over, not after: the name is held by an
            // instance existing, and between the last one closing and the next
            // being made it would be free for the taking.
            match instance(&self.listener.name, &self.listener.security, false) {
                Ok(next) => self.listener.pending.set(Some(next)),
                Err(why) => return Some(Err(why)),
            }
            Some(Ok(Stream::from(pending)))
        }
    }

    /// Speak to the instance already running, if there is one.
    ///
    /// # Errors
    ///
    /// Where there is nothing listening, or the connection fails.
    pub fn connect(at: &Path) -> io::Result<Stream> {
        let open = || std::fs::OpenOptions::new().read(true).write(true).open(at);
        for _ in 0..BUSY_TRIES {
            match open() {
                Err(why) if is_error(&why, ERROR_PIPE_BUSY) => {
                    std::thread::sleep(BUSY_PAUSE);
                }
                settled => return settled,
            }
        }
        open()
    }

    /// Become the instance.
    ///
    /// # Errors
    ///
    /// Where the pipe cannot be created, including where another instance holds
    /// the name already — which answers `AddrInUse`, so that a caller which lost
    /// the race connects instead.
    pub fn bind(at: &Path) -> io::Result<Listener> {
        let name = wide(at.as_os_str());
        let security = wide(std::ffi::OsStr::new(&descriptor()?));
        let first = instance(&name, &security, true).map_err(|why| {
            if is_error(&why, ERROR_ACCESS_DENIED) {
                io::Error::new(io::ErrorKind::AddrInUse, "another instance is listening")
            } else {
                why
            }
        })?;
        Ok(Listener {
            name,
            security,
            pending: Cell::new(Some(first)),
        })
    }

    /// Whether an error is this Win32 code.
    ///
    /// `raw_os_error` answers `i32` and the constants are `u32`. A cast between
    /// them is a lint this crate would have to silence, and the conversion says
    /// the same thing without one.
    fn is_error(why: &io::Error, code: u32) -> bool {
        why.raw_os_error()
            .and_then(|got| u32::try_from(got).ok())
            .is_some_and(|got| got == code)
    }

    /// A wide, null-terminated copy, which is what every `W` entry point wants.
    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// One entry — this account, full control — and nothing inherited into it.
    ///
    /// `D:P` is the protected part, and it is not decoration: without it the
    /// pipe takes whatever inheritable entries the container offers, which is
    /// not a set this code chose.
    fn descriptor() -> io::Result<String> {
        Ok(format!("D:P(A;;GA;;;{})", own_sid()?))
    }

    /// The SID of the account this process runs as, in string form.
    ///
    /// # Errors
    ///
    /// Where the process token cannot be opened or read.
    #[allow(unsafe_code)]
    pub(super) fn own_sid() -> io::Result<String> {
        let mut token = std::ptr::null_mut();
        // SAFETY: `GetCurrentProcess` is a pseudo-handle needing no release, and
        // `token` is a live pointer the callee writes an owned handle into. The
        // return is checked before it is read.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call above succeeded, so this is an owned handle and this
        // scope is now what closes it.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };

        let mut wanted = 0u32;
        // SAFETY: the documented way to ask for the size — a null buffer of
        // length zero, which fails and writes the length it would have needed.
        // The failure is expected, so the return is deliberately not read here;
        // what checks `wanted` is the call below succeeding with it.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &raw mut wanted,
            );
        }
        // `u64` rather than `u8`, and this is alignment rather than taste:
        // `TOKEN_USER` is read back out of these bytes and wants eight, where a
        // `Vec<u8>` promises one. Enough words to cover `wanted` bytes.
        let mut buffer = vec![0u64; (wanted as usize).div_ceil(8)];
        // SAFETY: `buffer` covers `wanted` bytes, which is the size the call
        // above asked for, and the callee writes no more than it is given.
        let read = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                wanted,
                &raw mut wanted,
            )
        };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: the call above filled `buffer` with a `TOKEN_USER` whose `Sid`
        // points inside it, and `buffer` outlives this call.
        let made = unsafe {
            ConvertSidToStringSidW(
                (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid,
                &raw mut text,
            )
        };
        if made == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `text` is a null-terminated string the call above allocated,
        // so it is read to its terminator and released with the matching free.
        let sid = unsafe {
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text.cast());
            sid
        };
        Ok(sid)
    }

    /// One instance of the pipe, waiting for nobody yet.
    ///
    /// `first` asks for `FILE_FLAG_FIRST_PIPE_INSTANCE`, which is what makes
    /// `bind` exclusive. The instances made afterwards to replace a connected
    /// one must not ask for it, because by then the name is deliberately taken.
    #[allow(unsafe_code)]
    fn instance(name: &[u16], security: &[u16], first: bool) -> io::Result<OwnedHandle> {
        let mut sd = std::ptr::null_mut();
        // SAFETY: `security` is null-terminated by `wide`, and `sd` is a live
        // pointer the callee writes an allocated descriptor into. The return is
        // checked before `sd` is used.
        let made = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                security.as_ptr(),
                SDDL_REVISION_1,
                &raw mut sd,
                std::ptr::null_mut(),
            )
        };
        if made == 0 {
            return Err(io::Error::last_os_error());
        }

        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let mut access = PIPE_ACCESS_DUPLEX;
        if first {
            access |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: `name` is null-terminated by `wide`, and `attributes` holds
        // the descriptor made above, which is live until it is released below.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                access,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                &raw const attributes,
            )
        };
        // Taken before the descriptor is released, because the free sets this
        // thread's last error too and would then answer for the wrong call.
        let why = io::Error::last_os_error();
        // SAFETY: `sd` was allocated by the conversion above and nothing else
        // holds it — `CreateNamedPipeW` copies what it needs.
        unsafe {
            LocalFree(sd.cast());
        }

        if handle == INVALID_HANDLE_VALUE {
            return Err(why);
        }
        // SAFETY: the call succeeded, so this is an owned handle and nothing
        // else holds a copy of it.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    /// Block until somebody connects to `pending`.
    #[allow(unsafe_code)]
    fn wait_for_client(pending: &OwnedHandle) -> io::Result<()> {
        // SAFETY: `pending` is a live pipe instance owned by the caller, and a
        // null overlapped structure is the documented blocking form.
        let connected = unsafe { ConnectNamedPipe(pending.as_raw_handle(), std::ptr::null_mut()) };
        if connected != 0 {
            return Ok(());
        }
        let why = io::Error::last_os_error();
        // A client that arrived between the instance being made and this call is
        // already through the door, which is success wearing an error code.
        if is_error(&why, ERROR_PIPE_CONNECTED) {
            return Ok(());
        }
        Err(why)
    }
}

/// Neither a socket nor a pipe, so there is no front door to offer.
///
/// # Errors
///
/// Always.
#[cfg(not(any(unix, windows)))]
pub fn connect(_at: &std::path::Path) -> io::Result<std::net::TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the front door is not implemented on this platform yet",
    ))
}

// The Unix socket's own. Gated at the module rather than per test, which was
// written so that the day the pipe landed the gate would be one line to
// reconsider rather than six. That day was Phase 4 and the answer was that the
// gate stays: what is below tests a socket — its permission bits, and the
// clearing rule a file that outlives its process needs — and the pipe answers
// none of those questions. Its tests are in `windows_tests` below.
#[cfg(all(test, unix))]
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

// The named pipe's own. Concept 8's front door on Windows, tested against the
// same questions the Unix module asks, with one asked in reverse: there is no
// debris to clear here, and that is a property worth a test rather than a
// paragraph.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::{bind, connect, path};
    use crate::ipc::{answer, ask, take, Request, Response};

    /// A door of this test's own.
    ///
    /// The pipe namespace belongs to the machine, so two tests sharing a name
    /// would share a door and the suite would turn on which of them bound
    /// first. The process id keeps concurrent runs apart as well.
    fn a_door(what: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            r"\\.\pipe\slipcase-open-test.{}.{}",
            std::process::id(),
            what
        ))
    }

    #[test]
    fn the_endpoint_is_this_users_pipe() {
        let at = path().unwrap();
        let name = at.to_str().unwrap();
        assert!(name.starts_with(r"\\.\pipe\slipcase-open."), "{name}");
        // The SID is what keeps two accounts logged into one machine off a
        // single door, so its absence would be the whole guarantee missing.
        assert!(name.contains("S-1-"), "{name}");
    }

    #[test]
    fn a_request_reaches_the_instance_and_the_answer_comes_back() {
        let at = a_door("round-trip");
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

    #[test]
    fn the_door_stays_open_for_the_next_caller() {
        // The replacement instance is made before the connected one is handed
        // over, and this is what that is for: a second caller arriving after
        // the first has been served finds a door rather than a closed name.
        let at = a_door("second-caller");
        let listener = bind(&at).unwrap();

        let serving = std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let request = take(&mut stream).unwrap();
                answer(&mut stream, &Response::Ok(vec![format!("{request:?}")])).unwrap();
            }
        });

        for _ in 0..2 {
            let mut client = connect(&at).unwrap();
            assert_eq!(
                ask(&mut client, &Request::Ping).unwrap(),
                Response::Ok(vec!["Ping".to_string()])
            );
        }
        serving.join().unwrap();
    }

    #[test]
    fn nothing_listening_is_a_connection_that_fails_rather_than_a_hang() {
        assert!(connect(&a_door("empty")).is_err());
    }

    #[test]
    fn an_endpoint_somebody_is_serving_is_not_taken_from_them() {
        // `FILE_FLAG_FIRST_PIPE_INSTANCE` is what refuses, and the refusal is
        // translated so that `main` reads it as somebody to hand over to rather
        // than as a failure to report.
        let at = a_door("rival");
        let _live = bind(&at).unwrap();

        match bind(&at) {
            Ok(_) => panic!("the endpoint was taken from a live instance"),
            Err(why) => assert_eq!(why.kind(), std::io::ErrorKind::AddrInUse),
        }
    }

    #[test]
    fn a_pipe_leaves_nothing_behind_to_clear() {
        // The counterpart of the Unix arm's test for clearing a socket that a
        // crash left behind, and it asserts the opposite: a name does not
        // outlive the handles that hold it, so a crashed instance leaves
        // nothing for the next one to reason about. That is why `bind` here has
        // no clearing rule, rather than because it was skipped.
        let at = a_door("debris");
        {
            let _listener = bind(&at).unwrap();
        }
        assert!(connect(&at).is_err(), "the name outlived its listener");
        // And so binding again is an ordinary bind rather than a recovery.
        assert!(bind(&at).is_ok());
    }
}
