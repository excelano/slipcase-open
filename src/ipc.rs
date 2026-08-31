//! The front door: one instance, and every other invocation a client of it.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8. Launching a document returns at once, so something has to outlive
//! the launch to hold the watch, and a session list in the plural means a second
//! invocation hands its container to the first rather than starting a rival with
//! a list of its own.
//!
//! **The front door is a control surface and is treated as one.** §10 says a
//! value supplied over IPC is a policy bypass, and the requests themselves are
//! the same problem: a local process that can reach the endpoint could hand the
//! engine a container, close a session before its final repack, or discard a
//! recovery item. The endpoint sits in a directory only its owner can traverse,
//! which is the platform's own mechanism and a requirement rather than a
//! hardening measure. A frame longer than [`MAX_FRAME`] is refused before it is
//! allocated, because a length somebody else chose is not a length to trust.
//!
//! ## The wire
//!
//! A frame is four bytes of big-endian length and then that many bytes. The
//! body is NUL-separated fields, the first of which names the verb. NUL rather
//! than a newline or a tab, because a path may contain either of those on Unix
//! and may not contain NUL — a protocol a filename can break is a protocol that
//! breaks on exactly the containers somebody was careless naming.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;

/// The largest frame this will read. Requests are a verb and a path; anything
/// approaching this is a mistake or an attempt.
pub const MAX_FRAME: usize = 64 * 1024;

/// Who says what came of an `open`.
///
/// An invocation started from a desktop entry has no terminal, so the lines it
/// is handed back go nowhere and the person who double-clicked learns nothing —
/// including, on the paths that matter most, that the payload was refused or
/// that it is an executable wearing a document's name (concept 5.1). An
/// invocation from a shell has a terminal and will print them itself, and an
/// instance that also announced them would say everything twice.
///
/// Only the client knows which it is, so the client says. This decides where a
/// message is shown and nothing else: concept 8 calls the front door a control
/// surface, and a field that moves text between two of this tool's own outputs
/// is not one of its controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// The client has somewhere to print, and will.
    Client,
    /// The client has not, so the instance speaks through concept 9's channel.
    Instance,
}

/// What an invocation asks the resident instance to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Open this container, or bring its session forward if it is already open.
    Open {
        container: PathBuf,
        /// Who says what came of it.
        voice: Voice,
    },
    /// What is open, and what is left over.
    List,
    /// Close this session, by the name `List` gives it.
    Close(String),
    /// Are you there.
    Ping,
}

/// What it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Done, with lines for the caller to print.
    Ok(Vec<String>),
    /// Not done, and why.
    Err(String),
}

/// A frame that could not be understood.
#[derive(Debug)]
pub enum Error {
    /// The connection failed, or ended mid-frame.
    Io(io::Error),
    /// The frame said it was longer than [`MAX_FRAME`].
    TooLong(usize),
    /// The body was not a request or a response this build knows.
    Malformed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::TooLong(n) => write!(f, "a frame of {n} bytes is longer than {MAX_FRAME}"),
            Self::Malformed(what) => write!(f, "unintelligible frame: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl Request {
    fn fields(&self) -> Vec<Vec<u8>> {
        match self {
            Self::Open { container, voice } => vec![
                b"open".to_vec(),
                path_bytes(container),
                match voice {
                    Voice::Client => b"client".to_vec(),
                    Voice::Instance => b"instance".to_vec(),
                },
            ],
            Self::List => vec![b"list".to_vec()],
            Self::Close(id) => vec![b"close".to_vec(), id.as_bytes().to_vec()],
            Self::Ping => vec![b"ping".to_vec()],
        }
    }

    fn from_fields(fields: &[Vec<u8>]) -> Result<Self, Error> {
        let verb = fields.first().map(Vec::as_slice).unwrap_or_default();
        match (verb, fields.len()) {
            (b"open", 3) => Ok(Self::Open {
                container: path_from(&fields[1]),
                voice: match fields[2].as_slice() {
                    b"client" => Voice::Client,
                    b"instance" => Voice::Instance,
                    other => {
                        return Err(Error::Malformed(format!(
                            "open with an unknown voice {}",
                            String::from_utf8_lossy(other)
                        )))
                    }
                },
            }),
            (b"list", 1) => Ok(Self::List),
            (b"close", 2) => Ok(Self::Close(text(&fields[1]))),
            (b"ping", 1) => Ok(Self::Ping),
            _ => Err(Error::Malformed(format!(
                "{} with {} field(s)",
                String::from_utf8_lossy(verb),
                fields.len()
            ))),
        }
    }
}

impl Response {
    fn fields(&self) -> Vec<Vec<u8>> {
        match self {
            Self::Ok(lines) => std::iter::once(b"ok".to_vec())
                .chain(lines.iter().map(|l| l.as_bytes().to_vec()))
                .collect(),
            Self::Err(why) => vec![b"err".to_vec(), why.as_bytes().to_vec()],
        }
    }

