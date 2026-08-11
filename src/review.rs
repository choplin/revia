//! Pure in-memory review-session state.
//!
//! A session owns the diff snapshot currently under review and the cursor that
//! identifies a file, hunk, and inline thread within it. It has no terminal,
//! Git-command, or persistence dependency; the application layer supplies new
//! snapshots and persists thread changes separately.

use crate::{
    anchor::HunkLocation,
    diff::{DiffFile, LoadedDiff},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewCursor {
    selected_file: usize,
    selected_hunk: usize,
    selected_thread: usize,
}

impl ReviewCursor {
    pub fn selected_file(self) -> usize {
        self.selected_file
    }

    pub fn selected_hunk(self) -> usize {
        self.selected_hunk
    }

    pub fn selected_thread(self) -> usize {
        self.selected_thread
    }
}

#[derive(Debug, Clone)]
pub struct ReviewSession {
    diff: LoadedDiff,
    cursor: ReviewCursor,
}

impl ReviewSession {
    pub fn new(diff: LoadedDiff) -> Self {
        Self {
            diff,
            cursor: ReviewCursor::default(),
        }
    }

    pub fn diff(&self) -> &LoadedDiff {
        &self.diff
    }

    pub fn cursor(&self) -> ReviewCursor {
        self.cursor
    }

    pub fn file(&self) -> Option<&DiffFile> {
        self.diff.document.files.get(self.cursor.selected_file)
    }

    pub fn selected_location(&self) -> Option<HunkLocation> {
        let file = self.file()?;
        let hunk = file.hunks.get(self.cursor.selected_hunk)?;
        Some(HunkLocation::new(&file.path, &hunk.header))
    }

    pub fn replace_diff(&mut self, diff: LoadedDiff) {
        let previous_file_index = self.cursor.selected_file;
        let previous_hunk_index = self.cursor.selected_hunk;
        let previous_file = self.file().map(|file| file.path.clone());
        let previous_hunk = self
            .file()
            .and_then(|file| file.hunks.get(self.cursor.selected_hunk))
            .map(|hunk| (hunk.header.clone(), hunk.coordinates));
        self.diff = diff;
        let restored_file = previous_file.as_ref().and_then(|path| {
            self.diff
                .document
                .files
                .iter()
                .position(|file| &file.path == path)
        });
        self.cursor.selected_file = restored_file.unwrap_or_else(|| {
            previous_file_index.min(self.diff.document.files.len().saturating_sub(1))
        });
        self.cursor.selected_hunk = self.file().map_or(0, |file| {
            if restored_file.is_none() {
                return previous_hunk_index.min(file.hunks.len().saturating_sub(1));
            }
            let Some((header, coordinates)) = &previous_hunk else {
                return previous_hunk_index.min(file.hunks.len().saturating_sub(1));
            };
            file.hunks
                .iter()
                .position(|hunk| hunk.header == *header)
                .or_else(|| {
                    coordinates.and_then(|coordinates| {
                        file.hunks
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, hunk)| {
                                hunk.coordinates.map_or(usize::MAX, |candidate| {
                                    candidate
                                        .old
                                        .start
                                        .abs_diff(coordinates.old.start)
                                        .saturating_add(
                                            candidate.new.start.abs_diff(coordinates.new.start),
                                        )
                                })
                            })
                            .map(|(index, _)| index)
                    })
                })
                .unwrap_or_else(|| previous_hunk_index.min(file.hunks.len().saturating_sub(1)))
        });
        self.cursor.selected_thread = 0;
    }

    pub fn select_file(&mut self, file: usize) {
        self.cursor.selected_file = file;
        self.cursor.selected_hunk = 0;
        self.cursor.selected_thread = 0;
    }

    pub fn move_file(&mut self, direction: i32) -> bool {
        let count = self.diff.document.files.len();
        if count == 0 {
            return false;
        }
        self.select_file(wrapped_index(self.cursor.selected_file, count, direction));
        true
    }

    pub fn select_hunk(&mut self, file: usize, hunk: usize) {
        self.cursor.selected_file = file;
        self.cursor.selected_hunk = hunk;
        self.cursor.selected_thread = 0;
    }

    pub fn move_hunk(&mut self, direction: i32) -> bool {
        let locations = self.hunk_indices();
        if locations.is_empty() {
            return false;
        }
        let target = locations
            .iter()
            .position(|(file, hunk)| {
                *file == self.cursor.selected_file && *hunk == self.cursor.selected_hunk
            })
            .map(|current| wrapped_index(current, locations.len(), direction))
            .unwrap_or_else(|| {
                if direction < 0 {
                    locations
                        .iter()
                        .rposition(|(file, _)| *file < self.cursor.selected_file)
                        .unwrap_or(locations.len() - 1)
                } else {
                    locations
                        .iter()
                        .position(|(file, _)| *file > self.cursor.selected_file)
                        .unwrap_or(0)
                }
            });
        let (file, hunk) = locations[target];
        self.select_hunk(file, hunk);
        true
    }

    pub fn select_thread(&mut self, thread: usize) {
        self.cursor.selected_thread = thread;
    }

    fn hunk_indices(&self) -> Vec<(usize, usize)> {
        self.diff
            .document
            .files
            .iter()
            .enumerate()
            .flat_map(|(file_index, file)| {
                (0..file.hunks.len()).map(move |hunk_index| (file_index, hunk_index))
            })
            .collect()
    }
}

