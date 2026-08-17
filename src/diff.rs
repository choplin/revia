use std::{path::Path, process::Command, sync::Arc};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    WorkingTree,
    Staged,
    Commit(String),
    Range(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    pub target: DiffTarget,
    pub context_lines: usize,
}

impl DiffRequest {
    pub fn git_arguments(&self) -> Vec<String> {
        let mut arguments = vec!["diff".into(), "--no-ext-diff".into()];
        arguments.push(format!("--unified={}", self.context_lines));

        match &self.target {
            DiffTarget::WorkingTree => {}
            DiffTarget::Staged => arguments.push("--cached".into()),
            DiffTarget::Commit(revision) => {
                arguments[0] = "show".into();
                arguments.extend(["--format=".into(), revision.clone()]);
            }
            DiffTarget::Range(range) => arguments.push(range.clone()),
        }

        arguments
    }
}

impl DiffTarget {
    pub fn description(&self) -> String {
        match self {
            Self::WorkingTree => "working tree".into(),
            Self::Staged => "staged changes".into(),
            Self::Commit(revision) => format!("commit {revision}"),
            Self::Range(range) => format!("range {range}"),
        }
    }

    /// The comparison identity shown in the changeset header.
    ///
    /// This answers "which comparison am I reviewing" in the shortest form the
    /// user themselves selected, so a revision range stays recognisable instead
    /// of being restated as prose.
    pub fn comparison(&self) -> String {
        match self {
            Self::WorkingTree => "working tree".into(),
            Self::Staged => "staged".into(),
            Self::Commit(revision) => revision.clone(),
            Self::Range(range) => range.clone(),
        }
    }
}

/// Additions and deletions counted from the patch body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Magnitude {
    pub additions: usize,
    pub deletions: usize,
}

impl Magnitude {
    pub fn combined(self, other: Self) -> Self {
        Self {
            additions: self.additions.saturating_add(other.additions),
            deletions: self.deletions.saturating_add(other.deletions),
        }
    }

