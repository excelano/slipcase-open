//! Watching the payload directory, and what the events in it mean.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! **The watch is on the directory and never on the file**, which concept 6
//! calls one of the three things that make write-back detection hard. A serious
//! editor saves by writing a temporary sibling and renaming it over the target,
//! so a watcher registered on the payload loses its handle on the first save
//! and never fires again. `notify` will watch a directory on all three
//! platforms, but only if it is asked to.
//!
//! ## The sibling signal
//!
//! Concept 6.1: the payload directory holds one file, put there by this tool,
//! so anything else appearing in it was created by the target application — a
//! lock file, an autosave, a backup, a save in progress. Nothing here needs to
//! know which, or what any of them are called, which is why there is no table
//! of `~$name.docx` and `.~lock.name#` conventions to maintain and no
//! application it fails to know about.
//!
//! Siblings present means the application is working in there, which process
//! exit does not tell you. Siblings gone means it has cleaned up and has
//! probably finished. It stays a heuristic in both directions: most
//! read-oriented applications write no sibling at all, so an empty directory
//! means nothing, and an application that leaves a backup behind for good never
//! produces the cleaned-up signal. Both degrade to silence, which is the
//! intended fallback and is why the session model does not rest on this.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// What happened in a payload directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The payload itself was written, replaced, or removed. The write-back
    /// trigger.
    Payload,
    /// Something the target application made appeared beside it.
    SiblingAppeared,
    /// Something it had made went away.
    SiblingWentAway,
}

/// What an event in the payload directory means, given the payload's name.
///
/// Pure, so the rule is testable without a filesystem or a race. Paths are
/// compared by their final component: `notify` reports absolute paths, and a
/// rename within the directory arrives as paths that differ only there.
///
/// A rename over the payload produces events naming both the temporary sibling
/// and the payload, and both are worth reporting — the first says the
/// application is working, the second is the save.
#[must_use]
pub fn classify(payload: &str, paths: &[&Path], kind: EventKind) -> Vec<Change> {
    paths
        .iter()
        .map(|p| {
            let is_payload = p.file_name().is_some_and(|n| n == payload);
            match (is_payload, kind) {
                (true, _) => Change::Payload,
                (false, EventKind::Gone) => Change::SiblingWentAway,
                (false, _) => Change::SiblingAppeared,
            }
        })
        .collect()
}

/// The shape of an event, reduced to what the rule above needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Created, written, or renamed into place.
    Touched,
    /// Removed, or renamed away.
    Gone,
}

impl EventKind {
    /// Reduce one of `notify`'s events, or discard it.
    ///
    /// **Reading a file is not changing it.** A plain read of the watched
    /// payload emits `Access(Open(Any))` on Linux — measured on 2026-08-30 —
    /// and treating that as a save makes every reader of the payload a source
    /// of spurious write-backs. Anything that reads it counts: the write-back
    /// itself opens the payload, and so does `recover::state`, which is called
    /// once per session by `sessions`. Running `sessions` in a loop beside an
    /// open session produced a repack per poll before this arm existed.
    ///
    /// The repacks are invisible from outside, which is why this is a rule and
    /// a test rather than something anyone would notice: each one writes the
    /// same bytes, so the container stays right and only the work is wrong.
    ///
    /// The exception is a close after writing, which is the one access event
    /// that means a save finished. Linux reports it and the other platforms do
    /// not, so it is a signal to take where it is offered rather than one to
    /// depend on.
    ///
    /// `Any` and `Other` stay a touch. An unrecognised event in this directory
    /// is still something happening in it, and the cost of treating one as a
    /// save is a repack that writes what is already there.
    fn of(kind: notify::EventKind) -> Option<Self> {
        use notify::event::{AccessKind, AccessMode, ModifyKind, RenameMode};
        match kind {
            // A removal, and the half of a rename that names where the file
            // was. The other half names where it went, which is a touch.
            notify::EventKind::Remove(_)
            | notify::EventKind::Modify(ModifyKind::Name(RenameMode::From)) => Some(Self::Gone),
            notify::EventKind::Access(AccessKind::Close(AccessMode::Write)) => Some(Self::Touched),
            notify::EventKind::Access(_) => None,
            _ => Some(Self::Touched),
        }
    }
}

/// A watch on one payload directory.
///
/// Holds the platform watcher, which stops when this is dropped.
pub struct Watch {
    _watcher: RecommendedWatcher,
    changes: Receiver<Change>,
}

impl Watch {
    /// Watch `dir` for changes to `payload` and to anything beside it.
    ///
    /// Non-recursive: the payload directory has no subdirectories of this
    /// tool's making, and an application that creates one has still created a
    /// sibling, which is the signal either way.
    ///
    /// # Errors
    ///
    /// Where the platform watcher cannot be created or cannot watch `dir`.
    pub fn on(dir: &Path, payload: &str) -> notify::Result<Self> {
        let (tx, changes) = mpsc::channel();
        let payload = payload.to_string();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else {
                    // A dropped or errored event is not a reason to tear down the
                    // watch. Concept 6.2 exists because detection is unreliable,
                    // and the session close is the backstop for everything this
                    // misses.
                    return;
                };
                let Some(kind) = EventKind::of(event.kind) else {
                    return;
                };
                let paths: Vec<&Path> = event.paths.iter().map(AsRef::as_ref).collect();
                for change in classify(&payload, &paths, kind) {
                    // A closed receiver means the session is gone and there is
                    // nobody to tell.
                    if tx.send(change).is_err() {
                        return;
                    }
                }
            })?;
        watcher.watch(dir, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _watcher: watcher,
            changes,
        })
    }

    /// Every change that has arrived, without waiting.
    pub fn drain(&self) -> impl Iterator<Item = Change> + '_ {
        self.changes.try_iter()
    }

    /// Wait up to `within` for the next change.
    #[must_use]
    pub fn next_change(&self, within: Duration) -> Option<Change> {
        self.changes.recv_timeout(within).ok()
    }
}