fn wrapped_index(current: usize, length: usize, direction: i32) -> usize {
    ((current as i32 + direction).rem_euclid(length as i32)) as usize
}

#[cfg(test)]
mod tests {
    use crate::diff::{DiffDocument, LoadedDiff};

    use super::ReviewSession;

    fn session(text: &str) -> ReviewSession {
        ReviewSession::new(LoadedDiff {
            text: text.into(),
            document: DiffDocument::parse(text),
        })
    }

    const TWO_FILES: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n@@ -3 +3 @@\n-c\n+d\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-e\n+f\n";

    #[test]
    fn selection_transitions_reset_dependent_targets() {
        let mut session = session(TWO_FILES);
        session.select_hunk(0, 1);
        session.select_thread(4);
        session.move_file(1);
        assert_eq!(
            (
                session.cursor().selected_file(),
                session.cursor().selected_hunk(),
                session.cursor().selected_thread()
            ),
            (1, 0, 0)
        );
        session.select_thread(3);
        session.move_hunk(-1);
        assert_eq!(
            (
                session.cursor().selected_file(),
                session.cursor().selected_hunk(),
                session.cursor().selected_thread()
            ),
            (0, 1, 0)
        );
    }

    #[test]
    fn replacement_clamps_selection_to_the_loaded_snapshot() {
        let mut session = session(TWO_FILES);
        session.select_hunk(1, 0);
        session.select_thread(2);
        session.replace_diff(LoadedDiff {
            text: String::new(),
            document: DiffDocument::default(),
        });
        assert_eq!(
            (
                session.cursor().selected_file(),
                session.cursor().selected_hunk(),
                session.cursor().selected_thread()
            ),
            (0, 0, 0)
        );
        assert!(session.selected_location().is_none());
    }

    #[test]
    fn replacement_preserves_file_identity_and_selects_the_closest_changed_hunk() {
        let original = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2 +2 @@\n-c\n+d\n@@ -20 +20 @@\n-e\n+f\n";
        let reloaded = "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -5 +5 @@\n-c\n+d\n@@ -22 +22 @@\n-e\n+f\ndiff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n";
        let mut session = session(original);
        session.select_hunk(1, 1);

        session.replace_diff(LoadedDiff {
            text: reloaded.into(),
            document: DiffDocument::parse(reloaded),
        });

        assert_eq!(session.cursor().selected_file(), 0);
        assert_eq!(session.cursor().selected_hunk(), 1);
        assert_eq!(
            session.selected_location(),
            Some(crate::anchor::HunkLocation::new("b.rs", "@@ -22 +22 @@"))
        );
    }

    #[test]
    fn replacement_does_not_apply_a_missing_files_hunk_identity_to_another_file() {
        let original = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -20 +20 @@\n-c\n+d\n";
        let reloaded = "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -1 +1 @@\n-a\n+b\n@@ -20 +20 @@\n-c\n+d\n";
        let mut session = session(original);
        session.select_hunk(1, 0);

        session.replace_diff(LoadedDiff {
            text: reloaded.into(),
            document: DiffDocument::parse(reloaded),
        });

        assert_eq!(session.cursor().selected_file(), 0);
        assert_eq!(session.cursor().selected_hunk(), 0);
        assert_eq!(
            session.selected_location(),
            Some(crate::anchor::HunkLocation::new("c.rs", "@@ -1 +1 @@"))
        );
    }

    #[test]
    fn hunk_traversal_can_leave_a_changed_file_without_hunks() {
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/image.bin b/image.bin\nBinary files a/image.bin and b/image.bin differ\ndiff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -3 +3 @@\n-c\n+d\n";
        let mut session = session(raw);
        session.select_file(1);

        assert!(session.move_hunk(1));
        assert_eq!(session.cursor().selected_file(), 2);
        session.select_file(1);
        assert!(session.move_hunk(-1));
        assert_eq!(session.cursor().selected_file(), 0);
    }
}
