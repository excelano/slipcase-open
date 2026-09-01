//! Where a session lives on disk, and what it remembers.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 6.4. **Not the system temporary directory**, which is the obvious
//! place and the wrong one: a reboot, a `tmpfiles` cleaner or Storage Sense may
//! delete anything there, and doing so would destroy an edit the user has made
//! and the tool has not yet written back — silently, in the window concept 6.3
//! exists to survive.
//!
//! So a session is a directory under the application's own per-user state
//! directory, and recovery is a scan of that one tree rather than a record
//! pointing somewhere that may no longer be there.
//!
//! ## The payload sits one level down
//!
//! A session directory holds `session.toml` and a `payload/` directory, and the
//! payload goes inside the latter under its own name. Two reasons, and the
//! first is a collision: SPEC 2.3 permits any plain filename, `session.toml`
//! included, so a payload beside the record could overwrite it. The second is
//! that concept 6.1 reads *anything else in the directory* as the target
//! application's doing, and that inference is only sound if the tool put
//! exactly one file there.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The file inside a session directory that carries [`Record`].
const RECORD: &str = "session.toml";

/// The directory inside a session directory that carries the payload, and
/// nothing this tool put there.
const PAYLOAD_DIR: &str = "payload";

/// How many names [`create`] will try before giving up. A thousand sessions
/// opened inside one second is not a thing that happens, and a directory that
/// somehow defeats the counter should say so rather than spin.
const ATTEMPTS: u32 = 1024;

/// What a session remembers across a crash.
///
/// Deliberately small. Concept 6.3 removed the digest this used to carry: the
/// container records a CRC-32 for its payload already, so recovery compares
/// against the container rather than against a second copy of the fact that can
/// drift from it — and drift is likeliest at the moment this file is consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Where the container was when the session opened, resolved. It may have
    /// moved or gone since, which concept 6.4 requires recovery to survive
    /// rather than fail at the rename.
    pub container: PathBuf,
    /// The payload's name inside the container, which is also its name inside
    /// `payload/` and therefore what decides which application opens it.
    pub payload: String,
    /// When the session opened, in seconds since the Unix epoch.
    ///
    /// A number rather than a formatted timestamp, because nothing in the tool
    /// needs to render it: concept 6.3 shows a person the payload's own
    /// modification time, which comes from the filesystem. Storing it this way
    /// keeps a date-formatting dependency out of a crate that would otherwise
    /// have no use for one.
    pub started: u64,
    /// How many write-backs this session has performed, which concept 6.2 shows
    /// beside the session.
    pub write_backs: u64,
}

/// An open or recoverable session on disk.
#[derive(Debug, Clone)]
pub struct Session {
    dir: PathBuf,
    record: Record,
}

impl Session {
    /// The session's own directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// What this session remembers.
    #[must_use]
    pub fn record(&self) -> &Record {
        &self.record
    }

    /// The directory the payload sits in, and the one to watch. Concept 6.1
    /// reads anything else appearing here as the target application's work.
    #[must_use]
    pub fn payload_dir(&self) -> PathBuf {
        self.dir.join(PAYLOAD_DIR)
    }

    /// The payload itself.
    #[must_use]
    pub fn payload_path(&self) -> PathBuf {
        self.payload_dir().join(&self.record.payload)
    }

    /// Count a write-back, and write that down before returning.
    ///
    /// Persisted rather than held, because the number is only worth anything to
    /// a session that crashed, and a count kept in memory is a count lost by
    /// the event it exists to describe.
    ///
    /// # Errors
    ///
    /// Where the record cannot be rewritten.
    pub fn note_write_back(&mut self) -> io::Result<()> {
        self.record.write_backs += 1;
        write_record(&self.dir, &self.record)
    }

    /// Remove the session and everything in it.
    ///
    /// # Errors
    ///
    /// Where the directory cannot be removed.
    pub fn remove(self) -> io::Result<()> {
        fs::remove_dir_all(&self.dir)
    }
}

/// The per-user state directory this build keeps sessions under.
///
/// `$XDG_STATE_HOME` on Linux, which is defined for state that must survive a
/// restart without being configuration or data, and never `XDG_RUNTIME_DIR`,
/// which is cleared at logout. `~/Library/Application Support` on macOS rather
/// than `Caches`, which the system may purge at will. `%LOCALAPPDATA%` on
/// Windows and deliberately not the roaming profile, since an extracted payload
/// cannot follow a user between machines.
///
/// # Errors
///
/// Where the platform names no home for this, which is a machine too unusual to
/// guess about rather than a condition to work around.
pub fn default_root() -> io::Result<PathBuf> {
    let base = platform_base().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no per-user state directory: set XDG_STATE_HOME, HOME, or LOCALAPPDATA",
        )
    })?;
    Ok(base.join("slipcase-open").join("sessions"))
}

#[cfg(target_os = "linux")]
fn platform_base() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x));
    }
    // The fallback the XDG base directory specification names, rather than one
    // of this project's choosing.
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/state"))
}

#[cfg(target_os = "macos")]
fn platform_base() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support"))
}