/// Whether the target application has anything of its own in the payload
/// directory.
///
/// Asked of the directory rather than tracked from events, because events can
/// be missed and the answer has to be right at the moment somebody is deciding
/// whether to close a session (concept 6.2).
///
/// # Errors
///
/// Where the directory cannot be read.
pub fn siblings_present(dir: &Path, payload: &str) -> std::io::Result<bool> {
    for entry in std::fs::read_dir(dir)? {
        if entry?.file_name() != *payload {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::{classify, siblings_present, Change, EventKind, Watch};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn at(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| Path::new("/s/payload").join(n))
            .collect()
    }

    fn refs(paths: &[PathBuf]) -> Vec<&Path> {
        paths.iter().map(AsRef::as_ref).collect()
    }

    #[test]
    fn writing_the_payload_is_the_write_back_trigger() {
        let p = at(&["report.pdf"]);
        assert_eq!(
            classify("report.pdf", &refs(&p), EventKind::Touched),
            [Change::Payload]
        );
    }

    #[test]
    fn anything_else_appearing_is_the_application_working() {
        // No table of lock file conventions. The directory held one file, this
        // tool put it there, so whatever this is came from the editor.
        for name in [
            "~$report.docx",
            ".~lock.report.pdf#",
            "report.pdf.tmp",
            "4919",
        ] {
            let p = at(&[name]);
            assert_eq!(
                classify("report.pdf", &refs(&p), EventKind::Touched),
                [Change::SiblingAppeared],
                "{name}"
            );
        }
    }

    #[test]
    fn a_sibling_going_away_is_the_application_finishing() {
        let p = at(&["~$report.docx"]);
        assert_eq!(
            classify("report.pdf", &refs(&p), EventKind::Gone),
            [Change::SiblingWentAway]
        );
    }

    #[test]
    fn the_payload_going_away_is_still_the_payload() {
        // A rename over it arrives as the payload being replaced, and an
        // application that deletes and rewrites is doing a save in two steps.
        // Either way the container should be asked to catch up.
        let p = at(&["report.pdf"]);
        assert_eq!(
            classify("report.pdf", &refs(&p), EventKind::Gone),
            [Change::Payload]
        );
    }

    #[test]
    fn a_rename_naming_both_paths_reports_both() {
        // The atomic save: a temporary sibling renamed over the target. The
        // sibling says the application is working and the payload is the save,
        // and dropping either would lose one of the two things the watch is for.
        let p = at(&["report.pdf.tmp", "report.pdf"]);
        assert_eq!(
            classify("report.pdf", &refs(&p), EventKind::Touched),
            [Change::SiblingAppeared, Change::Payload]
        );
    }

    #[test]
    fn a_payload_named_like_a_lock_file_is_still_the_payload() {
        // SPEC 2.3 permits any plain filename. Matching by name and not by
        // shape is what keeps this true.
        let p = at(&["~$report.docx"]);
        assert_eq!(
            classify("~$report.docx", &refs(&p), EventKind::Touched),
            [Change::Payload]
        );
    }

    #[test]
    fn siblings_are_asked_of_the_directory_rather_than_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("report.pdf"), b"x").unwrap();
        assert!(!siblings_present(tmp.path(), "report.pdf").unwrap());

        std::fs::write(tmp.path().join("~$report.pdf"), b"").unwrap();
        assert!(siblings_present(tmp.path(), "report.pdf").unwrap());

        std::fs::remove_file(tmp.path().join("~$report.pdf")).unwrap();
        assert!(!siblings_present(tmp.path(), "report.pdf").unwrap());
    }

    #[test]
    fn reading_the_payload_is_not_a_change_to_it() {
        // Write-back opens the payload to read it, which inotify reports as an
        // access on the watched file. Treating that as a save makes the
        // write-back its own trigger: measured on 2026-08-30, one edit produced
        // three repacks and would have produced more had the session stayed
        // open.
        let tmp = tempfile::tempdir().unwrap();
        let payload = tmp.path().join("report.pdf");
        std::fs::write(&payload, b"first").unwrap();

        let watch = Watch::on(tmp.path(), "report.pdf").unwrap();
        let _ = std::fs::read(&payload).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut seen = Vec::new();
        while std::time::Instant::now() < deadline {
            if let Some(c) = watch.next_change(Duration::from_millis(100)) {
                seen.push(c);
            }
        }
        assert!(
            !seen.contains(&Change::Payload),
            "reading the payload was reported as a change: {seen:?}"
        );
    }

    #[test]
    fn a_real_atomic_save_reaches_the_watch() {
        // The one test that goes through the platform. It saves the way a
        // serious editor does — write a temporary sibling, rename over the
        // target — which is the case a watch registered on the file would miss
        // entirely.
        let tmp = tempfile::tempdir().unwrap();
        let payload = tmp.path().join("report.pdf");
        std::fs::write(&payload, b"first").unwrap();

        let watch = Watch::on(tmp.path(), "report.pdf").unwrap();

        let scratch = tmp.path().join("report.pdf.tmp");
        std::fs::write(&scratch, b"second").unwrap();
        std::fs::rename(&scratch, &payload).unwrap();

        // Generously, because this is at the platform's pace and not ours.
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !seen.contains(&Change::Payload) {
            if let Some(c) = watch.next_change(Duration::from_millis(250)) {
                seen.push(c);
            }
        }
        assert!(
            seen.contains(&Change::Payload),
            "the save never arrived: {seen:?}"
        );
        assert_eq!(std::fs::read(&payload).unwrap(), b"second");
    }
}
