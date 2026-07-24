use crate::{
    diff::DiffLine,
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
    pub focused: bool,
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
    pub focused: bool,
    pub scroll: u16,
    pub files: Vec<ReviewFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFile {
    pub path: String,
    pub extension: Option<String>,
    pub metadata: Vec<String>,
    pub hunks: Vec<ReviewHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewHunk {
    pub header: Option<String>,
    pub selected: bool,
    pub lines: Vec<DiffLine>,
    pub threads: Vec<ThreadCard>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCard {
    pub id: ThreadId,
    pub state: ThreadState,
    pub outdated: bool,
    pub active: bool,
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
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Composer { input: String, replying: bool },
    Help { text: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutPolicy {
    pub focus: FocusArea,
    pub diff_layout: LayoutMode,
    pub wrap_lines: bool,
}
