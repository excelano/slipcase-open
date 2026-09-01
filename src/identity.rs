//! What makes two paths the same file.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8 keys the live session table on file identity rather than on a
//! path, and the reason is the case §7 already warns about: a container
//! reachable under two hard links is two paths and one file. Canonicalising
//! resolves symbolic links and does nothing about that, so a table keyed on the
//! canonical path would open two sessions on one container, both repacking it,
//! the second write-back overwriting the first with nothing said.
//!
//! **Where the filesystem will not answer, the path is the answer.** Some
//! network mounts report a zero or unstable file index, and a key built from
//! one would make every lookup a miss — which is the failure that opens the
//! second session. Falling back to the canonical path narrows the guarantee to
//! what canonicalising gives, and says so in the type rather than pretending.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// What the filesystem says this file is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Identity {
    /// The file itself, as the filesystem numbers it. Two hard links to one
    /// inode compare equal here, which is the point.
    File {
        /// Device on Unix, volume serial number on Windows.
        volume: u64,
        /// Inode on Unix, file index on Windows.
        file: u64,
    },
    /// The filesystem would not give a stable number, so this is the
    /// canonicalised path and the narrower guarantee that comes with it.
    Path(PathBuf),
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File { volume, file } => write!(f, "{volume:x}:{file:x}"),
            Self::Path(p) => write!(f, "{}", p.display()),
        }
    }
}

/// What the filesystem says about the file at `path`.
///
/// # Errors
///
/// Where the path cannot be resolved or the file cannot be stat'd. A container
/// that is not there has no identity, which is a different answer from two
/// containers not matching.
pub fn of(path: &Path) -> io::Result<Identity> {
    // Resolved first, so that the fallback arm is a canonical path rather than
    // whatever the caller happened to type, and so a symbolic link is the file
    // it points at on both arms.
    let real = std::fs::canonicalize(path)?;
    let meta = std::fs::metadata(&real)?;
    Ok(
        numbers(&real, &meta).map_or(Identity::Path(real), |(volume, file)| Identity::File {
            volume,
            file,
        }),
    )
}

/// The device and inode, where they are worth anything.
///
/// A zero inode is the tell that nothing real is behind the number. Some
/// network filesystems report one for every file, and a key that collapses
/// every container on such a mount into one entry is worse than no key at all —
/// it would refuse to open the second container the user asked for, believing
/// it already had it.
#[cfg(unix)]
fn numbers(_path: &Path, meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    match (meta.dev(), meta.ino()) {
        (_, 0) => None,
        (dev, ino) => Some((dev, ino)),
    }
}

/// The volume serial number and the file index, through
/// `GetFileInformationByHandle`.
///
/// Concept 8 names that pair and this answered `None` until Phase 4, so Windows
/// had no hard-link arm — which is the case the whole module exists for.
/// `std::os::windows::fs::MetadataExt` exposes both and has kept them behind the
/// unstable `windows_by_handle` feature since 2019, and this crate builds on
/// stable, so the obvious implementation was never available.
///
/// **`same-file` was the other route PLAN.md named, and it does not fit.** Its
/// `Handle` keys on exactly this pair and compares on it, but `key` and the
/// `Key` type behind it are both private with no accessor, and a `Handle` holds
/// the file open for as long as it lives. [`Identity`] is owned data held in the
/// session table across a whole session and printed by `Display`, so the two
/// numbers have to come out. Measured by reading `same-file-1.0.6/src/win.rs`.
/// That leaves the call itself, and the `deny` `Cargo.toml` now carries in place
/// of `forbid` so that this one module can lift it.
///
/// **Opened for its attributes and nothing more.** `FILE_READ_ATTRIBUTES` is
/// what the call needs and the least it can ask for, and the share mode is
/// `std`'s default of read, write and delete. Nothing here may stand in the way
/// of the application editing its payload, and measured on 2026-09-01: a
/// container already held open by another handle for reading and writing still
/// answers.
///
/// A zero index is the tell the Unix arm reads from a zero inode, and gets the
/// same answer — no stable number, so the caller falls back to the canonicalised
/// path and the narrower guarantee that comes with it.
#[cfg(windows)]
#[allow(unsafe_code)]
fn numbers(path: &Path, _meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_READ_ATTRIBUTES,
    };

    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .open(path)
        .ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open and owned by `file`, which outlives the call;
    // `info` is a live, correctly typed allocation the callee only writes. The
    // return value is checked before anything in `info` is read.
    let got = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) };
    if got == 0 {
        return None;
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    match (u64::from(info.dwVolumeSerialNumber), index) {
        (_, 0) => None,
        pair => Some(pair),
    }
}

#[cfg(not(any(unix, windows)))]
fn numbers(_path: &Path, _meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::of;
    use std::fs;

    #[test]
    fn a_file_is_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("report.slpc");
        fs::write(&p, b"x").unwrap();
        assert_eq!(of(&p).unwrap(), of(&p).unwrap());
    }

    #[test]
    fn two_files_are_not_each_other() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.slpc");
        let b = tmp.path().join("b.slpc");
        fs::write(&a, b"x").unwrap();
        fs::write(&b, b"x").unwrap();
        assert_ne!(of(&a).unwrap(), of(&b).unwrap());
    }

    #[test]
    fn a_relative_path_and_an_absolute_one_are_the_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("report.slpc");
        fs::write(&p, b"x").unwrap();

        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let relative = of(std::path::Path::new("report.slpc"));
        std::env::set_current_dir(previous).unwrap();

        assert_eq!(relative.unwrap(), of(&p).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_the_file_it_points_at() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("report.slpc");
        let link = tmp.path().join("link.slpc");
        fs::write(&real, b"x").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(of(&link).unwrap(), of(&real).unwrap());
    }

    #[test]
    fn two_hard_links_are_one_container() {
        // The case a canonicalised path cannot see, and the reason concept 8
        // keys on identity. Both names resolve to themselves and differ, and
        // both are the same file.
        //
        // Not `cfg(unix)` since Phase 4. This is the test the Windows arm was
        // written for, and gating it there would have left the arm asserting
        // nothing on the platform it was added for.
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.slpc");
        let b = tmp.path().join("b.slpc");
        fs::write(&a, b"x").unwrap();
        fs::hard_link(&a, &b).unwrap();

        assert_ne!(fs::canonicalize(&a).unwrap(), fs::canonicalize(&b).unwrap());
        assert_eq!(of(&a).unwrap(), of(&b).unwrap());
    }

    #[test]
    fn a_file_that_is_not_there_has_no_identity() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(of(&tmp.path().join("gone.slpc")).is_err());
    }

    #[test]
    fn identity_survives_the_file_being_rewritten_in_place() {
        // Write-back replaces the container by renaming over it, which gives it
        // a new inode. The table is keyed while a session is open, so what
        // matters is that the same path still resolves to whatever is there —
        // and that a stale key is a miss rather than a wrong hit.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("report.slpc");
        fs::write(&p, b"first").unwrap();
        let before = of(&p).unwrap();

        let scratch = tmp.path().join("scratch");
        fs::write(&scratch, b"second").unwrap();
        fs::rename(&scratch, &p).unwrap();
        let after = of(&p).unwrap();

        assert_ne!(before, after, "a replaced file is a different file");
        assert_eq!(after, of(&p).unwrap());
    }
}
