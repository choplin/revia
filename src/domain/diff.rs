use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentId([u8; 32]);

impl ContentId {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchContent {
    text: Arc<str>,
    content_id: ContentId,
}

impl PatchContent {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let content_id = ContentId::from_bytes(text.as_bytes());
        Self { text, content_id }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn content_id(&self) -> ContentId {
        self.content_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedInput {
    Git(CapturedGitInput),
    PatchFile { path: PathBuf, patch: PatchContent },
    PatchStdin { patch: PatchContent },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedGitInput {
    repository: PathBuf,
    comparison: CapturedGitComparison,
    file_evidence: Vec<CapturedFileEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedFileEvidence {
    Complete(CapturedFileState),
    PatchOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFileState {
    pub before: CapturedFileSide,
    pub after: CapturedFileSide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedFileSide {
    Missing,
    Present(CapturedFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFile {
    pub path: String,
    pub mode: String,
    pub content: CapturedFileContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedFileContent {
    Text(Arc<str>),
    Binary(ContentId),
    Symlink(Arc<[u8]>),
    Gitlink(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedGitComparison {
    Changes {
        head: String,
        patch: PatchContent,
    },
    Staged {
        head: String,
        patch: PatchContent,
    },
    Unstaged {
        head: String,
        patch: PatchContent,
    },
    Revision {
        requested: String,
        commit: String,
        patch: PatchContent,
    },
    Range {
        requested: String,
        left: String,
        right: String,
        base: String,
        target: String,
        patch: PatchContent,
    },
}

impl CapturedInput {
    pub fn patch(&self) -> &PatchContent {
        match self {
            Self::Git(git) => git.comparison.patch(),
            Self::PatchFile { patch, .. } | Self::PatchStdin { patch } => patch,
        }
    }

    pub fn content_id(&self) -> ContentId {
        self.patch().content_id()
    }

    pub fn git_comparison(&self) -> Option<&CapturedGitComparison> {
        match self {
            Self::Git(git) => Some(&git.comparison),
            Self::PatchFile { .. } | Self::PatchStdin { .. } => None,
        }
    }

    pub fn is_stdin(&self) -> bool {
        matches!(self, Self::PatchStdin { .. })
    }
}

impl CapturedGitInput {
    pub fn new(
        repository: PathBuf,
        comparison: CapturedGitComparison,
        file_evidence: Vec<CapturedFileEvidence>,
    ) -> Self {
        Self {
            repository,
            comparison,
            file_evidence,
        }
    }
}

impl CapturedGitComparison {
    pub fn patch(&self) -> &PatchContent {
        match self {
            Self::Changes { patch, .. }
            | Self::Staged { patch, .. }
            | Self::Unstaged { patch, .. }
            | Self::Revision { patch, .. }
            | Self::Range { patch, .. } => patch,
        }
    }

    pub fn immutable_target(&self) -> Option<&str> {
        match self {
            Self::Revision { commit, .. } => Some(commit),
            Self::Range { target, .. } => Some(target),
            Self::Changes { .. } | Self::Staged { .. } | Self::Unstaged { .. } => None,
        }
    }
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

    fn count(&mut self, kind: PatchLineKind) {
        match kind {
            PatchLineKind::Added => self.additions = self.additions.saturating_add(1),
            PatchLineKind::Removed => self.deletions = self.deletions.saturating_add(1),
            PatchLineKind::Context | PatchLineKind::Meta => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPatch {
    content_id: ContentId,
    files: Vec<ParsedFilePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFilePatch {
    dialect: PatchDialect,
    path: String,
    previous_path: Option<String>,
    change: FileChange,
    sides: ParsedFileSides,
    body: ParsedPatchBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchDialect {
    PlainUnified,
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedFileSides {
    TwoWay {
        before: DeclaredFileSide,
        after: DeclaredFileSide,
    },
    Combined {
        parents: Vec<DeclaredFileSide>,
        result: DeclaredFileSide,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredFileSide {
    Missing,
    Present(DeclaredFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredFile {
    path: String,
    mode: DeclaredMode,
    kind: DeclaredFileKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredMode {
    Specified(String),
    NotSpecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredFileKind {
    Specified(FileKind),
    NotSpecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Regular,
    Symlink,
    Gitlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPatchBody {
    Unified(Vec<ParsedSection>),
    Combined {
        parent_count: usize,
        sections: Vec<ParsedCombinedSection>,
    },
    Binary,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSection {
    pub header: String,
    pub coordinates: HunkCoordinates,
    pub lines: Arc<Vec<PatchLine>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCombinedSection {
    pub header: String,
    pub parent_ranges: Vec<HunkRange>,
    pub result_range: HunkRange,
    pub lines: Arc<Vec<PatchLine>>,
}

impl ParsedPatch {
    pub fn parse(content: &PatchContent) -> anyhow::Result<Self> {
        let files = parse_patch(content.text())?;
        Ok(Self {
            content_id: content.content_id(),
            files,
        })
    }

    pub fn files(&self) -> &[ParsedFilePatch] {
        &self.files
    }
}

impl ParsedFilePatch {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn previous_path(&self) -> Option<&str> {
        self.previous_path.as_deref()
    }

    pub fn change(&self) -> &FileChange {
        &self.change
    }

    pub fn body(&self) -> &ParsedPatchBody {
        &self.body
    }

    #[cfg(test)]
    pub fn dialect(&self) -> PatchDialect {
        self.dialect
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewDiff {
    content_id: ContentId,
    files: Vec<FileDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiff {
    Complete(CompleteFileDiff),
    Patch(PatchFileDiff),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteFileDiff {
    Text(CompleteTextDiff),
    Replacement(CompleteReplacementDiff),
    MetadataOnly(CompleteMetadataDiff),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteTextDiff {
    before: TextFileSide,
    after: TextFileSide,
    changes: Vec<TextChange>,
    metadata: FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFileSide {
    Missing,
    Present(TextFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    path: String,
    mode: String,
    document: Arc<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChange {
    before: std::ops::Range<usize>,
    after: std::ops::Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteReplacementDiff {
    before: CompleteFileSide,
    after: CompleteFileSide,
    metadata: FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteFileSide {
    Missing,
    Present(CompleteFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteFile {
    path: String,
    mode: String,
    content: CompleteFileContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteFileContent {
    Text(Arc<str>),
    Binary(ContentId),
    Symlink(Arc<[u8]>),
    Gitlink(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteMetadataDiff {
    before: CompleteFile,
    after: CompleteFile,
    metadata: FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchFileDiff {
    TwoWay(TwoWayPatchFileDiff),
    Combined(CombinedPatchFileDiff),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoWayPatchFileDiff {
    before: DeclaredFileSide,
    after: DeclaredFileSide,
    path: String,
    previous_path: Option<String>,
    change: FileChange,
    body: TwoWayPatchBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwoWayPatchBody {
    Unified(Vec<ParsedSection>),
    Binary,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombinedPatchFileDiff {
    parents: Vec<DeclaredFileSide>,
    result: DeclaredFileSide,
    path: String,
    previous_path: Option<String>,
    change: FileChange,
    parent_count: usize,
    sections: Vec<ParsedCombinedSection>,
}

impl ReviewDiff {
    pub fn from_capture(captured: &CapturedInput) -> anyhow::Result<Self> {
        let parsed = ParsedPatch::parse(captured.patch())?;
        let mut diff = Self::from_parsed(parsed)?;
        if let CapturedInput::Git(git) = captured {
            if git.file_evidence.len() != diff.files.len() {
                anyhow::bail!("captured Git file evidence does not match parsed files");
            }
            for (file, evidence) in diff.files.iter_mut().zip(&git.file_evidence) {
                if let CapturedFileEvidence::Complete(state) = evidence {
                    let metadata = match file {
                        FileDiff::Patch(PatchFileDiff::TwoWay(file)) => file.change.clone(),
                        FileDiff::Patch(PatchFileDiff::Combined(_)) => continue,
                        FileDiff::Complete(_) => unreachable!("diff has not been enriched yet"),
                    };
                    *file = FileDiff::Complete(complete_file_diff(state, metadata)?);
                }
            }
            coalesce_type_changes(&mut diff.files);
        }
        Ok(diff)
    }

    pub fn from_parsed(parsed: ParsedPatch) -> anyhow::Result<Self> {
        let files = parsed
            .files
            .into_iter()
            .map(|file| {
                let ParsedFilePatch {
                    dialect: _,
                    path,
                    previous_path,
                    change,
                    sides,
                    body,
                } = file;
                let file = match (sides, body) {
                    (
                        ParsedFileSides::TwoWay { before, after },
                        ParsedPatchBody::Unified(sections),
                    ) => PatchFileDiff::TwoWay(TwoWayPatchFileDiff {
                        before,
                        after,
                        path,
                        previous_path,
                        change,
                        body: TwoWayPatchBody::Unified(sections),
                    }),
                    (ParsedFileSides::TwoWay { before, after }, ParsedPatchBody::Binary) => {
                        PatchFileDiff::TwoWay(TwoWayPatchFileDiff {
                            before,
                            after,
                            path,
                            previous_path,
                            change,
                            body: TwoWayPatchBody::Binary,
                        })
                    }
                    (ParsedFileSides::TwoWay { before, after }, ParsedPatchBody::None) => {
                        PatchFileDiff::TwoWay(TwoWayPatchFileDiff {
                            before,
                            after,
                            path,
                            previous_path,
                            change,
                            body: TwoWayPatchBody::None,
                        })
                    }
                    (
                        ParsedFileSides::Combined { parents, result },
                        ParsedPatchBody::Combined {
                            parent_count,
                            sections,
                        },
                    ) => {
                        if parent_count < 2
                            || parents.len() != parent_count
                            || sections
                                .iter()
                                .any(|section| section.parent_ranges.len() != parent_count)
                        {
                            anyhow::bail!("combined patch has inconsistent parent arity");
                        }
                        PatchFileDiff::Combined(CombinedPatchFileDiff {
                            parents,
                            result,
                            path,
                            previous_path,
                            change,
                            parent_count,
                            sections,
                        })
                    }
                    _ => anyhow::bail!("patch side cardinality does not match its body"),
                };
                Ok(FileDiff::Patch(file))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            content_id: parsed.content_id,
            files,
        })
    }

    #[cfg(test)]
    pub fn content_id(&self) -> ContentId {
        self.content_id
    }

    #[cfg(test)]
    pub fn files(&self) -> &[FileDiff] {
        &self.files
    }
}

fn coalesce_type_changes(files: &mut Vec<FileDiff>) {
    let mut index = 0;
    while index + 1 < files.len() {
        let Some((before, before_path)) = deleted_complete_side(&files[index]) else {
            index += 1;
            continue;
        };
        let Some((after, after_path)) = added_complete_side(&files[index + 1]) else {
            index += 1;
            continue;
        };
        if before_path != after_path {
            index += 1;
            continue;
        }
        let mode = match (&before, &after) {
            (CompleteFileSide::Present(before), CompleteFileSide::Present(after)) => {
                Some(ModeChange {
                    old: before.mode.clone(),
                    new: after.mode.clone(),
                })
            }
            _ => None,
        };
        let binary = side_is_binary(&before) || side_is_binary(&after);
        files[index] = FileDiff::Complete(CompleteFileDiff::Replacement(CompleteReplacementDiff {
            before,
            after,
            metadata: FileChange {
                mode,
                binary,
                ..FileChange::default()
            },
        }));
        files.remove(index + 1);
    }
}

fn deleted_complete_side(file: &FileDiff) -> Option<(CompleteFileSide, String)> {
    let (before, after) = complete_sides(file)?;
    if !matches!(after, CompleteFileSide::Missing) {
        return None;
    }
    let CompleteFileSide::Present(file) = &before else {
        return None;
    };
    let path = file.path.clone();
    Some((before, path))
}

fn added_complete_side(file: &FileDiff) -> Option<(CompleteFileSide, String)> {
    let (before, after) = complete_sides(file)?;
    if !matches!(before, CompleteFileSide::Missing) {
        return None;
    }
    let CompleteFileSide::Present(file) = &after else {
        return None;
    };
    let path = file.path.clone();
    Some((after, path))
}

fn complete_sides(file: &FileDiff) -> Option<(CompleteFileSide, CompleteFileSide)> {
    match file {
        FileDiff::Complete(CompleteFileDiff::Text(file)) => Some((
            complete_text_side(&file.before),
            complete_text_side(&file.after),
        )),
        FileDiff::Complete(CompleteFileDiff::Replacement(file)) => {
            Some((file.before.clone(), file.after.clone()))
        }
        FileDiff::Complete(CompleteFileDiff::MetadataOnly(_)) | FileDiff::Patch(_) => None,
    }
}

fn complete_text_side(side: &TextFileSide) -> CompleteFileSide {
    match side {
        TextFileSide::Missing => CompleteFileSide::Missing,
        TextFileSide::Present(file) => CompleteFileSide::Present(CompleteFile {
            path: file.path.clone(),
            mode: file.mode.clone(),
            content: CompleteFileContent::Text(file.document.clone()),
        }),
    }
}

fn side_is_binary(side: &CompleteFileSide) -> bool {
    matches!(
        side,
        CompleteFileSide::Present(CompleteFile {
            content: CompleteFileContent::Binary(_),
            ..
        })
    )
}

fn complete_file_diff(
    state: &CapturedFileState,
    metadata: FileChange,
) -> anyhow::Result<CompleteFileDiff> {
    if matches!(state.before, CapturedFileSide::Missing)
        && matches!(state.after, CapturedFileSide::Missing)
    {
        anyhow::bail!("complete file diff cannot have two missing sides");
    }
    let before_text = captured_text_side(&state.before);
    let after_text = captured_text_side(&state.after);
    if let (Some(before), Some(after)) = (before_text, after_text) {
        let changes = text_changes(side_text(&before), side_text(&after));
        if changes.is_empty()
            && matches!(
                (&before, &after),
                (TextFileSide::Present(_), TextFileSide::Present(_))
            )
        {
            let (CapturedFileSide::Present(before), CapturedFileSide::Present(after)) =
                (&state.before, &state.after)
            else {
                unreachable!("present text sides were checked above")
            };
            if before.path == after.path && before.mode == after.mode {
                anyhow::bail!("complete file evidence contains no change");
            }
            return Ok(CompleteFileDiff::MetadataOnly(CompleteMetadataDiff {
                before: complete_file(before),
                after: complete_file(after),
                metadata,
            }));
        }
        return Ok(CompleteFileDiff::Text(CompleteTextDiff {
            before,
            after,
            changes,
            metadata,
        }));
    }

    let before = complete_side(&state.before);
    let after = complete_side(&state.after);
    if let (CompleteFileSide::Present(before), CompleteFileSide::Present(after)) = (&before, &after)
        && before.content == after.content
    {
        if before.path == after.path && before.mode == after.mode {
            anyhow::bail!("complete file evidence contains no change");
        }
        return Ok(CompleteFileDiff::MetadataOnly(CompleteMetadataDiff {
            before: before.clone(),
            after: after.clone(),
            metadata,
        }));
    }
    Ok(CompleteFileDiff::Replacement(CompleteReplacementDiff {
        before,
        after,
        metadata,
    }))
}

fn captured_text_side(side: &CapturedFileSide) -> Option<TextFileSide> {
    match side {
        CapturedFileSide::Missing => Some(TextFileSide::Missing),
        CapturedFileSide::Present(file) => match &file.content {
            CapturedFileContent::Text(document) => Some(TextFileSide::Present(TextFile {
                path: file.path.clone(),
                mode: file.mode.clone(),
                document: document.clone(),
            })),
            CapturedFileContent::Binary(_)
            | CapturedFileContent::Symlink(_)
            | CapturedFileContent::Gitlink(_) => None,
        },
    }
}

fn complete_side(side: &CapturedFileSide) -> CompleteFileSide {
    match side {
        CapturedFileSide::Missing => CompleteFileSide::Missing,
        CapturedFileSide::Present(file) => CompleteFileSide::Present(complete_file(file)),
    }
}

fn complete_file(file: &CapturedFile) -> CompleteFile {
    CompleteFile {
        path: file.path.clone(),
        mode: file.mode.clone(),
        content: match &file.content {
            CapturedFileContent::Text(text) => CompleteFileContent::Text(text.clone()),
            CapturedFileContent::Binary(hash) => CompleteFileContent::Binary(*hash),
            CapturedFileContent::Symlink(target) => CompleteFileContent::Symlink(target.clone()),
            CapturedFileContent::Gitlink(object) => CompleteFileContent::Gitlink(object.clone()),
        },
    }
}

fn side_text(side: &TextFileSide) -> &str {
    match side {
        TextFileSide::Missing => "",
        TextFileSide::Present(file) => &file.document,
    }
}

fn text_changes(before: &str, after: &str) -> Vec<TextChange> {
    let before_offsets = line_offsets(before);
    let after_offsets = line_offsets(after);
    similar::TextDiff::from_lines(before, after)
        .ops()
        .iter()
        .filter(|operation| operation.tag() != similar::DiffTag::Equal)
        .map(|operation| TextChange {
            before: line_range_to_bytes(operation.old_range(), &before_offsets, before.len()),
            after: line_range_to_bytes(operation.new_range(), &after_offsets, after.len()),
        })
        .collect()
}

fn line_offsets(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect()
}

fn line_range_to_bytes(
    range: std::ops::Range<usize>,
    offsets: &[usize],
    text_len: usize,
) -> std::ops::Range<usize> {
    let start = offsets.get(range.start).copied().unwrap_or(text_len);
    let end = offsets.get(range.end).copied().unwrap_or(text_len);
    start..end
}

pub fn build_review(
    captured: &CapturedInput,
    context_lines: usize,
) -> anyhow::Result<(Arc<ReviewDiff>, Arc<ReviewPresentation>)> {
    let diff = Arc::new(ReviewDiff::from_capture(captured)?);
    let presentation = Arc::new(ReviewPresentation::from_review_diff(&diff, context_lines));
    Ok((diff, presentation))
}

struct ParsedFileBuilder {
    dialect: PatchDialect,
    path: String,
    previous_path: Option<String>,
    change: FileChange,
    unified: Vec<ParsedSection>,
    combined: Vec<ParsedCombinedSection>,
    combined_parents: Option<usize>,
    before_mode: Option<String>,
    after_mode: Option<String>,
    combined_parent_modes: Vec<String>,
}

impl ParsedFileBuilder {
    fn git(header: &str) -> Self {
        let paths = split_git_header_paths(
            header
                .split_once(' ')
                .and_then(|(_, rest)| rest.split_once(' '))
                .map_or("", |(_, paths)| paths),
        );
        let path = paths
            .get(1)
            .or_else(|| paths.first())
            .map(|path| normalize_patch_path(path))
            .unwrap_or_else(|| "(unknown file)".to_owned());
        let previous_path = paths.first().map(|path| normalize_patch_path(path));
        Self::new(PatchDialect::Git, path, previous_path)
    }

    fn plain(previous_path: &str) -> Self {
        let previous_path = normalize_patch_path(previous_path);
        let path = if previous_path == "/dev/null" {
            "(unknown file)".to_owned()
        } else {
            previous_path.clone()
        };
        Self::new(PatchDialect::PlainUnified, path, Some(previous_path))
    }

    fn new(dialect: PatchDialect, path: String, previous_path: Option<String>) -> Self {
        Self {
            dialect,
            path,
            previous_path,
            change: FileChange::default(),
            unified: Vec::new(),
            combined: Vec::new(),
            combined_parents: None,
            before_mode: None,
            after_mode: None,
            combined_parent_modes: Vec::new(),
        }
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
        if let Some(mode) = line.strip_prefix("new file mode ") {
            self.after_mode = Some(mode.trim().to_owned());
        } else if let Some(mode) = line.strip_prefix("deleted file mode ") {
            self.before_mode = Some(mode.trim().to_owned());
        } else if let Some(mode) = line.strip_prefix("old mode ") {
            self.before_mode = Some(mode.trim().to_owned());
        } else if let Some(mode) = line.strip_prefix("new mode ") {
            self.after_mode = Some(mode.trim().to_owned());
        } else if let Some(modes) = line.strip_prefix("mode ") {
            if let Some((parents, result)) = modes.trim().split_once("..") {
                self.combined_parent_modes = parents.split(',').map(str::to_owned).collect();
                self.after_mode = Some(result.to_owned());
            }
        } else if let Some(mode) = line
            .strip_prefix("index ")
            .and_then(|value| value.split_whitespace().nth(1))
        {
            self.before_mode.get_or_insert_with(|| mode.to_owned());
            self.after_mode.get_or_insert_with(|| mode.to_owned());
        }
        self.change.absorb(line);
    }

    fn finish(self) -> ParsedFilePatch {
        let (sides, body) = if let Some(parent_count) = self.combined_parents {
            let parent_path = self.previous_path.as_deref().unwrap_or(&self.path);
            let parents = (0..parent_count)
                .map(|index| {
                    declared_present(
                        parent_path,
                        self.combined_parent_modes
                            .get(index)
                            .map(String::as_str)
                            .or(self.before_mode.as_deref()),
                    )
                })
                .collect();
            let result = declared_present(&self.path, self.after_mode.as_deref());
            (
                ParsedFileSides::Combined { parents, result },
                ParsedPatchBody::Combined {
                    parent_count,
                    sections: self.combined,
                },
            )
        } else if self.change.binary {
            (two_way_sides(&self), ParsedPatchBody::Binary)
        } else if self.unified.is_empty() {
            (two_way_sides(&self), ParsedPatchBody::None)
        } else {
            (two_way_sides(&self), ParsedPatchBody::Unified(self.unified))
        };
        ParsedFilePatch {
            dialect: self.dialect,
            path: self.path,
            previous_path: self.previous_path,
            change: self.change,
            sides,
            body,
        }
    }
}

fn two_way_sides(file: &ParsedFileBuilder) -> ParsedFileSides {
    let before = if file.change.status == FileStatus::Added
        || file.previous_path.as_deref() == Some("/dev/null")
    {
        DeclaredFileSide::Missing
    } else {
        declared_present(
            file.previous_path.as_deref().unwrap_or(&file.path),
            file.before_mode.as_deref(),
        )
    };
    let after = if file.change.status == FileStatus::Deleted || file.path == "/dev/null" {
        DeclaredFileSide::Missing
    } else {
        declared_present(&file.path, file.after_mode.as_deref())
    };
    ParsedFileSides::TwoWay { before, after }
}

fn declared_present(path: &str, mode: Option<&str>) -> DeclaredFileSide {
    let mode = mode.map_or(DeclaredMode::NotSpecified, |mode| {
        DeclaredMode::Specified(mode.to_owned())
    });
    let kind = match mode.as_ref() {
        Some("120000") => DeclaredFileKind::Specified(FileKind::Symlink),
        Some("160000") => DeclaredFileKind::Specified(FileKind::Gitlink),
        Some(_) => DeclaredFileKind::Specified(FileKind::Regular),
        None => DeclaredFileKind::NotSpecified,
    };
    DeclaredFileSide::Present(DeclaredFile {
        path: path.to_owned(),
        mode,
        kind,
    })
}

impl DeclaredMode {
    fn as_ref(&self) -> Option<&str> {
        match self {
            Self::Specified(mode) => Some(mode),
            Self::NotSpecified => None,
        }
    }
}

fn parse_patch(text: &str) -> anyhow::Result<Vec<ParsedFilePatch>> {
    let mut files = Vec::new();
    let mut current: Option<ParsedFileBuilder> = None;

    for line in text.lines() {
        if line.starts_with("diff --git ")
            || line.starts_with("diff --cc ")
            || line.starts_with("diff --combined ")
        {
            if let Some(file) = current.take() {
                files.push(file.finish());
            }
            current = Some(ParsedFileBuilder::git(line));
            continue;
        }
        if current.is_none() && line.starts_with("--- ") {
            current = Some(ParsedFileBuilder::plain(
                line.strip_prefix("--- ").unwrap_or_default(),
            ));
        }
        let Some(file) = current.as_mut() else {
            continue;
        };

        if line.starts_with("@@@") {
            let (parent_ranges, result_range) = parse_combined_header(line)
                .ok_or_else(|| anyhow::anyhow!("malformed combined hunk header: {line}"))?;
            let parent_count = parent_ranges.len();
            if parent_count < 2
                || file
                    .combined_parents
                    .is_some_and(|count| count != parent_count)
            {
                anyhow::bail!("combined patch has inconsistent parent arity");
            }
            file.combined_parents = Some(parent_count);
            file.combined.push(ParsedCombinedSection {
                header: line.to_owned(),
                parent_ranges,
                result_range,
                lines: Arc::new(Vec::new()),
            });
        } else if line.starts_with("@@") {
            let coordinates = HunkCoordinates::parse(line)
                .ok_or_else(|| anyhow::anyhow!("malformed hunk header: {line}"))?;
            file.unified.push(ParsedSection {
                header: line.to_owned(),
                coordinates,
                lines: Arc::new(Vec::new()),
            });
        } else if let Some(section) = file.combined.last_mut() {
            Arc::make_mut(&mut section.lines).push(PatchLine::from_combined_raw(
                line,
                file.combined_parents.unwrap_or(2),
            ));
        } else if let Some(section) = file.unified.last_mut() {
            Arc::make_mut(&mut section.lines).push(PatchLine::from_raw(line));
        } else {
            file.absorb_metadata(line);
        }
    }
    if let Some(file) = current {
        files.push(file.finish());
    }
    Ok(files)
}

fn parse_combined_header(header: &str) -> Option<(Vec<HunkRange>, HunkRange)> {
    let marker_count = header.bytes().take_while(|byte| *byte == b'@').count();
    let parent_count = marker_count.checked_sub(1)?;
    if parent_count < 2 {
        return None;
    }
    let mut fields = header[marker_count..].split_whitespace();
    let parents = (0..parent_count)
        .map(|_| HunkRange::parse(fields.next()?, '-'))
        .collect::<Option<Vec<_>>>()?;
    let result = HunkRange::parse(fields.next()?, '+')?;
    Some((parents, result))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewPresentation {
    pub files: Vec<PresentedFile>,
}

impl ReviewPresentation {
    pub fn from_review_diff(diff: &ReviewDiff, context_lines: usize) -> Self {
        let files = diff
            .files
            .iter()
            .map(|file| PresentedFile::from_diff(file, context_lines))
            .collect();
        Self { files }
    }

    #[cfg(test)]
    pub fn parse(text: &str) -> Self {
        let content = PatchContent::new(Arc::<str>::from(text));
        ParsedPatch::parse(&content)
            .and_then(ReviewDiff::from_parsed)
            .map(|diff| Self::from_review_diff(&diff, 3))
            .unwrap_or_default()
    }

    /// Total additions and deletions across every file in the changeset.
    pub fn magnitude(&self) -> Magnitude {
        self.files.iter().fold(Magnitude::default(), |total, file| {
            total.combined(file.magnitude)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub change: FileChange,
    pub magnitude: Magnitude,
    pub hunks: Vec<PresentedSection>,
}

impl PresentedFile {
    fn from_diff(file: &FileDiff, context_lines: usize) -> Self {
        match file {
            FileDiff::Patch(PatchFileDiff::TwoWay(file)) => {
                let hunks = match &file.body {
                    TwoWayPatchBody::Unified(sections) => sections
                        .iter()
                        .map(|section| PresentedSection {
                            header: section.header.clone(),
                            coordinates: Some(section.coordinates),
                            lines: section.lines.clone(),
                        })
                        .collect(),
                    TwoWayPatchBody::Binary | TwoWayPatchBody::None => Vec::new(),
                };
                let magnitude = magnitude(&hunks);
                Self {
                    path: file.path.clone(),
                    previous_path: file.previous_path.clone(),
                    change: file.change.clone(),
                    magnitude,
                    hunks,
                }
            }
            FileDiff::Patch(PatchFileDiff::Combined(file)) => {
                debug_assert!(
                    file.parent_count >= 2
                        && file
                            .sections
                            .iter()
                            .all(|section| section.parent_ranges.len() == file.parent_count)
                );
                let hunks: Vec<PresentedSection> = file
                    .sections
                    .iter()
                    .map(|section| PresentedSection {
                        header: section.header.clone(),
                        coordinates: None,
                        lines: section.lines.clone(),
                    })
                    .collect();
                let magnitude = magnitude(&hunks);
                Self {
                    path: file.path.clone(),
                    previous_path: file.previous_path.clone(),
                    change: file.change.clone(),
                    magnitude,
                    hunks,
                }
            }
            FileDiff::Complete(file) => Self::from_complete(file, context_lines),
        }
    }

    fn from_complete(file: &CompleteFileDiff, context_lines: usize) -> Self {
        let (path, previous_path, change) = match file {
            CompleteFileDiff::Text(file) => {
                let (path, previous_path, derived) = complete_text_descriptors(file);
                (
                    path,
                    previous_path,
                    merge_file_change(derived, &file.metadata),
                )
            }
            CompleteFileDiff::Replacement(file) => {
                let (path, previous_path, derived) =
                    complete_side_descriptors(&file.before, &file.after);
                (
                    path,
                    previous_path,
                    merge_file_change(derived, &file.metadata),
                )
            }
            CompleteFileDiff::MetadataOnly(file) => {
                let derived = FileChange {
                    mode: (file.before.mode != file.after.mode).then(|| ModeChange {
                        old: file.before.mode.clone(),
                        new: file.after.mode.clone(),
                    }),
                    moved: (file.before.path != file.after.path).then_some(PathMove::default()),
                    ..FileChange::default()
                };
                (
                    file.after.path.clone(),
                    Some(file.before.path.clone()),
                    merge_file_change(derived, &file.metadata),
                )
            }
        };
        let hunks = match file {
            CompleteFileDiff::Text(file) => complete_text_hunks(file, context_lines),
            CompleteFileDiff::Replacement(_) | CompleteFileDiff::MetadataOnly(_) => Vec::new(),
        };
        let magnitude = magnitude(&hunks);
        Self {
            path,
            previous_path,
            change,
            magnitude,
            hunks,
        }
    }

    pub fn extension(&self) -> Option<&str> {
        Path::new(&self.path)
            .extension()
            .and_then(|extension| extension.to_str())
    }
}

fn merge_file_change(mut derived: FileChange, parsed: &FileChange) -> FileChange {
    derived.status = parsed.status;
    if parsed.moved.is_some() {
        derived.moved = parsed.moved;
    }
    if parsed.mode.is_some() {
        derived.mode = parsed.mode.clone();
    }
    derived.binary |= parsed.binary;
    derived
}

fn complete_text_hunks(file: &CompleteTextDiff, context_lines: usize) -> Vec<PresentedSection> {
    let before = side_text(&file.before);
    let after = side_text(&file.after);
    debug_assert!(
        file.changes
            .iter()
            .all(|change| change.before.end <= before.len() && change.after.end <= after.len())
    );
    let diff = similar::TextDiff::from_lines(before, after);
    diff.grouped_ops(context_lines)
        .into_iter()
        .filter_map(|group| {
            let first = group.first()?;
            let last = group.last()?;
            let old = first.old_range().start..last.old_range().end;
            let new = first.new_range().start..last.new_range().end;
            let coordinates = HunkCoordinates {
                old: unified_range(old.clone()),
                new: unified_range(new.clone()),
            };
            let mut lines = Vec::new();
            for change in group
                .iter()
                .flat_map(|operation| diff.iter_changes(operation))
            {
                lines.push(PatchLine {
                    kind: match change.tag() {
                        similar::ChangeTag::Equal => PatchLineKind::Context,
                        similar::ChangeTag::Delete => PatchLineKind::Removed,
                        similar::ChangeTag::Insert => PatchLineKind::Added,
                    },
                    text: change
                        .value()
                        .strip_suffix('\n')
                        .unwrap_or(change.value())
                        .to_owned(),
                });
                if change.missing_newline() {
                    lines.push(PatchLine {
                        kind: PatchLineKind::Meta,
                        text: "\\ No newline at end of file".into(),
                    });
                }
            }
            Some(PresentedSection {
                header: format!(
                    "@@ -{},{} +{},{} @@",
                    coordinates.old.start,
                    coordinates.old.count,
                    coordinates.new.start,
                    coordinates.new.count
                ),
                coordinates: Some(coordinates),
                lines: Arc::new(lines),
            })
        })
        .collect()
}

fn unified_range(range: std::ops::Range<usize>) -> HunkRange {
    HunkRange {
        start: if range.is_empty() {
            range.start
        } else {
            range.start + 1
        },
        count: range.len(),
    }
}

fn magnitude(hunks: &[PresentedSection]) -> Magnitude {
    hunks.iter().flat_map(|hunk| hunk.lines.iter()).fold(
        Magnitude::default(),
        |mut magnitude, line| {
            magnitude.count(line.kind);
            magnitude
        },
    )
}

fn complete_text_descriptors(file: &CompleteTextDiff) -> (String, Option<String>, FileChange) {
    match (&file.before, &file.after) {
        (TextFileSide::Missing, TextFileSide::Present(after)) => (
            after.path.clone(),
            None,
            FileChange {
                status: FileStatus::Added,
                ..FileChange::default()
            },
        ),
        (TextFileSide::Present(before), TextFileSide::Missing) => (
            before.path.clone(),
            Some(before.path.clone()),
            FileChange {
                status: FileStatus::Deleted,
                ..FileChange::default()
            },
        ),
        (TextFileSide::Present(before), TextFileSide::Present(after)) => (
            after.path.clone(),
            Some(before.path.clone()),
            FileChange {
                mode: (before.mode != after.mode).then(|| ModeChange {
                    old: before.mode.clone(),
                    new: after.mode.clone(),
                }),
                moved: (before.path != after.path).then_some(PathMove::default()),
                ..FileChange::default()
            },
        ),
        (TextFileSide::Missing, TextFileSide::Missing) => {
            unreachable!("complete text diff cannot have two missing sides")
        }
    }
}

fn complete_side_descriptors(
    before: &CompleteFileSide,
    after: &CompleteFileSide,
) -> (String, Option<String>, FileChange) {
    match (before, after) {
        (CompleteFileSide::Missing, CompleteFileSide::Present(after)) => (
            after.path.clone(),
            None,
            FileChange {
                status: FileStatus::Added,
                binary: matches!(after.content, CompleteFileContent::Binary(_)),
                ..FileChange::default()
            },
        ),
        (CompleteFileSide::Present(before), CompleteFileSide::Missing) => (
            before.path.clone(),
            Some(before.path.clone()),
            FileChange {
                status: FileStatus::Deleted,
                binary: matches!(before.content, CompleteFileContent::Binary(_)),
                ..FileChange::default()
            },
        ),
        (CompleteFileSide::Present(before), CompleteFileSide::Present(after)) => (
            after.path.clone(),
            Some(before.path.clone()),
            FileChange {
                binary: matches!(before.content, CompleteFileContent::Binary(_))
                    || matches!(after.content, CompleteFileContent::Binary(_)),
                mode: (before.mode != after.mode).then(|| ModeChange {
                    old: before.mode.clone(),
                    new: after.mode.clone(),
                }),
                moved: (before.path != after.path).then_some(PathMove::default()),
                ..FileChange::default()
            },
        ),
        (CompleteFileSide::Missing, CompleteFileSide::Missing) => {
            unreachable!("complete replacement cannot have two missing sides")
        }
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

/// A rename or copy. The old and new paths already live on the `PresentedFile`, so
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
pub struct PresentedSection {
    pub header: String,
    pub coordinates: Option<HunkCoordinates>,
    pub lines: Arc<Vec<PatchLine>>,
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
pub struct PatchLine {
    pub kind: PatchLineKind,
    pub text: String,
}

impl PatchLine {
    fn from_raw(line: &str) -> Self {
        let (kind, text) = match line.as_bytes().first() {
            Some(b'+') => (PatchLineKind::Added, &line[1..]),
            Some(b'-') => (PatchLineKind::Removed, &line[1..]),
            Some(b' ') => (PatchLineKind::Context, &line[1..]),
            _ => (PatchLineKind::Meta, line),
        };
        Self {
            kind,
            text: text.to_owned(),
        }
    }

    fn from_combined_raw(line: &str, parent_count: usize) -> Self {
        if line.starts_with('\\') {
            return Self {
                kind: PatchLineKind::Meta,
                text: line.to_owned(),
            };
        }
        let prefixes = line.as_bytes().get(..parent_count).unwrap_or_default();
        let kind = if prefixes.iter().all(|prefix| *prefix == b'+') {
            PatchLineKind::Added
        } else if prefixes.contains(&b'-') && !prefixes.contains(&b'+') {
            PatchLineKind::Removed
        } else {
            PatchLineKind::Context
        };
        Self {
            kind,
            text: line.get(parent_count..).unwrap_or(line).to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchLineKind {
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
        CapturedFile, CapturedFileContent, CapturedFileEvidence, CapturedFileSide,
        CapturedFileState, CapturedGitComparison, CapturedGitInput, CapturedInput,
        CompleteFileDiff, DeclaredFileSide, DiffSource, EDGE_FIXTURE, FileDiff, FileStatus,
        GitComparison, HunkCoordinates, HunkRange, Magnitude, ModeChange, ParsedPatch,
        ParsedPatchBody, PatchContent, PatchDialect, PatchFileDiff, PatchInput, PatchLineKind,
        PathMove, ReviewDiff, ReviewPresentation,
    };

    #[test]
    fn quoted_git_paths_keep_spaces_and_reversible_non_utf8_identity() {
        let spaced = ReviewPresentation::parse(
            "diff --git \"a/name with space.txt\" \"b/name with space.txt\"\n--- \"a/name with space.txt\"\n+++ \"b/name with space.txt\"\n@@ -0,0 +1 @@\n+new\n",
        );
        let non_utf8 = ReviewPresentation::parse(
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
        let document = ReviewPresentation::parse(
            "diff --git a/src/lib.rs b/src/lib.rs\nindex 111..222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n old\n+new\n\\ No newline at end of file\ndiff --git a/readme.md b/readme.md\nnew file mode 100644\n--- /dev/null\n+++ b/readme.md\n@@ -0,0 +1 @@\n+hello\n",
        );

        assert_eq!(document.files.len(), 2);
        assert_eq!(document.files[0].path, "src/lib.rs");
        assert_eq!(document.files[0].hunks.len(), 1);
        assert_eq!(
            document.files[0].hunks[0].lines[1].kind,
            PatchLineKind::Added
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
        let document = ReviewPresentation::parse(
            "diff --git a/old-a.rs b/old-a.rs\ndeleted file mode 100644\n--- a/old-a.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old a\ndiff --git a/old-b.rs b/old-b.rs\ndeleted file mode 100644\n--- a/old-b.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old b\n",
        );

        assert_eq!(document.files[0].path, "old-a.rs");
        assert_eq!(document.files[1].path, "old-b.rs");
    }

    #[test]
    fn counts_per_file_and_changeset_magnitude_from_the_patch_body() {
        let document = ReviewPresentation::parse(EDGE_FIXTURE);
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
        let document = ReviewPresentation::parse(EDGE_FIXTURE);
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

    #[test]
    fn combined_patch_keeps_ordered_parent_arity() {
        let content = PatchContent::new(
            "diff --cc example.txt\nindex 111,222..333\n--- a/example.txt\n+++ b/example.txt\n@@@ -1,1 -1,1 +1,1 @@@\n- old\n +new\n",
        );
        let parsed = ParsedPatch::parse(&content).unwrap();

        let ParsedPatchBody::Combined {
            parent_count,
            sections,
        } = parsed.files()[0].body()
        else {
            panic!("combined syntax must not become a two-way hunk");
        };
        assert_eq!(*parent_count, 2);
        assert_eq!(sections[0].parent_ranges.len(), 2);

        let semantic = ReviewDiff::from_parsed(parsed).unwrap();
        let [FileDiff::Patch(PatchFileDiff::Combined(file))] = semantic.files() else {
            panic!("combined syntax must remain multi-parent semantic evidence");
        };
        assert_eq!(file.parents.len(), 2);
        assert!(matches!(file.result, DeclaredFileSide::Present(_)));
    }

    #[test]
    fn plain_unified_input_is_parsed_without_git_headers() {
        let presentation = ReviewPresentation::parse(
            "--- old/example.txt\n+++ new/example.txt\n@@ -1 +1 @@\n-before\n+after\n",
        );

        assert_eq!(presentation.files.len(), 1);
        assert_eq!(presentation.files[0].path, "new/example.txt");
        assert_eq!(presentation.files[0].hunks.len(), 1);

        let parsed = ParsedPatch::parse(&PatchContent::new(
            "--- old/example.txt\n+++ new/example.txt\n@@ -1 +1 @@\n-before\n+after\n",
        ))
        .unwrap();
        assert_eq!(parsed.files()[0].dialect(), PatchDialect::PlainUnified);
    }

    #[test]
    fn complete_git_evidence_becomes_a_complete_text_diff() {
        let patch = PatchContent::new(
            "diff --git a/example.txt b/example.txt\n--- a/example.txt\n+++ b/example.txt\n@@ -1 +1 @@\n-before\n+after\n",
        );
        let expected_id = patch.content_id();
        let captured = CapturedInput::Git(CapturedGitInput::new(
            ".".into(),
            CapturedGitComparison::Changes {
                head: "head".into(),
                patch,
            },
            vec![CapturedFileEvidence::Complete(CapturedFileState {
                before: CapturedFileSide::Present(CapturedFile {
                    path: "example.txt".into(),
                    mode: "100644".into(),
                    content: CapturedFileContent::Text("before\n".into()),
                }),
                after: CapturedFileSide::Present(CapturedFile {
                    path: "example.txt".into(),
                    mode: "100644".into(),
                    content: CapturedFileContent::Text("after\n".into()),
                }),
            })],
        ));

        let diff = ReviewDiff::from_capture(&captured).unwrap();

        assert_eq!(diff.content_id(), expected_id);
        assert!(matches!(
            diff.files(),
            [FileDiff::Complete(CompleteFileDiff::Text(_))]
        ));
        let presentation = ReviewPresentation::from_review_diff(&diff, 3);
        assert_eq!(presentation.files[0].hunks.len(), 1);
        assert_eq!(presentation.files[0].magnitude.additions, 1);
        assert_eq!(presentation.files[0].magnitude.deletions, 1);
    }
}
