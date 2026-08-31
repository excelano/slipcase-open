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
        numbers(&meta).map_or(Identity::Path(real), |(volume, file)| Identity::File {
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
fn numbers(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    match (meta.dev(), meta.ino()) {
        (_, 0) => None,
        (dev, ino) => Some((dev, ino)),
    }
}

/// Not yet on Windows, and deliberately not approximated.
///
/// Concept 8 names the volume serial number and the file index, which is the
/// right pair. `std::os::windows::fs::MetadataExt` exposes both and has kept
/// them behind the unstable `windows_by_handle` feature since 2019, so the
/// obvious implementation is nightly-only and this crate builds on stable.
/// Written and never compiled until a cross-target check was added, which is
/// the argument for having one.
///
/// Answering `None` is not a gap: concept 8 already says that where a
/// filesystem returns no stable identity the lookup falls back to the
/// canonicalised path and accepts the narrower guarantee. What Windows loses
/// until Phase 4 is the hard-link arm, and it loses it visibly rather than by
/// an approximation that looks like an answer — the same rule `platform.rs`
/// follows for the launcher there.
///
/// Phase 4 has two ways to settle it and neither needs this file to change its
/// shape: `GetFileInformationByHandle` through a crate that carries the unsafe,
/// `same-file` being the obvious one, or `windows-sys` and a lifted `forbid`
/// in this module alone.
#[cfg(windows)]
fn numbers(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(not(any(unix, windows)))]
fn numbers(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
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

    #[cfg(unix)]
    #[test]
    fn two_hard_links_are_one_container() {
        // The case a canonicalised path cannot see, and the reason concept 8
        // keys on identity. Both names resolve to themselves and differ, and
        // both are the same file.
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
