use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    Git(GitComparison),
    Patch(PatchInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitComparison {
    Changes,
    Staged,
    Unstaged,
    Revision(String),
    Range(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchInput {
    File(PathBuf),
    Stdin,
}

pub fn split_revision_range(value: &str) -> Option<(&str, &'static str, &str)> {
    let bytes = value.as_bytes();
    let mut separator = None;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'.' {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index] == b'.' {
            index += 1;
        }
        let length = index - start;
        if length >= 2 {
            if separator.is_some() || !matches!(length, 2 | 3) {
                return None;
            }
            separator = Some((start, length));
        }
    }
    let (start, length) = separator?;
    let left = &value[..start];
    let right = &value[start + length..];
    (!left.is_empty() && !right.is_empty()).then_some((
        left,
        if length == 2 { ".." } else { "..." },
        right,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    pub source: DiffSource,
    pub context_lines: usize,
}

impl DiffSource {
    pub fn description(&self) -> String {
        match self {
            Self::Git(comparison) => comparison.description(),
            Self::Patch(PatchInput::File(path)) => format!("patch {}", path.display()),
            Self::Patch(PatchInput::Stdin) => "patch from stdin".into(),
        }
    }

    /// The comparison identity shown in the changeset header.
    ///
    /// This answers "which comparison am I reviewing" in the shortest form the
    /// user themselves selected, so a revision range stays recognisable instead
    /// of being restated as prose.
    pub fn comparison(&self) -> String {
        match self {
            Self::Git(comparison) => comparison.comparison(),
            Self::Patch(PatchInput::File(path)) => path.display().to_string(),
            Self::Patch(PatchInput::Stdin) => "stdin".into(),
        }
    }

    pub fn supports_persistent_threads(&self) -> bool {
        matches!(self, Self::Git(_))
    }

    pub fn persistent_threads_unavailable_reason(&self) -> Option<&'static str> {
        (!self.supports_persistent_threads()).then_some(
            "persistent thread operations are unavailable for patch input because it has no immutable Git provenance",
        )
    }
}

impl GitComparison {
    fn description(&self) -> String {
        match self {
            Self::Changes => "changes".into(),
            Self::Staged => "staged changes".into(),
            Self::Unstaged => "unstaged changes".into(),
            Self::Revision(revision) => format!("revision {revision}"),
            Self::Range(range) => format!("range {range}"),
        }
    }

    fn comparison(&self) -> String {
        match self {
            Self::Changes => "changes".into(),
            Self::Staged => "staged".into(),
            Self::Unstaged => "unstaged".into(),
            Self::Revision(revision) => revision.clone(),
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
    pub text: Arc<str>,
    pub document: Arc<DiffDocument>,
    pub provenance: DiffProvenance,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DiffProvenance {
    GitObject(String),
    MutableGit {
        expected_document: Arc<DiffDocument>,
        context_lines: usize,
    },
    #[default]
    None,
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
        let paths = split_git_header_paths(header.strip_prefix("diff --git ").unwrap_or_default());
        let path = paths
            .get(1)
            .or_else(|| paths.first())
            .map(|path| normalize_patch_path(path))
            .unwrap_or_else(|| "(unknown file)".to_owned());
        let previous_path = paths.first().map(|path| normalize_patch_path(path));

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

fn split_git_header_paths(header: &str) -> Vec<&str> {
    let bytes = header.as_bytes();
    let mut paths = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if byte == b' ' && !quoted {
            if start < index {
                paths.push(&header[start..index]);
            }
            start = index + 1;
        }
    }
    if start < header.len() {
        paths.push(&header[start..]);
    }
    paths
}

fn normalize_patch_path(path: &str) -> String {
    if path == "/dev/null" {
        return path.to_owned();
    }
    let path = path.split('\t').next().unwrap_or(path);
    let mut bytes = decode_c_quoted_path(path).unwrap_or_else(|| path.as_bytes().to_vec());
    if bytes.starts_with(b"a/") || bytes.starts_with(b"b/") {
        bytes.drain(..2);
    }
    match String::from_utf8(bytes.clone()) {
        Ok(path) if path.starts_with("git-path:") => format!("git-path:utf8:{path}"),
        Ok(path) => path,
        Err(_) => quote_git_path(&bytes),
    }
}

pub fn decode_git_path(path: &str) -> Option<Vec<u8>> {
    if let Some(path) = path.strip_prefix("git-path:utf8:") {
        return Some(path.as_bytes().to_vec());
    }
    decode_c_quoted_path(path.strip_prefix("git-path:bytes:")?)
}

fn decode_c_quoted_path(path: &str) -> Option<Vec<u8>> {
    let inner = path.strip_prefix('"')?.strip_suffix('"')?;
    let bytes = inner.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *bytes.get(index)?;
        index += 1;
        match escaped {
            b'a' => decoded.push(7),
            b'b' => decoded.push(8),
            b't' => decoded.push(b'\t'),
            b'n' => decoded.push(b'\n'),
            b'v' => decoded.push(11),
            b'f' => decoded.push(12),
            b'r' => decoded.push(b'\r'),
            b'\\' | b'"' => decoded.push(escaped),
            b'0'..=b'7' => {
                let mut value = escaped - b'0';
                for _ in 0..2 {
                    let Some(next @ b'0'..=b'7') = bytes.get(index).copied() else {
                        break;
                    };
                    value = value.saturating_mul(8).saturating_add(next - b'0');
                    index += 1;
                }
                decoded.push(value);
            }
            other => decoded.push(other),
        }
    }
    Some(decoded)
}

fn quote_git_path(path: &[u8]) -> String {
    let mut quoted = String::from("git-path:bytes:\"");
    for byte in path {
        match byte {
            b'\\' => quoted.push_str("\\\\"),
            b'"' => quoted.push_str("\\\""),
            0x20..=0x7e => quoted.push(char::from(*byte)),
            _ => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push('"');
    quoted
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
    include_str!("../../tests/fixtures/diff_viewer_baseline_edges.patch");

#[cfg(test)]
mod tests {
    use super::{
        DiffDocument, DiffLineKind, DiffSource, EDGE_FIXTURE, FileStatus, GitComparison,
        HunkCoordinates, HunkRange, Magnitude, ModeChange, PatchInput, PathMove,
    };

    #[test]
    fn quoted_git_paths_keep_spaces_and_reversible_non_utf8_identity() {
        let spaced = DiffDocument::parse(
            "diff --git \"a/name with space.txt\" \"b/name with space.txt\"\n--- \"a/name with space.txt\"\n+++ \"b/name with space.txt\"\n@@ -0,0 +1 @@\n+new\n",
        );
        let non_utf8 = DiffDocument::parse(
            "diff --git \"a/invalid-\\200.txt\" \"b/invalid-\\200.txt\"\n--- /dev/null\n+++ \"b/invalid-\\200.txt\"\n@@ -0,0 +1 @@\n+new\n",
        );

        assert_eq!(spaced.files[0].path, "name with space.txt");
        assert_eq!(
            non_utf8.files[0].path,
            "git-path:bytes:\"invalid-\\200.txt\""
        );
        assert_eq!(
            super::decode_git_path(&non_utf8.files[0].path).unwrap(),
            b"invalid-\x80.txt"
        );
    }

    #[test]
    fn source_descriptions_identify_every_selectable_diff() {
        assert_eq!(
            DiffSource::Git(GitComparison::Changes).description(),
            "changes"
        );
        assert_eq!(
            DiffSource::Git(GitComparison::Staged).description(),
            "staged changes"
        );
        assert_eq!(
            DiffSource::Git(GitComparison::Unstaged).description(),
            "unstaged changes"
        );
        assert_eq!(
            DiffSource::Git(GitComparison::Revision("abc123".into())).description(),
            "revision abc123"
        );
        assert_eq!(
            DiffSource::Git(GitComparison::Range("main...HEAD".into())).description(),
            "range main...HEAD"
        );
        assert_eq!(
            DiffSource::Patch(PatchInput::File("review.patch".into())).description(),
            "patch review.patch"
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
