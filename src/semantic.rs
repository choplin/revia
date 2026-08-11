use crate::{
    anchor::HunkLocation,
    diff::{DiffLine, HunkCoordinates},
    thread::ThreadId,
    ui::{FocusArea, LayoutMode},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub file_count: usize,
    pub needs_attention: usize,
    pub open: usize,
    pub resolved: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRail {
    pub selected: Option<usize>,
    pub items: Vec<FileItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    pub path: String,
    pub hunk_count: usize,
    pub thread_count: usize,
    pub attention: FileAttention,
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
    Review(ReviewBody),
    Rollup(RollupBody),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBody {
    pub scroll: usize,
    pub viewport: ReviewViewport,
    pub empty_state: Option<String>,
    pub search_target: Option<DiffSearchTarget>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickyReviewContext {
    pub file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub hunk_header: Option<String>,
    pub hunk_index: Option<usize>,
    pub hunk_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFile {
    pub path: String,
    pub selected: bool,
    pub extension: Option<String>,
    pub metadata: Vec<String>,
    pub hunks: Vec<ReviewHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewHunk {
    pub anchor: HunkLocation,
    pub header: Option<String>,
    pub coordinates: Option<HunkCoordinates>,
    pub selected: bool,
    pub lines: Vec<DiffLine>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Composer(ComposerOverlay),
    Help { text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutPolicy {
    pub focus: FocusArea,
    pub diff_layout: LayoutMode,
    pub wrap_lines: bool,
}