#[cfg(target_os = "windows")]
fn platform_base() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_base() -> Option<PathBuf> {
    None
}

/// Start a session under `root`, creating the tree if it is not there.
///
/// The container path is resolved before it is written down, so a container
/// reached through a symbolic link records the file rather than the link, and
/// so that what recovery reads later is a path and not a relative fragment
/// interpreted against whatever directory a later process happens to be in.
///
/// # Errors
///
/// Where the container cannot be resolved, or the tree cannot be created.
pub fn create(root: &Path, container: &Path, payload: &str) -> io::Result<Session> {
    let container = fs::canonicalize(container)?;
    let started = seconds_since_epoch();

    create_private_dir_all(root)?;

    // Named for when it started and made unique by the create itself, which is
    // atomic. No randomness: the root is the user's own and private, and a
    // counter that cannot collide is a smaller thing to get right than a source
    // of entropy would be.
    let mut made = None;
    for n in 0..ATTEMPTS {
        let candidate = root.join(format!("{started:x}-{n}"));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                made = Some(candidate);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let Some(dir) = made else {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{ATTEMPTS} session directories already exist for this second"),
        ));
    };
    private(&dir)?;

    let session = Session {
        record: Record {
            container,
            payload: payload.to_string(),
            started,
            write_backs: 0,
        },
        dir,
    };

    create_private_dir_all(&session.payload_dir())?;
    write_record(&session.dir, &session.record)?;
    Ok(session)
}

/// Every session under `root`, open or left behind.
///
/// A directory carrying no readable record is skipped rather than reported: it
/// is a session being created by another process right now, or the remains of
/// one that died between the two operations, and neither is something to fail a
/// recovery scan over.
///
/// # Errors
///
/// Where `root` exists and cannot be read. A `root` that is not there yet is an
/// empty list, because a machine that has never opened a container has no
/// sessions rather than a problem.
pub fn scan(root: &Path) -> io::Result<Vec<Session>> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found: Vec<Session> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let dir = e.path();
            read_record(&dir).ok().map(|record| Session { dir, record })
        })
        .collect();
    found.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(found)
}

/// The session under `root` with this directory name.
///
/// The name is what [`scan`] shows, so it is what a person types back.
///
/// # Errors
///
/// Where there is no such session, or its record cannot be read.
pub fn find(root: &Path, id: &str) -> io::Result<Session> {
    // Rejected rather than joined. A name carrying a separator would reach out
    // of the root, and the only names this answers to are ones `scan` printed.
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no session {id}"),
        ));
    }
    let dir = root.join(id);
    let record = read_record(&dir)?;
    Ok(Session { dir, record })
}

fn seconds_since_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Create a directory and its parents, owner-only.
fn create_private_dir_all(at: &Path) -> io::Result<()> {
    fs::create_dir_all(at)?;
    private(at)
}

/// Narrow a directory to its owner.
///
/// Set after creation rather than through the umask, because the umask is the
/// user's and a permissive one would leave a payload readable by every account
/// on the machine. Windows needs nothing: `%LOCALAPPDATA%` is already scoped by
/// an inherited ACL, and there is no mode to set.
#[cfg(unix)]
fn private(at: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(at, fs::Permissions::from_mode(0o700))
}

/// Nothing to narrow, for the reason the arm above gives. `Result` because that
/// arm has one to give.
#[allow(clippy::unnecessary_wraps)]
#[cfg(not(unix))]
fn private(_at: &Path) -> io::Result<()> {
    Ok(())
}

fn write_record(dir: &Path, record: &Record) -> io::Result<()> {
    let mut doc = toml_edit::DocumentMut::new();
    // Lossy is wrong for a path and right for nothing, so a path that is not
    // Unicode is refused here rather than written down wrongly and acted on
    // later. Rare on every platform this ships to, and silently mangling a
    // container's location is worse than saying so.
    let container = record.container.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "container path is not Unicode: {}",
                record.container.display()
            ),
        )
    })?;
    doc["container"] = toml_edit::value(container);
    doc["payload"] = toml_edit::value(record.payload.as_str());
    doc["started"] = toml_edit::value(i64::try_from(record.started).unwrap_or(i64::MAX));
    doc["write_backs"] = toml_edit::value(i64::try_from(record.write_backs).unwrap_or(i64::MAX));
    fs::write(dir.join(RECORD), doc.to_string())
}

fn read_record(dir: &Path) -> io::Result<Record> {
    let text = fs::read_to_string(dir.join(RECORD))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{RECORD}: {e}")))?;

    let mut want = BTreeMap::new();
    for key in ["container", "payload"] {
        let v = doc.get(key).and_then(|v| v.as_str()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{RECORD}: no string `{key}`"),
            )
        })?;
        want.insert(key, v.to_string());
    }
    let number = |key: &str| -> u64 {
        doc.get(key)
            .and_then(toml_edit::Item::as_integer)
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or_default()
    };

    Ok(Record {
        container: PathBuf::from(&want["container"]),
        payload: want["payload"].clone(),
        started: number("started"),
        write_backs: number("write_backs"),
    })
}

