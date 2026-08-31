//! The live sessions, and how a container is matched against them.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8: a container that already has a live session is not opened twice,
//! because two sessions on one container would both repack it and the second
//! write-back would overwrite the first with nothing said.
//!
//! ## Neither key is enough on its own
//!
//! §8 says to key on file identity rather than on a path, and gives the reason:
//! a container reachable under two hard links is two paths and one file, and a
//! canonical path cannot see that.
//!
//! What §8 does not account for — found while writing `identity.rs` — is that
//! **write-back replaces the container by renaming a new file over it, so a
//! container acquires a new inode every time a session saves.** An identity
//! recorded when the session opened stops matching the file at that path after
//! the first write-back, and the next invocation of the same container would
//! find no entry and open the second session that all of this exists to
//! prevent.
//!
//! So a lookup matches on either. The path is stable across replacement and
//! blind to hard links; the identity is the opposite. Together they cover both,
//! and the case where they disagree is handled correctly rather than by
//! accident: after a write-back through one of two hard links, §7 says the
//! other name still points at the original with the old contents, so the two
//! really are different files by then — different path, different identity, no
//! match, and a new session, which is right.

use std::path::{Path, PathBuf};

use crate::identity::{self, Identity};

/// One live session and the two ways of finding it again.
#[derive(Debug)]
struct Entry<T> {
    identity: Identity,
    path: PathBuf,
    held: T,
}

/// The sessions this instance is holding.
#[derive(Debug)]
pub struct Table<T> {
    entries: Vec<Entry<T>>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> Table<T> {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many sessions are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this instance is holding nothing, which is half of concept 8's
    /// exit rule.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Everything held, in the order it was opened.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|e| &e.held)
    }

    /// Everything held, mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|e| &mut e.held)
    }

    /// Take a session in, keyed on what `container` is now.
    ///
    /// # Errors
    ///
    /// Where the container's identity cannot be established.
    pub fn insert(&mut self, container: &Path, held: T) -> std::io::Result<()> {
        self.entries.push(Entry {
            identity: identity::of(container)?,
            path: std::fs::canonicalize(container)?,
            held,
        });
        Ok(())
    }

    /// The session already open on this container, if there is one.
    ///
    /// A container that is not there matches nothing rather than failing: an
    /// invocation naming a path that does not exist has a different problem,
    /// and it is not this function's to report.
    pub fn find_mut(&mut self, container: &Path) -> Option<&mut T> {
        let identity = identity::of(container).ok();
        let path = std::fs::canonicalize(container).ok();
        self.entries
            .iter_mut()
            .find(|e| {
                identity.as_ref() == Some(&e.identity) || path.as_deref() == Some(e.path.as_path())
            })
            .map(|e| &mut e.held)
    }

    /// Re-read the identity of a session's container.
    ///
    /// Called after a write-back, which renamed a new file over the container
    /// and so gave it a new inode. Without this the entry keeps matching by
    /// path and stops matching by identity, which quietly loses the hard-link
    /// half of the guarantee for the rest of the session.
    ///
    /// A container that has gone keeps the identity it had. There is nothing
    /// better to record, and the path arm still finds the session so that the
    /// person can be told.
    pub fn refresh(&mut self, container: &Path) {
        let Ok(now) = identity::of(container) else {
            return;
        };
        if let Some(entry) = self.entries.iter_mut().find(|e| {
            e.path == container || Some(&e.path) == std::fs::canonicalize(container).ok().as_ref()
        }) {
            entry.identity = now;
        }
    }

    /// Drop a session and hand it back.
    pub fn remove(&mut self, container: &Path) -> Option<T> {
        let identity = identity::of(container).ok();
        let path = std::fs::canonicalize(container).ok();
        let at = self.entries.iter().position(|e| {
            identity.as_ref() == Some(&e.identity) || path.as_deref() == Some(e.path.as_path())
        })?;
        Some(self.entries.remove(at).held)
    }

    /// Take everything, for the shutdown that closes each in turn.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.entries.drain(..).map(|e| e.held)
    }
}

#[cfg(test)]
mod tests {
    use super::Table;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn a_file(at: &Path, name: &str) -> PathBuf {
        let p = at.join(name);
        fs::write(&p, b"container").unwrap();
        p
    }

    #[test]
    fn a_container_finds_its_own_session() {
        let tmp = tempfile::tempdir().unwrap();
        let c = a_file(tmp.path(), "report.slpc");
        let mut table = Table::new();
        table.insert(&c, "session".to_string()).unwrap();
        assert_eq!(table.find_mut(&c).map(|s| s.as_str()), Some("session"));
    }

