use std::sync::Arc;

use crate::{
    app::view_state::{FocusArea, LayoutMode},
    domain::{
        anchor::HunkLocation,
        diff::{DiffLine, FileChange, HunkCoordinates, Magnitude},
        thread::ThreadId,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub header: Header,
    pub file_rail: Option<FileRail>,
    pub body: Body,
    pub footer: Footer,
    pub overlay: Option<Overlay>,
    pub layout: LayoutPolicy,
}

/// The opening frame: which comparison is under review and how large it is.
///
/// Discussion aggregation deliberately lives elsewhere; magnitude must not be
/// displaced by collaboration counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub comparison: String,
    pub file_count: usize,
    pub magnitude: Magnitude,
    pub active_filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRail {
    pub selected: Option<usize>,
    pub tree: bool,
    pub rows: Vec<FileRailRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRailRow {
    pub label: String,
    pub path: String,
    pub depth: usize,
    pub kind: FileRailRowKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRailRowKind {
    Directory {
        collapsed: bool,
    },
    File {
        change: FileChange,
        attention: FileAttention,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAttention {
    NeedsAttention,
    Open,
    Resolved,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Review(Box<ReviewBody>),
    Rollup(RollupBody),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBody {
    pub scroll: usize,
    pub viewport: ReviewViewport,
    pub empty_state: Option<String>,
    pub search_target: Option<DiffSearchTarget>,
    /// The active query is presentation state rather than source data.  Keeping
    /// it here lets the renderer mark the exact matching graphemes without
    /// changing the semantic diff rows.
    pub search_query: Option<String>,
    pub files: Vec<ReviewFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSearchTarget {
    FilePath {
        path: String,
    },
    HunkHeader {
        location: HunkLocation,
    },
    DiffLine {
        location: HunkLocation,
        line_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewViewport {
    pub presentation_width: u16,
    pub total_rows: usize,
    pub visible_rows: usize,
    pub sticky_context: Option<StickyReviewContext>,
    pub sections: Arc<Vec<ReviewWindowSection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewWindowSection {
    FileHeader {
        file_index: usize,
        start: usize,
        end: usize,
    },
    Hunk {
        file_index: usize,
        hunk_index: usize,
        start: usize,
        end: usize,
    },
}

impl ReviewWindowSection {
    pub(crate) fn start(self) -> usize {
        match self {
            Self::FileHeader { start, .. } | Self::Hunk { start, .. } => start,
        }
    }

    pub(crate) fn end(self) -> usize {
        match self {
            Self::FileHeader { end, .. } | Self::Hunk { end, .. } => end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickyReviewContext {
    pub file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub magnitude: Magnitude,
    /// Shown on the hunk row for files that have no hunks to name, so the
    /// second sticky row never becomes blank chrome.
    pub notes: Vec<String>,
    pub hunk_header: Option<String>,
    pub hunk_index: Option<usize>,
    pub hunk_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFile {
    pub path: String,
    pub selected: bool,
    pub extension: Option<String>,
    pub magnitude: Magnitude,
    /// Semantic file-change facts (rename, mode, binary, new/deleted), already
    /// reduced from Git's transport headers.
    pub notes: Vec<String>,
    pub hunks: Vec<ReviewHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewHunk {
    pub anchor: HunkLocation,
    pub header: Option<String>,
    pub coordinates: Option<HunkCoordinates>,
    pub selected: bool,
    pub lines: Arc<Vec<DiffLine>>,
    pub threads: Vec<ThreadCard>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCard {
    pub id: ThreadId,
    pub state: ThreadState,
    pub resolved: bool,
    pub outdated: bool,
    pub active: bool,
    pub expanded: bool,
    pub message_count: usize,
    pub closed_by: Option<String>,
    pub latest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    NeedsAttention,
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollupBody {
    pub summary: String,
    pub scroll: u16,
    pub items: Vec<RollupItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollupItem {
    pub id: ThreadId,
    pub selected: bool,
    pub state: ThreadState,
    pub path: String,
    pub hunk_header: String,
    pub closed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    pub current_context: CurrentContext,
    pub contextual_keys: ContextualKeys,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentContext {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextualKeys {
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceContext {
    Review,
    Threads,
    Files,
    SearchInput,
    SearchResults,
    Rollup,
    Composer,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    ChangeAdded,
    ChangeRemoved,
    FocusSelection,
    Attention,
    MutedResolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOverlay {
    pub context: String,
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_column: usize,
    pub scroll: usize,
    pub height: u16,
    pub message: Option<String>,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpOverlay {
    pub lines: Vec<String>,
    pub scroll: usize,
    pub position_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadOverlay {
    pub id: ThreadId,
    pub context: String,
    pub state: ThreadState,
    pub outdated: bool,
    pub position: String,
    pub messages: Vec<ThreadMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessage {
    pub author: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Composer(ComposerOverlay),
    Help(HelpOverlay),
    Thread(ThreadOverlay),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutPolicy {
    pub focus: FocusArea,
    pub diff_layout: LayoutMode,
    pub wrap_lines: bool,
}
