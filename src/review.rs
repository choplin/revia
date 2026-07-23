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
        self.diff = diff;
        self.cursor.selected_file = self
            .cursor
            .selected_file
            .min(self.diff.document.files.len().saturating_sub(1));
        self.cursor.selected_hunk = self.file().map_or(0, |file| {
            self.cursor
                .selected_hunk
                .min(file.hunks.len().saturating_sub(1))
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
        let Some(current) = locations.iter().position(|(file, hunk)| {
            *file == self.cursor.selected_file && *hunk == self.cursor.selected_hunk
        }) else {
            return false;
        };
        let (file, hunk) = locations[wrapped_index(current, locations.len(), direction)];
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
}