    #[test]
    fn another_container_finds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let a = a_file(tmp.path(), "a.slpc");
        let b = a_file(tmp.path(), "b.slpc");
        let mut table = Table::new();
        table.insert(&a, "a".to_string()).unwrap();
        assert!(table.find_mut(&b).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_second_hard_link_finds_the_same_session() {
        // Concept 8's stated reason for keying on identity: two names, one
        // file, and a canonical path cannot tell.
        let tmp = tempfile::tempdir().unwrap();
        let a = a_file(tmp.path(), "a.slpc");
        let b = tmp.path().join("b.slpc");
        fs::hard_link(&a, &b).unwrap();

        let mut table = Table::new();
        table.insert(&a, "one session".to_string()).unwrap();
        assert_eq!(table.find_mut(&b).map(|s| s.as_str()), Some("one session"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_finds_the_same_session() {
        let tmp = tempfile::tempdir().unwrap();
        let a = a_file(tmp.path(), "a.slpc");
        let link = tmp.path().join("link.slpc");
        std::os::unix::fs::symlink(&a, &link).unwrap();

        let mut table = Table::new();
        table.insert(&a, "one session".to_string()).unwrap();
        assert!(table.find_mut(&link).is_some());
    }

    #[test]
    fn a_container_replaced_by_a_write_back_still_finds_its_session() {
        // The case identity alone loses. `Destination::in_place` renames a new
        // file over the container, so the inode changes on every save; an entry
        // keyed only on the identity recorded at open would stop matching after
        // the first write-back, and the next invocation would start the second
        // session concept 8 exists to prevent.
        let tmp = tempfile::tempdir().unwrap();
        let c = a_file(tmp.path(), "report.slpc");
        let mut table = Table::new();
        table.insert(&c, "session".to_string()).unwrap();

        let scratch = tmp.path().join("scratch");
        fs::write(&scratch, b"repacked").unwrap();
        fs::rename(&scratch, &c).unwrap();

        assert_eq!(table.find_mut(&c).map(|s| s.as_str()), Some("session"));
    }

    #[cfg(unix)]
    #[test]
    fn the_other_hard_link_is_a_different_container_once_one_has_been_written_back() {
        // §7 says a hard link to the original keeps pointing at the original,
        // which now holds the old contents. So after a write-back through one
        // name the two really are different files, and opening the other is a
        // new session rather than a match. Correct rather than accidental: the
        // path differs and so does the identity.
        let tmp = tempfile::tempdir().unwrap();
        let a = a_file(tmp.path(), "a.slpc");
        let b = tmp.path().join("b.slpc");
        fs::hard_link(&a, &b).unwrap();

        let mut table = Table::new();
        table.insert(&a, "session".to_string()).unwrap();

        let scratch = tmp.path().join("scratch");
        fs::write(&scratch, b"repacked").unwrap();
        fs::rename(&scratch, &a).unwrap();
        table.refresh(&a);

        assert!(table.find_mut(&a).is_some());
        assert!(
            table.find_mut(&b).is_none(),
            "the other link still holds the old contents and is its own container now"
        );
    }

    #[test]
    fn refreshing_keeps_the_identity_arm_working_after_a_save() {
        let tmp = tempfile::tempdir().unwrap();
        let c = a_file(tmp.path(), "report.slpc");
        let mut table = Table::new();
        table.insert(&c, "session".to_string()).unwrap();

        let scratch = tmp.path().join("scratch");
        fs::write(&scratch, b"repacked").unwrap();
        fs::rename(&scratch, &c).unwrap();
        table.refresh(&c);

        // A fresh hard link to what the container is *now* finds the session,
        // which it would not if the entry still held the identity from open.
        #[cfg(unix)]
        {
            let link = tmp.path().join("link.slpc");
            fs::hard_link(&c, &link).unwrap();
            assert!(table.find_mut(&link).is_some());
        }
        assert!(table.find_mut(&c).is_some());
    }

    #[test]
    fn a_container_that_is_not_there_matches_nothing_rather_than_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let c = a_file(tmp.path(), "report.slpc");
        let mut table = Table::new();
        table.insert(&c, "session".to_string()).unwrap();
        assert!(table.find_mut(&tmp.path().join("gone.slpc")).is_none());
    }

    #[test]
    fn removing_hands_the_session_back_and_empties_the_table() {
        let tmp = tempfile::tempdir().unwrap();
        let c = a_file(tmp.path(), "report.slpc");
        let mut table = Table::new();
        table.insert(&c, "session".to_string()).unwrap();
        assert_eq!(table.remove(&c), Some("session".to_string()));
        assert!(table.is_empty());
        assert!(table.remove(&c).is_none());
    }
}
