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
/// Deliberately small. Concept 6.3 removed the *payload* digest this used to
/// carry: the container records a CRC-32 for its payload already, so recovery
/// compares against the container rather than against a second copy of the fact
/// that can drift from it — and drift is likeliest at the moment this file is
/// consulted.
///
/// [`Record::agreed`] is not that digest coming back, and the difference is
/// worth being exact about. The removed one answered *has the payload changed*,
/// which the container can answer better. This one answers *which side changed*,
/// which nothing can answer without a record, because both sides are only
/// visible now and the question is about then. It is a note of a past moment
/// rather than a cached copy of a present fact, so there is nothing for it to
/// drift from: if it is stale, the answer it gives — that the container is not
/// where we left it — is the true one.
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
    /// The container's payload CRC-32 at the last moment this session and the
    /// container were known to agree: the extraction, or the most recent
    /// write-back.
    ///
    /// **What it is for is telling which side moved.** A payload that differs
    /// from its container is either an edit that never landed or a container
    /// that changed underneath a dead session, and those want opposite
    /// treatment — the first is the person's own work and goes back, the second
    /// is a conflict only they can settle. Comparing the two sides now cannot
    /// separate them, because both are only observable in the present.
    ///
    /// `None` for a session written by a build that did not record it, and for
    /// one whose container could not be read at the time. Recovery treats that
    /// as *not known to agree*, which is the cautious direction: it asks.
    pub agreed: Option<u32>,
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

    /// Write down that the container's payload is this, and that it is what
    /// this session's payload came from or was last put into.
    ///
    /// Called at the two moments the two sides are made to agree: the
    /// extraction, and the commit of a write-back. Nowhere else — a value
    /// recorded at any other moment would be recording an agreement that was
    /// never established, which is the one way [`Record::agreed`] could tell a
    /// lie rather than simply not know.
    ///
    /// # Errors
    ///
    /// Where the record cannot be rewritten.
    pub fn note_agreement(&mut self, crc: u32) -> io::Result<()> {
        self.record.agreed = Some(crc);
        write_record(&self.dir, &self.record)
    }

    /// Remove the session and everything in it.
    ///
    /// # Errors
    ///
    /// Where the directory cannot be removed inside [`PATIENCE`].
    pub fn remove(self) -> io::Result<()> {
        keep_trying(|| fs::remove_dir_all(&self.dir))
    }
}

/// How long a removal keeps trying before it reports the failure.
///
/// **What was measured, on 2026-09-03, and nothing beyond it.** Sessions were
/// surviving their own removal: the payload gone, an empty `payload/` left
/// behind, and the record still there, so `sessions` and the tray listed a
/// corpse. Two appeared during an ordinary sitting at the keyboard. The failure
/// is `ERROR_SHARING_VIOLATION` on the empty `payload/`, so `remove_dir_all`
/// got as far as unlinking the payload and no further.
///
/// **It is transient.** One corpse's directory was removed by hand a minute
/// later with nothing else changed. A scripted reproduction caught one and
/// retried it free immediately. That is the whole of what this constant rests
/// on, and it is enough: a condition that clears is one to wait out.
///
/// **It is also intermittent, and no trigger has been found.** Around twenty
/// runs of the sequence that produces it in real use — open, save, kill the
/// instance, let the next one's startup sweep remove what was left — produced
/// one corpse. An earlier reading, that the state directory's location was the
/// discriminator, did not survive six more runs in that same location and is
/// not the reason for anything here.
///
/// So this does not claim to end the corpse; it claims that a removal which
/// would have succeeded a moment later now does. Whether that covers every
/// occurrence is unknown, and the honest test is whether they stop appearing in
/// use.
///
/// Three hundred milliseconds is a dozen attempts, far more than the one that
/// sufficed, and short enough that a sweep of a dozen corpses cannot noticeably
/// delay a launch.
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(300);

/// How long to wait between attempts.
const BETWEEN: std::time::Duration = std::time::Duration::from_millis(20);

/// Do it, and go on doing it while it keeps failing, up to [`PATIENCE`].
///
/// **This accommodates a transient condition; it does not fix a known holder,
/// and it must not be described as doing so.** What is established is that the
/// directory is unremovable for a moment and removable shortly afterwards. Who
/// has it is not established, and two stories that fit have already been
/// measured and found wrong: Windows delete-pending, tested directly and shown
/// not to block the `rmdir` at all, and the target application holding the
/// payload, ruled out by a corpse that cleared with the editor still open.
///
/// A missing directory is not retried: it is not going to appear, and the
/// caller asked for it gone.
fn keep_trying(mut attempt: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut waited = std::time::Duration::ZERO;
    loop {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(e),
            Err(e) if waited >= PATIENCE => return Err(e),
            Err(_) => {
                std::thread::sleep(BETWEEN);
                waited += BETWEEN;
            }
        }
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
            // Not known yet. `extract` is what sets it, because that is the
            // moment the two are made to agree.
            agreed: None,
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
    if let Some(agreed) = record.agreed {
        doc["agreed"] = toml_edit::value(i64::from(agreed));
    }
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
        // Absent where an older build wrote this, which recovery reads as not
        // known to agree rather than as agreeing.
        agreed: doc
            .get("agreed")
            .and_then(toml_edit::Item::as_integer)
            .and_then(|n| u32::try_from(n).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::{create, default_root, keep_trying, scan, PATIENCE, PAYLOAD_DIR, RECORD};
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

    #[test]
    fn a_removal_that_fails_and_then_stops_failing_succeeds() {
        // The property the fix rests on, and the only one it claims: a
        // condition that clears is waited out rather than reported. The
        // measured case cleared on the first retry; this one takes two, which
        // is the same shape with margin.
        let attempts = std::cell::Cell::new(0);
        let outcome = keep_trying(|| {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "in use by another process",
                ))
            } else {
                Ok(())
            }
        });
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn a_removal_that_never_stops_failing_reports_it() {
        // Patience is not silence. Something that is genuinely stuck is still a
        // failure, and the caller still hears the operating system's own words
        // about it rather than a sentence this module invented.
        let attempts = std::cell::Cell::new(0);
        let outcome = keep_trying(|| {
            attempts.set(attempts.get() + 1);
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "in use by another process",
            ))
        });
        let why = outcome.expect_err("it should have given up");
        assert_eq!(why.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(why.to_string().contains("in use by another process"));
        // It tried more than once and stopped, rather than either giving up at
        // the first refusal or going round forever.
        assert!(attempts.get() > 1, "it did not retry at all");
        assert!(attempts.get() < 100, "{} attempts", attempts.get());
    }

    #[test]
    fn a_directory_that_is_not_there_is_not_waited_for() {
        // It is not going to appear. Waiting `PATIENCE` on every already-gone
        // session would put that wait into the sweep of a tidy state directory,
        // which is the common case and the one that must stay quick.
        let attempts = std::cell::Cell::new(0);
        let began = std::time::Instant::now();
        let outcome = keep_trying(|| {
            attempts.set(attempts.get() + 1);
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such directory",
            ))
        });
        assert!(outcome.is_err());
        assert_eq!(attempts.get(), 1);
        assert!(began.elapsed() < PATIENCE);
    }

    #[test]
    fn removing_a_session_takes_the_payload_and_the_record_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions");
        let c = a_container(tmp.path());
        let s = create(&root, &c, "report.pdf").unwrap();
        fs::write(s.payload_path(), b"something").unwrap();
        let dir = s.dir().to_path_buf();

        s.remove().unwrap();
        assert!(!dir.exists());
        assert!(scan(&root).unwrap().is_empty());
    }
}