#[cfg(test)]
mod tests {
    use super::{create, default_root, scan, PAYLOAD_DIR, RECORD};
    use std::fs;

    /// A container on disk to point a session at. Its contents do not matter
    /// here; what matters is that the path resolves.
    fn a_container(at: &std::path::Path) -> std::path::PathBuf {
        let p = at.join("report.pdf.slpc");
        fs::write(&p, b"not a real container").unwrap();
        p
    }

    #[test]
    fn a_session_holds_its_record_and_a_payload_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        let s = create(&root, &c, "report.pdf").unwrap();
        assert!(s.dir().join(RECORD).is_file());
        assert!(s.payload_dir().is_dir());
        assert_eq!(s.payload_dir().file_name().unwrap(), PAYLOAD_DIR);
        assert_eq!(s.payload_path(), s.payload_dir().join("report.pdf"));
    }

    #[test]
    fn the_payload_sits_below_the_record_rather_than_beside_it() {
        // SPEC 2.3 permits any plain filename, `session.toml` included, so a
        // payload beside the record could overwrite it. And concept 6.1 reads
        // anything else in the payload directory as the target application's
        // doing, which is only sound if the tool put one file there.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        let s = create(&root, &c, RECORD).unwrap();
        fs::write(s.payload_path(), b"payload").unwrap();

        assert!(s.dir().join(RECORD).is_file());
        assert!(fs::read_to_string(s.dir().join(RECORD))
            .unwrap()
            .contains("payload ="));
        assert_eq!(fs::read(s.payload_path()).unwrap(), b"payload");
    }

    #[test]
    fn the_container_path_is_resolved_before_it_is_written_down() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        // Reached through a relative fragment, which a later process in another
        // working directory could not interpret.
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let s = create(&root, std::path::Path::new("report.pdf.slpc"), "report.pdf");
        std::env::set_current_dir(previous).unwrap();

        let s = s.unwrap();
        assert!(s.record().container.is_absolute());
        assert_eq!(s.record().container, fs::canonicalize(&c).unwrap());
    }

    #[test]
    fn two_sessions_on_the_same_container_get_directories_of_their_own() {
        // Whether that should be allowed is concept 8's question and the
        // engine's answer. This is only that the naming does not collide.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        let a = create(&root, &c, "report.pdf").unwrap();
        let b = create(&root, &c, "report.pdf").unwrap();
        assert_ne!(a.dir(), b.dir());
    }

    #[test]
    fn a_session_survives_being_written_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        // A name carrying the characters that would break a hand-rolled
        // writer. SPEC 2.3 permits both.
        let mut s = create(&root, &c, "a \"quoted\" \\ name.pdf").unwrap();
        s.note_write_back().unwrap();
        s.note_write_back().unwrap();

        let found = scan(&root).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].record(), s.record());
        assert_eq!(found[0].record().write_backs, 2);
    }

    #[test]
    fn a_write_back_count_is_on_disk_before_the_call_returns() {
        // It is only worth anything to a session that crashed, so a count kept
        // in memory is a count lost by the event it describes.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        let mut s = create(&root, &c, "report.pdf").unwrap();
        s.note_write_back().unwrap();
        assert_eq!(scan(&root).unwrap()[0].record().write_backs, 1);
    }

    #[test]
    fn scanning_a_root_that_is_not_there_finds_nothing_rather_than_failing() {
        // A machine that has never opened a container has no sessions rather
        // than a problem, and recovery runs on every launch.
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan(&tmp.path().join("never-used")).unwrap().is_empty());
    }

    #[test]
    fn a_directory_with_no_readable_record_is_skipped_rather_than_fatal() {
        // Another process creating a session right now, or the remains of one
        // that died between the two operations. Neither should fail a scan.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());
        let good = create(&root, &c, "report.pdf").unwrap();

        fs::create_dir(root.join("half-made")).unwrap();
        fs::write(root.join("truncated"), b"not a directory").unwrap();
        fs::create_dir(root.join("garbled")).unwrap();
        fs::write(root.join("garbled").join(RECORD), b"= not toml =").unwrap();

        let found = scan(&root).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dir(), good.dir());
    }

    #[test]
    fn removing_a_session_takes_the_payload_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        let s = create(&root, &c, "report.pdf").unwrap();
        fs::write(s.payload_path(), b"edited").unwrap();
        let dir = s.dir().to_path_buf();
        s.remove().unwrap();

        assert!(!dir.exists());
        assert!(scan(&root).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_tree_is_owner_only_whatever_the_umask_says() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());

        let s = create(&root, &c, "report.pdf").unwrap();
        for d in [&root, &s.dir().to_path_buf(), &s.payload_dir()] {
            let mode = fs::metadata(d).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", d.display());
        }
    }

    #[test]
    fn the_default_root_is_under_the_platforms_state_directory() {
        // Not asserted against a literal path, which would only restate the
        // code. What matters is that it is named, that it is not the system
        // temporary directory, and that sessions are under a directory of this
        // application's own.
        let root = default_root().unwrap();
        assert!(root.ends_with("slipcase-open/sessions"));
        assert!(!root.starts_with(std::env::temp_dir()));
    }
}