    fn from_fields(fields: &[Vec<u8>]) -> Result<Self, Error> {
        match fields.first().map(Vec::as_slice) {
            Some(b"ok") => Ok(Self::Ok(fields[1..].iter().map(|f| text(f)).collect())),
            Some(b"err") if fields.len() == 2 => Ok(Self::Err(text(&fields[1]))),
            other => Err(Error::Malformed(format!(
                "{}",
                String::from_utf8_lossy(other.unwrap_or_default())
            ))),
        }
    }
}

/// A path as bytes, without going through UTF-8, because a Unix path need not
/// be UTF-8 and this must not refuse a container for how somebody named it.
#[cfg(unix)]
fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(not(unix))]
fn path_from(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn write_frame(to: &mut impl Write, fields: &[Vec<u8>]) -> Result<(), Error> {
    let body = fields.join(&0u8);
    let len = u32::try_from(body.len()).map_err(|_| Error::TooLong(body.len()))?;
    if body.len() > MAX_FRAME {
        return Err(Error::TooLong(body.len()));
    }
    to.write_all(&len.to_be_bytes())?;
    to.write_all(&body)?;
    to.flush()?;
    Ok(())
}

fn read_frame(from: &mut impl Read) -> Result<Vec<Vec<u8>>, Error> {
    let mut len = [0u8; 4];
    from.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    // Checked before the allocation, not after. The length is a number the
    // other end chose.
    if len > MAX_FRAME {
        return Err(Error::TooLong(len));
    }
    let mut body = vec![0u8; len];
    from.read_exact(&mut body)?;
    Ok(body.split(|b| *b == 0).map(<[u8]>::to_vec).collect())
}

/// Send a request and wait for the answer.
///
/// # Errors
///
/// Where the connection fails or the answer cannot be understood.
pub fn ask(stream: &mut (impl Read + Write), request: &Request) -> Result<Response, Error> {
    write_frame(stream, &request.fields())?;
    Response::from_fields(&read_frame(stream)?)
}

/// Read one request.
///
/// # Errors
///
/// Where the connection fails or the request cannot be understood.
pub fn take(stream: &mut impl Read) -> Result<Request, Error> {
    Request::from_fields(&read_frame(stream)?)
}

/// Answer one request.
///
/// # Errors
///
/// Where the connection fails.
pub fn answer(stream: &mut impl Write, response: &Response) -> Result<(), Error> {
    write_frame(stream, &response.fields())
}

#[cfg(test)]
mod tests {
    use super::{answer, ask, take, Error, Request, Response, Voice};
    use std::io::Cursor;
    use std::path::PathBuf;

    /// A pair of ends that hand bytes to each other, so a round trip needs no
    /// socket.
    struct Pair {
        to_them: Vec<u8>,
        from_them: Cursor<Vec<u8>>,
    }

    impl std::io::Write for Pair {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.to_them.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl std::io::Read for Pair {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::io::Read::read(&mut self.from_them, buf)
        }
    }

    fn round_trip(request: &Request) -> Request {
        let mut wire = Vec::new();
        let mut out = Pair {
            to_them: Vec::new(),
            from_them: Cursor::new(Vec::new()),
        };
        super::write_frame(&mut out, &request.fields()).unwrap();
        wire.extend_from_slice(&out.to_them);
        take(&mut Cursor::new(wire)).unwrap()
    }

    #[test]
    fn every_request_survives_the_wire() {
        for r in [
            Request::Open {
                container: PathBuf::from("/tmp/report.slpc"),
                voice: Voice::Client,
            },
            Request::List,
            Request::Close("6a94-0".to_string()),
            Request::Ping,
        ] {
            assert_eq!(round_trip(&r), r, "{r:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_path_a_line_protocol_would_break_survives() {
        // Both are legal in a Unix filename and both would end a frame in a
        // protocol delimited by them. NUL is the one byte a path cannot carry,
        // which is why it is the separator.
        for name in [
            "/tmp/two\tcolumns.slpc",
            "/tmp/two\nlines.slpc",
            "/tmp/a \"quoted\" name.slpc",
        ] {
            let r = Request::Open {
                container: PathBuf::from(name),
                voice: Voice::Instance,
            };
            assert_eq!(round_trip(&r), r, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_survives() {
        // A Unix path is bytes. Refusing one for how somebody named it would be
        // this tool deciding which containers may be opened on grounds SPEC has
        // no opinion about.
        use std::os::unix::ffi::OsStringExt as _;
        let name = PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'/', b't', b'm', b'p', b'/', 0xff, 0xfe, b'.', b's', b'l', b'p', b'c',
        ]));
        let r = Request::Open {
            container: name,
            voice: Voice::Client,
        };
        assert_eq!(round_trip(&r), r);
    }

    #[test]
    fn a_response_survives_the_wire() {
        let mut wire = Vec::new();
        answer(&mut wire, &Response::Ok(vec!["one".into(), "two".into()])).unwrap();
        let mut c = Cursor::new(wire);
        let mut both = Pair {
            to_them: Vec::new(),
            from_them: Cursor::new(Vec::new()),
        };
        std::io::copy(&mut c, &mut both.to_them).unwrap();
        both.from_them = Cursor::new(both.to_them.clone());
        // Read it back through the response decoder.
        let fields = super::read_frame(&mut both.from_them).unwrap();
        assert_eq!(
            super::Response::from_fields(&fields).unwrap(),
            Response::Ok(vec!["one".into(), "two".into()])
        );
    }

    #[test]
    fn an_error_response_carries_its_reason() {
        let mut wire = Vec::new();
        answer(&mut wire, &Response::Err("pdf is on the deny list".into())).unwrap();
        let fields = super::read_frame(&mut Cursor::new(wire)).unwrap();
        assert_eq!(
            super::Response::from_fields(&fields).unwrap(),
            Response::Err("pdf is on the deny list".into())
        );
    }

    #[test]
    fn a_frame_longer_than_the_cap_is_refused_before_it_is_allocated() {
        // The length is a number the other end chose. A local process that can
        // reach the endpoint should not be able to ask for a gigabyte.
        let mut wire = Vec::new();
        wire.extend_from_slice(&u32::MAX.to_be_bytes());
        match super::read_frame(&mut Cursor::new(wire)) {
            Err(Error::TooLong(n)) => assert_eq!(n, u32::MAX as usize),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_verb_this_build_does_not_know_is_refused_rather_than_guessed() {
        let mut wire = Vec::new();
        super::write_frame(&mut wire, &[b"drop-everything".to_vec()]).unwrap();
        assert!(matches!(
            take(&mut Cursor::new(wire)),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn a_known_verb_with_the_wrong_shape_is_refused() {
        // `open` with no path, and `close` with two ids. Both are a caller this
        // build does not understand, and guessing at either would act on
        // something nobody asked for.
        for fields in [
            vec![b"open".to_vec()],
            vec![b"close".to_vec(), b"a".to_vec(), b"b".to_vec()],
        ] {
            let mut wire = Vec::new();
            super::write_frame(&mut wire, &fields).unwrap();
            assert!(matches!(
                take(&mut Cursor::new(wire)),
                Err(Error::Malformed(_))
            ));
        }
    }

    #[test]
    fn a_connection_that_ends_mid_frame_is_an_error_and_not_a_hang() {
        let mut wire = Vec::new();
        super::write_frame(&mut wire, &Request::List.fields()).unwrap();
        wire.truncate(wire.len() - 1);
        assert!(matches!(take(&mut Cursor::new(wire)), Err(Error::Io(_))));
    }

    #[test]
    fn asking_writes_a_request_and_reads_the_answer() {
        let mut server_side = Vec::new();
        answer(&mut server_side, &Response::Ok(vec!["fine".into()])).unwrap();
        let mut pair = Pair {
            to_them: Vec::new(),
            from_them: Cursor::new(server_side),
        };
        assert_eq!(
            ask(&mut pair, &Request::Ping).unwrap(),
            Response::Ok(vec!["fine".into()])
        );
        // And the request really went out.
        assert_eq!(take(&mut Cursor::new(pair.to_them)).unwrap(), Request::Ping);
    }
}