    fn count(&mut self, kind: DiffLineKind) {
        match kind {
            DiffLineKind::Added => self.additions = self.additions.saturating_add(1),
            DiffLineKind::Removed => self.deletions = self.deletions.saturating_add(1),
            DiffLineKind::Context | DiffLineKind::Meta => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDiff {
    pub text: String,
    pub document: DiffDocument,
}

impl LoadedDiff {
    pub fn load(repository: &Path, request: &DiffRequest) -> Result<Self> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(request.git_arguments())
            .output()
            .with_context(|| format!("could not run git in {}", repository.display()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "git could not load the selected {} diff: {}",
                request.target.description(),
                stderr.trim()
            );
        }

        let text = String::from_utf8(output.stdout).context("git produced a non-UTF-8 diff")?;
        Ok(Self {
            document: DiffDocument::parse(&text),
            text,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffDocument {
    pub files: Vec<DiffFile>,
}

impl DiffDocument {
    pub fn parse(text: &str) -> Self {
        let mut files = Vec::new();
        let mut current: Option<DiffFile> = None;

        for line in text.lines() {
            if line.starts_with("diff --git ") {
                if let Some(file) = current.take() {
                    files.push(file);
                }
                current = Some(DiffFile::from_header(line));
                continue;
            }

            let Some(file) = current.as_mut() else {
                continue;
            };

            if line.starts_with("@@") {
                file.hunks.push(DiffHunk {
                    header: line.to_owned(),
                    coordinates: HunkCoordinates::parse(line),
                    lines: Arc::new(Vec::new()),
                });
            } else if !file.absorb_source_line(line) {
                file.absorb_metadata(line);
            }
        }

        if let Some(file) = current {
            files.push(file);
        }

        Self { files }
    }

    /// Total additions and deletions across every file in the changeset.
    pub fn magnitude(&self) -> Magnitude {
        self.files.iter().fold(Magnitude::default(), |total, file| {
            total.combined(file.magnitude)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub change: FileChange,
    pub magnitude: Magnitude,
    pub hunks: Vec<DiffHunk>,
}

impl DiffFile {
    fn from_header(header: &str) -> Self {
        let paths = header
            .strip_prefix("diff --git ")
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>();
        let path = paths
            .get(1)
            .or_else(|| paths.first())
            .map(|path| strip_git_prefix(path))
            .unwrap_or_else(|| "(unknown file)".to_owned());
        let previous_path = paths.first().map(|path| strip_git_prefix(path));

        Self {
            path,
            previous_path,
            change: FileChange::default(),
            magnitude: Magnitude::default(),
            hunks: Vec::new(),
        }
    }

    fn absorb_source_line(&mut self, line: &str) -> bool {
        let parsed = DiffLine::from_raw(line);
        let kind = parsed.kind;
        let Some(hunk) = self.hunks.last_mut() else {
            return false;
        };
        Arc::make_mut(&mut hunk.lines).push(parsed);
        self.magnitude.count(kind);
        true
    }

    fn absorb_metadata(&mut self, line: &str) {
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = normalize_patch_path(path);
            if path != "/dev/null" {
                self.path = path;
            }
        } else if let Some(path) = line.strip_prefix("--- ") {
            self.previous_path = Some(normalize_patch_path(path));
        }
        self.change.absorb(line);
    }

    pub fn extension(&self) -> Option<&str> {
        Path::new(&self.path)
            .extension()
            .and_then(|extension| extension.to_str())
    }
}

/// What the patch header says happened to a file, independent of its content.
///
/// Git states this through transport plumbing that reviewers should not have to
/// read. Parsing it once here lets every surface present the review fact
/// (renamed, deleted, binary, mode changed) instead of the raw header rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChange {
    pub status: FileStatus,
    pub moved: Option<PathMove>,
    pub mode: Option<ModeChange>,
    pub binary: bool,
}

impl FileChange {
    fn absorb(&mut self, line: &str) {
        if line.starts_with("new file mode ") {
            self.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode ") {
            self.status = FileStatus::Deleted;
        } else if let Some(mode) = line.strip_prefix("old mode ") {
            self.mode.get_or_insert_with(ModeChange::default).old = mode.trim().to_owned();
        } else if let Some(mode) = line.strip_prefix("new mode ") {
            self.mode.get_or_insert_with(ModeChange::default).new = mode.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("similarity index ") {
            let similarity = value.trim().trim_end_matches('%').parse().ok();
            self.moved.get_or_insert_with(PathMove::default).similarity = similarity;
        } else if line.starts_with("rename from ") || line.starts_with("rename to ") {
            self.moved.get_or_insert_with(PathMove::default);
        } else if line.starts_with("copy from ") || line.starts_with("copy to ") {
            self.moved.get_or_insert_with(PathMove::default).copied = true;
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            self.binary = true;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileStatus {
    #[default]
    Modified,
    Added,
    Deleted,
}

/// A rename or copy. The old and new paths already live on the `DiffFile`, so
/// only the facts Git states separately are recorded here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathMove {
    pub copied: bool,
    pub similarity: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModeChange {
    pub old: String,
    pub new: String,
}

fn strip_git_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_owned()
}

fn normalize_patch_path(path: &str) -> String {
    if path == "/dev/null" {
        return path.to_owned();
    }
    strip_git_prefix(path.split('\t').next().unwrap_or(path))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub coordinates: Option<HunkCoordinates>,
    pub lines: Arc<Vec<DiffLine>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkCoordinates {
    pub old: HunkRange,
    pub new: HunkRange,
}

impl HunkCoordinates {
    pub(crate) fn parse(header: &str) -> Option<Self> {
        let mut fields = header.strip_prefix("@@ ")?.split_whitespace();
        Some(Self {
            old: HunkRange::parse(fields.next()?, '-')?,
            new: HunkRange::parse(fields.next()?, '+')?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    pub start: usize,
    pub count: usize,
}

impl HunkRange {
    fn parse(value: &str, prefix: char) -> Option<Self> {
        let value = value.strip_prefix(prefix)?;
        let (start, count) = value.split_once(',').unwrap_or((value, "1"));
        Some(Self {
            start: start.parse().ok()?,
            count: count.parse().ok()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

impl DiffLine {
    fn from_raw(line: &str) -> Self {
        let (kind, text) = match line.as_bytes().first() {
            Some(b'+') => (DiffLineKind::Added, &line[1..]),
            Some(b'-') => (DiffLineKind::Removed, &line[1..]),
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            _ => (DiffLineKind::Meta, line),
        };
        Self {
            kind,
            text: text.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Added,
    Removed,
    Context,
    Meta,
}

/// The checked-in edge fixture from the diff-viewer presentation baseline.
///
/// It is real `git diff` output so tests exercise rename, mode, binary,
/// deleted, new-file, missing-newline, and repeated-filename cases without
/// mocking the parser's input.
#[cfg(test)]
pub(crate) const EDGE_FIXTURE: &str =
    include_str!("../tests/fixtures/diff_viewer_baseline_edges.patch");

#[cfg(test)]
mod tests {
    use super::{
        DiffDocument, DiffLineKind, DiffRequest, DiffTarget, EDGE_FIXTURE, FileStatus,
        HunkCoordinates, HunkRange, Magnitude, ModeChange, PathMove,
    };

    #[test]
    fn working_tree_uses_git_diff_with_requested_context() {
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 7,
        };

        assert_eq!(
            request.git_arguments(),
            ["diff", "--no-ext-diff", "--unified=7"]
        );
    }

    #[test]
    fn staged_and_revision_targets_preserve_git_fidelity() {
        let staged = DiffRequest {
            target: DiffTarget::Staged,
            context_lines: 3,
        };
        assert_eq!(
            staged.git_arguments(),
            ["diff", "--no-ext-diff", "--unified=3", "--cached"]
        );

        let commit = DiffRequest {
            target: DiffTarget::Commit("HEAD".into()),
            context_lines: 3,
        };
        assert_eq!(
            commit.git_arguments(),
            ["show", "--no-ext-diff", "--unified=3", "--format=", "HEAD"]
        );
    }

    #[test]
    fn target_descriptions_identify_every_selectable_diff() {
        assert_eq!(DiffTarget::WorkingTree.description(), "working tree");
        assert_eq!(DiffTarget::Staged.description(), "staged changes");
        assert_eq!(
            DiffTarget::Commit("abc123".into()).description(),
            "commit abc123"
        );
        assert_eq!(
            DiffTarget::Range("main...HEAD".into()).description(),
            "range main...HEAD"
        );
    }

    #[test]
    fn parses_files_hunks_and_line_kinds() {
        let document = DiffDocument::parse(
            "diff --git a/src/lib.rs b/src/lib.rs\nindex 111..222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n old\n+new\n\\ No newline at end of file\ndiff --git a/readme.md b/readme.md\nnew file mode 100644\n--- /dev/null\n+++ b/readme.md\n@@ -0,0 +1 @@\n+hello\n",
        );

        assert_eq!(document.files.len(), 2);
        assert_eq!(document.files[0].path, "src/lib.rs");
        assert_eq!(document.files[0].hunks.len(), 1);
        assert_eq!(
            document.files[0].hunks[0].lines[1].kind,
            DiffLineKind::Added
        );
        assert_eq!(document.files[1].path, "readme.md");
        assert_eq!(
            document.files[0].hunks[0].coordinates,
            Some(HunkCoordinates {
                old: HunkRange { start: 1, count: 1 },
                new: HunkRange { start: 1, count: 2 },
            })
        );
        assert_eq!(
            document.files[1].hunks[0].coordinates,
            Some(HunkCoordinates {
                old: HunkRange { start: 0, count: 0 },
                new: HunkRange { start: 1, count: 1 },
            })
        );
    }

    #[test]
    fn deleted_files_keep_their_unique_repository_paths() {
        let document = DiffDocument::parse(
            "diff --git a/old-a.rs b/old-a.rs\ndeleted file mode 100644\n--- a/old-a.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old a\ndiff --git a/old-b.rs b/old-b.rs\ndeleted file mode 100644\n--- a/old-b.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old b\n",
        );

        assert_eq!(document.files[0].path, "old-a.rs");
        assert_eq!(document.files[1].path, "old-b.rs");
    }

    #[test]
    fn counts_per_file_and_changeset_magnitude_from_the_patch_body() {
        let document = DiffDocument::parse(EDGE_FIXTURE);
        let magnitude = |path: &str| {
            document
                .files
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("{path} is part of the edge fixture"))
                .magnitude
        };

        assert_eq!(magnitude("src/lib.rs").additions, 2);
        assert_eq!(magnitude("src/lib.rs").deletions, 2);
        assert_eq!(magnitude("docs/added.md").additions, 1);
        assert_eq!(magnitude("docs/added.md").deletions, 0);
        assert_eq!(magnitude("docs/removed.md").deletions, 1);
        // A metadata-only change carries no source lines at all.
        assert_eq!(magnitude("scripts/review.sh"), Magnitude::default());
        assert_eq!(magnitude("assets/logo.png"), Magnitude::default());
        assert_eq!(
            document.magnitude(),
            Magnitude {
                additions: 6,
                deletions: 4,
            }
        );
    }

    #[test]
    fn classifies_semantic_file_changes_without_retaining_transport_headers() {
        let document = DiffDocument::parse(EDGE_FIXTURE);
        let change = |path: &str| {
            document
                .files
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("{path} is part of the edge fixture"))
                .change
                .clone()
        };

        assert_eq!(change("docs/added.md").status, FileStatus::Added);
        assert_eq!(change("docs/removed.md").status, FileStatus::Deleted);
        assert_eq!(change("src/lib.rs").status, FileStatus::Modified);
        assert!(change("assets/logo.png").binary);
        assert!(!change("src/lib.rs").binary);
        assert_eq!(
            change("scripts/review.sh").mode,
            Some(ModeChange {
                old: "100644".into(),
                new: "100755".into(),
            })
        );
        assert_eq!(
            change("src/new_name.rs").moved,
            Some(PathMove {
                copied: false,
                similarity: Some(92),
            })
        );

        let renamed = document
            .files
            .iter()
            .find(|file| file.path == "src/new_name.rs")
            .expect("the renamed file is part of the edge fixture");
        assert_eq!(renamed.previous_path.as_deref(), Some("src/old_name.rs"));
    }
}
