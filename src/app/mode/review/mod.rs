//! Diff review mode-local Elm program.

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use crate::{
    app::{
        input::{BindingResolution, Key, PhysicalInput},
        view::{
            Body, DiffSearchTarget, FileAttention, FileRail, FileRailRow, FileRailRowKind,
            LayoutPolicy, ReviewBody, ReviewFile, ReviewHunk, ReviewViewport, ThreadCard,
            ThreadState as SemanticThreadState,
        },
        view_state::{FocusArea, LayoutMode, ShellSize, ViewState, review_body_width},
    },
    domain::{
        anchor::HunkLocation,
        diff::{DiffHunk, DiffLineKind, DiffRequest, HunkCoordinates, HunkRange, LoadedDiff},
        review::{ReviewCursor, ReviewSession},
        thread::{
            Resolution, ReviewThread, ThreadChange, ThreadId, ThreadOperation,
            ThreadState as Threads, ThreadSuccess,
        },
    },
    presentation::{self, ReviewRowMap, ViewportAnchor},
};
use unicode_segmentation::UnicodeSegmentation;

const TARGET_REVEAL_MARGIN: usize = 2;

#[derive(Debug)]
pub struct Model {
    request: DiffRequest,
    session: ReviewSession,
    view: ViewState,
    sidebar_visible: bool,
    show_hunk_headers: bool,
    wrap_lines: bool,
    viewport_rows: u16,
    viewport_columns: u16,
    search: Option<SearchState>,
    expanded_threads: BTreeSet<ThreadId>,
    file_tree: bool,
    collapsed_directories: BTreeSet<String>,
    file_rail_target: Option<FileRailTarget>,
    filter: ReviewFilter,
    projection_revision: u64,
    row_map_cache: RefCell<Option<RowMapCache>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowMapKey {
    projection_revision: u64,
    presentation_width: u16,
    focus: FocusArea,
    layout: LayoutMode,
    wrap_lines: bool,
    show_hunk_headers: bool,
    filter: ReviewFilter,
    selected_file: usize,
    selected_hunk: usize,
    selected_thread: usize,
    expanded_threads: BTreeSet<ThreadId>,
}

#[derive(Debug, Clone)]
struct RowMapCache {
    key: RowMapKey,
    rows: Rc<ReviewRowMap>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReviewFilter {
    #[default]
    AllChanges,
    NeedsAttention,
    OpenThreads,
    ThreadedHunks,
}

impl ReviewFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::AllChanges => "All changes",
            Self::NeedsAttention => "Needs attention",
            Self::OpenThreads => "Open threads",
            Self::ThreadedHunks => "Threaded hunks",
        }
    }

    fn next(self, direction: i32) -> Self {
        let filters = [
            Self::AllChanges,
            Self::NeedsAttention,
            Self::OpenThreads,
            Self::ThreadedHunks,
        ];
        let current = filters
            .iter()
            .position(|filter| *filter == self)
            .unwrap_or(0);
        filters[wrapped_index(current, filters.len(), direction)]
    }
}

#[derive(Debug, Clone)]
struct SemanticSelection {
    location: Option<HunkLocation>,
    thread_id: Option<ThreadId>,
    raw_hunk_position: usize,
    thread_position: usize,
}

#[derive(Debug)]
struct SearchState {
    query: String,
    entries: Vec<SearchEntry>,
    matches: Vec<DiffSearchTarget>,
    selected: Option<usize>,
    editing: bool,
    origin: SearchOrigin,
}

#[derive(Debug)]
struct SearchEntry {
    normalized: String,
    target: DiffSearchTarget,
}

#[derive(Debug, Clone)]
struct SearchOrigin {
    cursor: ReviewCursor,
    focus: FocusArea,
    viewport_anchor: ViewportAnchor,
    scroll: usize,
    scroll_from_end: Option<usize>,
    geometry: SearchGeometry,
    filter: ReviewFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchGeometry {
    viewport_rows: u16,
    viewport_columns: u16,
    sidebar_visible: bool,
    show_hunk_headers: bool,
    wrap_lines: bool,
    layout: LayoutMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileRailTarget {
    Directory(String),
    File(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileRailNode {
    label: String,
    path: String,
    depth: usize,
    file_index: Option<usize>,
    children: Vec<FileRailNode>,
}

impl FileRailNode {
    fn directory(label: String, path: String) -> Self {
        Self {
            label,
            path,
            depth: 0,
            file_index: None,
            children: Vec::new(),
        }
    }

    fn file(label: String, path: String, depth: usize, file_index: usize) -> Self {
        Self {
            label,
            path,
            depth,
            file_index: Some(file_index),
            children: Vec::new(),
        }
    }

    fn target(&self) -> FileRailTarget {
        if self.file_index.is_some() {
            FileRailTarget::File(self.path.clone())
        } else {
            FileRailTarget::Directory(self.path.clone())
        }
    }

    fn file_path(&self) -> Option<&str> {
        self.file_index.map(|_| self.path.as_str())
    }
}

fn insert_file_tree_branch(
    branches: &mut Vec<FileRailNode>,
    components: &[&str],
    parent: &str,
    file_index: usize,
) {
    let Some((component, remaining)) = components.split_first() else {
        return;
    };
    let path = if parent.is_empty() {
        (*component).to_owned()
    } else {
        format!("{parent}/{component}")
    };
    if remaining.is_empty() {
        branches.push(FileRailNode::file(
            (*component).to_owned(),
            path,
            0,
            file_index,
        ));
        return;
    }

    let directory = branches
        .iter()
        .position(|branch| branch.file_index.is_none() && branch.path == path)
        .unwrap_or_else(|| {
            branches.push(FileRailNode::directory(
                (*component).to_owned(),
                path.clone(),
            ));
            branches.len() - 1
        });
    insert_file_tree_branch(
        &mut branches[directory].children,
        remaining,
        &path,
        file_index,
    );
}

fn flatten_file_tree(
    branches: &[FileRailNode],
    depth: usize,
    collapsed: &BTreeSet<String>,
    rows: &mut Vec<FileRailNode>,
) {
    for branch in branches {
        let mut row = branch.clone();
        row.depth = depth;
        row.children.clear();
        rows.push(row);
        if branch.file_index.is_none() && !collapsed.contains(&branch.path) {
            flatten_file_tree(&branch.children, depth.saturating_add(1), collapsed, rows);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSummary {
    pub query: String,
    pub selected: Option<usize>,
    pub match_count: usize,
    pub editing: bool,
}

impl Model {
    pub fn new(request: DiffRequest, diff: LoadedDiff) -> Self {
        Self {
            request,
            session: ReviewSession::new(diff),
            view: ViewState::default(),
            sidebar_visible: true,
            show_hunk_headers: true,
            wrap_lines: false,
            viewport_rows: 20,
            viewport_columns: 120,
            search: None,
            expanded_threads: BTreeSet::new(),
            file_tree: true,
            collapsed_directories: BTreeSet::new(),
            file_rail_target: None,
            filter: ReviewFilter::AllChanges,
            projection_revision: 0,
            row_map_cache: RefCell::new(None),
        }
    }

    pub fn request(&self) -> &DiffRequest {
        &self.request
    }

    pub fn session(&self) -> &ReviewSession {
        &self.session
    }

    /// The comparison identity shown in the changeset header.
    pub fn comparison(&self) -> String {
        self.request.source.comparison()
    }

    pub fn focus(&self) -> FocusArea {
        self.view.focus
    }

    #[cfg(test)]
    pub fn scroll(&self) -> usize {
        self.view.scroll
    }

    pub fn set_viewport(&mut self, rows: u16, columns: u16, threads: &Threads) {
        self.change_geometry(threads, |model| {
            model.viewport_rows = rows.max(1);
            model.viewport_columns = columns;
            model.release_file_rail_focus();
        });
    }

    /// Whether the file rail actually occupies screen space.  `sidebar_visible`
    /// only records the user's intent; a narrow shell drops the rail regardless.
    pub fn file_rail_visible(&self) -> bool {
        self.sidebar_visible
            && ShellSize::for_width(self.viewport_columns)
                .rail_width(self.viewport_columns)
                .is_some()
    }

    /// Focus must never rest on a region the shell is not drawing.
    fn release_file_rail_focus(&mut self) {
        if self.view.focus == FocusArea::Files && !self.file_rail_visible() {
            self.view.focus = FocusArea::Review;
        }
    }

    pub fn viewport_columns(&self) -> u16 {
        self.viewport_columns
    }

    pub fn filter(&self) -> ReviewFilter {
        self.filter
    }

    pub fn search_summary(&self) -> Option<SearchSummary> {
        self.search.as_ref().map(|search| SearchSummary {
            query: search.query.clone(),
            selected: search.selected,
            match_count: search.matches.len(),
            editing: search.editing,
        })
    }

    pub fn help_context(&self) -> super::help::Context {
        if self.search.as_ref().is_some_and(|search| !search.editing) {
            super::help::Context::SearchResults
        } else {
            match self.focus() {
                FocusArea::Threads => super::help::Context::Threads,
                FocusArea::Files => super::help::Context::Files,
                FocusArea::Review => super::help::Context::Review,
            }
        }
    }

    pub fn selected_location(&self) -> Option<HunkLocation> {
        self.session.selected_location()
    }

    pub fn projected_location(&self, threads: &Threads) -> Option<HunkLocation> {
        let location = self.selected_location()?;
        self.hunk_matches_filter(&location, threads)
            .then_some(location)
    }

    fn hunk_matches_filter(&self, location: &HunkLocation, threads: &Threads) -> bool {
        let threads = self.threads_at_current(location, threads);
        match self.filter {
            ReviewFilter::AllChanges => true,
            ReviewFilter::NeedsAttention => threads.iter().any(|thread| thread.needs_attention),
            ReviewFilter::OpenThreads => threads
                .iter()
                .any(|thread| matches!(thread.resolution, Resolution::Open)),
            ReviewFilter::ThreadedHunks => !threads.is_empty(),
        }
    }

    fn thread_matches_filter(&self, thread: &ReviewThread) -> bool {
        match self.filter {
            ReviewFilter::AllChanges | ReviewFilter::ThreadedHunks => true,
            ReviewFilter::NeedsAttention => thread.needs_attention,
            ReviewFilter::OpenThreads => matches!(thread.resolution, Resolution::Open),
        }
    }

    fn visible_hunks(&self, threads: &Threads) -> Vec<(usize, usize)> {
        self.session
            .diff()
            .document
            .files
            .iter()
            .enumerate()
            .flat_map(|(file_index, file)| {
                file.hunks
                    .iter()
                    .enumerate()
                    .filter_map(move |(hunk_index, hunk)| {
                        let location = HunkLocation::new(&file.path, &hunk.header);
                        self.hunk_matches_filter(&location, threads)
                            .then_some((file_index, hunk_index))
                    })
            })
            .collect()
    }

    fn visible_files(&self, threads: &Threads) -> Vec<usize> {
        if self.filter == ReviewFilter::AllChanges {
            return (0..self.session.diff().document.files.len()).collect();
        }
        let mut files = Vec::new();
        for (file, _) in self.visible_hunks(threads) {
            if files.last() != Some(&file) {
                files.push(file);
            }
        }
        files
    }

    fn file_rail_rows(&self, threads: &Threads) -> Vec<FileRailNode> {
        let visible_files = self.visible_files(threads);
        if !self.file_tree {
            return visible_files
                .into_iter()
                .filter_map(|file_index| {
                    let file = self.session.diff().document.files.get(file_index)?;
                    Some(FileRailNode::file(
                        file.path.clone(),
                        file.path.clone(),
                        0,
                        file_index,
                    ))
                })
                .collect();
        }

        let mut roots = Vec::new();
        for file_index in visible_files {
            let Some(file) = self.session.diff().document.files.get(file_index) else {
                continue;
            };
            let components = file
                .path
                .split('/')
                .filter(|component| !component.is_empty())
                .collect::<Vec<_>>();
            let components = if components.is_empty() {
                vec![file.path.as_str()]
            } else {
                components
            };
            insert_file_tree_branch(&mut roots, &components, "", file_index);
        }
        if roots.len() > 1 {
            let mut root = FileRailNode::directory("/".into(), "/".into());
            root.children = roots;
            roots = vec![root];
        }

        let mut rows = Vec::new();
        flatten_file_tree(&roots, 0, &self.collapsed_directories, &mut rows);
        rows
    }

    fn resolved_file_rail_target(&self, rows: &[FileRailNode]) -> Option<(usize, FileRailTarget)> {
        self.file_rail_target
            .as_ref()
            .filter(|_| self.view.focus == FocusArea::Files)
            .and_then(|target| {
                rows.iter()
                    .position(|row| row.target() == *target)
                    .map(|position| (position, target.clone()))
            })
            .or_else(|| {
                let path = &self
                    .session
                    .diff()
                    .document
                    .files
                    .get(self.session.cursor().selected_file())?
                    .path;
                rows.iter()
                    .position(|row| row.file_path() == Some(path.as_str()))
                    .map(|position| (position, FileRailTarget::File(path.clone())))
            })
            .or_else(|| rows.first().map(|row| (0, row.target())))
    }

    fn visible_threads_at<'a>(
        &self,
        location: &HunkLocation,
        threads: &'a Threads,
    ) -> Vec<&'a ReviewThread> {
        self.threads_at_current(location, threads)
            .into_iter()
            .filter(|thread| self.thread_matches_filter(thread))
            .collect()
    }

    fn threads_at_current<'a>(
        &self,
        location: &HunkLocation,
        threads: &'a Threads,
    ) -> Vec<&'a ReviewThread> {
        threads
            .threads()
            .iter()
            .filter(|thread| {
                self.resolve_current_location(&thread.anchor.location())
                    .as_ref()
                    == Some(location)
            })
            .collect()
    }

    fn resolve_current_location(&self, anchor: &HunkLocation) -> Option<HunkLocation> {
        let file = self
            .session
            .diff()
            .document
            .files
            .iter()
            .find(|file| file.path == anchor.path())?;
        if let Some(hunk) = file
            .hunks
            .iter()
            .find(|hunk| hunk.header == anchor.hunk_header())
        {
            return Some(HunkLocation::new(&file.path, &hunk.header));
        }
        let anchor_coordinates = HunkCoordinates::parse(anchor.hunk_header())?;
        file.hunks
            .iter()
            .filter_map(|hunk| {
                let candidate = hunk.coordinates?;
                hunk_change_overlaps(anchor_coordinates, hunk).then(|| {
                    let distance = anchor_coordinates
                        .old
                        .start
                        .abs_diff(candidate.old.start)
                        .saturating_add(anchor_coordinates.new.start.abs_diff(candidate.new.start));
                    (distance, hunk)
                })
            })
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, hunk)| HunkLocation::new(&file.path, &hunk.header))
    }

    fn semantic_selection(&self, threads: &Threads) -> SemanticSelection {
        let location = self.selected_location();
        let visible_threads = location
            .as_ref()
            .map(|location| self.visible_threads_at(location, threads))
            .unwrap_or_default();
        let thread_position = self.session.cursor().selected_thread();
        let thread_id = visible_threads.get(thread_position).map(|thread| thread.id);
        let raw_hunk_position = self
            .session
            .diff()
            .document
            .files
            .iter()
            .take(self.session.cursor().selected_file())
            .map(|file| file.hunks.len())
            .sum::<usize>()
            .saturating_add(self.session.cursor().selected_hunk());
        SemanticSelection {
            location,
            thread_id,
            raw_hunk_position,
            thread_position,
        }
    }

    fn reconcile_projection(&mut self, threads: &Threads, preferred: SemanticSelection) {
        let visible = self.visible_hunks(threads);
        if visible.is_empty() {
            self.view.focus = FocusArea::Review;
            self.view.set_scroll(0);
            return;
        }
        let exact = preferred.location.as_ref().and_then(|location| {
            visible.iter().position(|(file_index, hunk_index)| {
                self.session
                    .diff()
                    .document
                    .files
                    .get(*file_index)
                    .and_then(|file| file.hunks.get(*hunk_index).map(|hunk| (file, hunk)))
                    .is_some_and(|(file, hunk)| {
                        file.path == location.path() && hunk.header == location.hunk_header()
                    })
            })
        });
        let target_position = exact.unwrap_or_else(|| {
            visible
                .iter()
                .enumerate()
                .min_by_key(|(_, (file_index, hunk_index))| {
                    let position = self
                        .session
                        .diff()
                        .document
                        .files
                        .iter()
                        .take(*file_index)
                        .map(|file| file.hunks.len())
                        .sum::<usize>()
                        .saturating_add(*hunk_index);
                    position.abs_diff(preferred.raw_hunk_position)
                })
                .map(|(index, _)| index)
                .unwrap_or(0)
        });
        let (file_index, hunk_index) = visible[target_position];
        self.session.select_hunk(file_index, hunk_index);
        let location = self.session.selected_location();
        let visible_threads = location
            .as_ref()
            .map(|location| self.visible_threads_at(location, threads))
            .unwrap_or_default();
        if self.view.focus == FocusArea::Threads {
            if visible_threads.is_empty() {
                self.view.focus = FocusArea::Review;
            } else {
                let thread_index = preferred
                    .thread_id
                    .and_then(|id| visible_threads.iter().position(|thread| thread.id == id))
                    .unwrap_or_else(|| {
                        preferred
                            .thread_position
                            .min(visible_threads.len().saturating_sub(1))
                    });
                self.session.select_thread(thread_index);
            }
        }
    }

    pub fn reconcile_replaced_threads(&mut self, previous: &Threads, replacement: &Threads) {
        let selection = self.semantic_selection(previous);
        let anchor = self.viewport_anchor(previous);
        self.invalidate_projection();
        self.reconcile_projection(replacement, selection);
        self.restore_viewport(anchor, replacement);
        self.reveal_selected_target(replacement);
    }

    fn set_filter(&mut self, filter: ReviewFilter, threads: &Threads) {
        let selection = self.semantic_selection(threads);
        let anchor = self.viewport_anchor(threads);
        self.filter = filter;
        self.reconcile_projection(threads, selection);
        self.restore_viewport(anchor, threads);
        self.reveal_selected_target(threads);
    }

    pub fn select_thread_location(
        &mut self,
        id: ThreadId,
        location: &HunkLocation,
        threads: &Threads,
    ) -> Result<bool, String> {
        let Some(current_location) = self.resolve_current_location(location) else {
            return Err(format!("thread #{id} hunk is not in this diff"));
        };
        let Some((file_index, file)) = self
            .session
            .diff()
            .document
            .files
            .iter()
            .enumerate()
            .find(|(_, file)| file.path == current_location.path())
        else {
            return Err(format!("thread #{id} anchor is not in this diff"));
        };
        let Some(hunk_index) = file
            .hunks
            .iter()
            .position(|hunk| hunk.header == current_location.hunk_header())
        else {
            return Err(format!("thread #{id} hunk is not in this diff"));
        };
        self.session.select_hunk(file_index, hunk_index);
        let filter_was_reset = !self.hunk_matches_filter(&current_location, threads)
            || !self
                .visible_threads_at(&current_location, threads)
                .iter()
                .any(|thread| thread.id == id);
        if filter_was_reset {
            self.filter = ReviewFilter::AllChanges;
        }
        self.session.select_thread(
            self.visible_threads_at(&current_location, threads)
                .iter()
                .position(|thread| thread.id == id)
                .unwrap_or(0),
        );
        self.view.focus = FocusArea::Threads;
        self.reveal_selected_target(threads);
        Ok(filter_was_reset)
    }

    fn selected_threads<'a>(&self, threads: &'a Threads) -> Vec<&'a ReviewThread> {
        self.projected_location(threads)
            .map(|location| self.visible_threads_at(&location, threads))
            .unwrap_or_default()
    }

    fn current_thread_id(&self, threads: &Threads) -> Option<ThreadId> {
        let selected = self.selected_threads(threads);
        selected
            .get(self.session.cursor().selected_thread())
            .or_else(|| selected.last())
            .map(|thread| thread.id)
    }

    pub fn selected_thread_id(&self, threads: &Threads) -> Option<ThreadId> {
        self.current_thread_id(threads)
    }

    pub fn selected_thread_position(&self, threads: &Threads) -> Option<(usize, usize)> {
        let selected = self.selected_threads(threads);
        (!selected.is_empty()).then(|| {
            (
                self.session
                    .cursor()
                    .selected_thread()
                    .min(selected.len().saturating_sub(1)),
                selected.len(),
            )
        })
    }

    pub fn focus_thread(&mut self, id: ThreadId, threads: &Threads) -> bool {
        let Some(index) = self
            .selected_threads(threads)
            .iter()
            .position(|thread| thread.id == id)
        else {
            return false;
        };
        self.change_geometry(threads, |model| {
            model.session.select_thread(index);
            model.view.focus = FocusArea::Threads;
        });
        self.reveal_selected_target(threads);
        true
    }

    pub fn selected_target_label(&self, threads: &Threads) -> Option<String> {
        let location = self.projected_location(threads)?;
        let file = self
            .session
            .diff()
            .document
            .files
            .iter()
            .find(|file| file.path == location.path())?;
        let visible_hunks = file
            .hunks
            .iter()
            .filter(|hunk| {
                self.hunk_matches_filter(&HunkLocation::new(&file.path, &hunk.header), threads)
            })
            .collect::<Vec<_>>();
        let hunk_index = visible_hunks
            .iter()
            .position(|hunk| hunk.header == location.hunk_header())?;
        Some(format!(
            "{} • hunk {}/{}",
            file.path,
            hunk_index + 1,
            visible_hunks.len()
        ))
    }

    fn selected_file_label(&self, threads: &Threads) -> Option<String> {
        let selected_file = self.session.cursor().selected_file();
        let visible_files = self.visible_files(threads);
        let position = visible_files
            .iter()
            .position(|file| *file == selected_file)?;
        let file = self.session.diff().document.files.get(selected_file)?;
        Some(format!(
            "file {}/{}: {}",
            position + 1,
            visible_files.len(),
            file.path
        ))
    }

    fn visible_rows_for(&self, rows: &ReviewRowMap) -> usize {
        let sticky_rows = rows.sticky_rows();
        usize::from(self.viewport_rows)
            .saturating_sub(sticky_rows)
            .max(1)
    }

    fn row_map_key(&self) -> RowMapKey {
        RowMapKey {
            projection_revision: self.projection_revision,
            presentation_width: review_body_width(self.viewport_columns, self.sidebar_visible),
            focus: self.focus(),
            layout: self.view.layout,
            wrap_lines: self.wrap_lines,
            show_hunk_headers: self.show_hunk_headers,
            filter: self.filter,
            selected_file: self.session.cursor().selected_file(),
            selected_hunk: self.session.cursor().selected_hunk(),
            selected_thread: self.session.cursor().selected_thread(),
            expanded_threads: self.expanded_threads.clone(),
        }
    }

    fn row_map(&self, threads: &Threads) -> Rc<ReviewRowMap> {
        let key = self.row_map_key();
        if let Some(cache) = self
            .row_map_cache
            .borrow()
            .as_ref()
            .filter(|cache| cache.key == key)
        {
            return Rc::clone(&cache.rows);
        }
        let body = review_body(self, threads);
        let rows = Rc::new(presentation::review_row_map(
            &body,
            key.presentation_width,
            LayoutPolicy {
                focus: self.focus(),
                diff_layout: self.view.layout,
                wrap_lines: self.wrap_lines,
            },
        ));
        self.row_map_cache.replace(Some(RowMapCache {
            key,
            rows: Rc::clone(&rows),
        }));
        rows
    }

    fn invalidate_projection(&mut self) {
        self.projection_revision = self.projection_revision.wrapping_add(1);
        self.row_map_cache.get_mut().take();
    }

    fn viewport_anchor(&self, threads: &Threads) -> ViewportAnchor {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        let top = self.view.resolved_scroll(rows.total_rows(), visible_rows);
        rows.anchor_at(top)
    }

    fn restore_viewport(&mut self, anchor: ViewportAnchor, threads: &Threads) {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        let row = rows.row_for_anchor(&anchor);
        self.view.set_scroll(row);
        self.view.clamp(rows.total_rows(), visible_rows);
    }

    fn reveal_selected_target(&mut self, threads: &Threads) {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        if let Some(row) = rows.selected_target_row() {
            self.view.reveal(row, visible_rows, TARGET_REVEAL_MARGIN);
            self.view.clamp(rows.total_rows(), visible_rows);
        }
    }

    fn align_selected_file_to_top(&mut self, threads: &Threads) {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        if let Some(row) = rows.selected_file_start_row() {
            self.view.set_scroll(row);
            self.view.clamp(rows.total_rows(), visible_rows);
        }
    }

    fn center_selected_hunk(&mut self, threads: &Threads) {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        let Some(range) = rows.selected_hunk_range() else {
            return;
        };
        let hunk_rows = range.end.saturating_sub(range.start);
        let scroll = if hunk_rows > visible_rows {
            range.start
        } else {
            range
                .start
                .saturating_sub(visible_rows.saturating_sub(hunk_rows) / 2)
        };
        self.view.set_scroll(scroll);
        self.view.clamp(rows.total_rows(), visible_rows);
    }

    fn follow_scroll_target(&mut self, threads: &Threads) {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        let top = self.view.resolved_scroll(rows.total_rows(), visible_rows);
        // Follow what dominates the viewport rather than a hunk whose last
        // line merely clips the top edge.
        let focus_row = top.saturating_add(visible_rows / 2);
        let Some(location) = rows.hunk_at(focus_row).cloned() else {
            return;
        };
        let target = self
            .session
            .diff()
            .document
            .files
            .iter()
            .enumerate()
            .find_map(|(file_index, file)| {
                (file.path == location.path()).then(|| {
                    file.hunks
                        .iter()
                        .position(|hunk| hunk.header == location.hunk_header())
                        .map(|hunk_index| (file_index, hunk_index))
                })?
            });
        if let Some((file, hunk)) = target
            && (file != self.session.cursor().selected_file()
                || hunk != self.session.cursor().selected_hunk())
        {
            self.session.select_hunk(file, hunk);
        }
    }

    fn selected_target_is_visible(&self, threads: &Threads) -> bool {
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        let top = self.view.resolved_scroll(rows.total_rows(), visible_rows);
        rows.selected_target_row()
            .is_some_and(|selected| selected >= top && selected < top.saturating_add(visible_rows))
    }

    fn change_geometry(&mut self, threads: &Threads, change: impl FnOnce(&mut Self)) {
        let selected_was_visible = self.selected_target_is_visible(threads);
        let preserve_end_relative = self.view.scroll_from_end.is_some();
        let anchor = self
            .view
            .scroll_from_end
            .is_none()
            .then(|| self.viewport_anchor(threads));
        change(self);
        if let Some(anchor) = anchor {
            self.restore_viewport(anchor, threads);
            if selected_was_visible {
                self.reveal_selected_target(threads);
            }
        } else if selected_was_visible && !self.selected_target_is_visible(threads) {
            self.reveal_selected_target(threads);
            if preserve_end_relative {
                let rows = self.row_map(threads);
                let max_scroll = rows
                    .total_rows()
                    .saturating_sub(self.visible_rows_for(&rows));
                self.view.scroll_from_end = Some(max_scroll.saturating_sub(self.view.scroll));
            }
        }
        if let Some(target) = self.current_search_target().cloned() {
            self.reveal_search_target(&target, threads);
        }
    }

    fn search_geometry(&self) -> SearchGeometry {
        SearchGeometry {
            viewport_rows: self.viewport_rows,
            viewport_columns: self.viewport_columns,
            sidebar_visible: self.sidebar_visible,
            show_hunk_headers: self.show_hunk_headers,
            wrap_lines: self.wrap_lines,
            layout: self.view.layout,
        }
    }

    fn current_search_target(&self) -> Option<&DiffSearchTarget> {
        let search = self.search.as_ref()?;
        search.selected.and_then(|index| search.matches.get(index))
    }

    fn reveal_search_target(&mut self, target: &DiffSearchTarget, threads: &Threads) -> bool {
        let filter_was_reset = self.filter != ReviewFilter::AllChanges;
        if filter_was_reset {
            self.filter = ReviewFilter::AllChanges;
        }
        match target {
            DiffSearchTarget::FilePath { path } => {
                if let Some(file_index) = self
                    .session
                    .diff()
                    .document
                    .files
                    .iter()
                    .position(|file| &file.path == path)
                {
                    self.session.select_file(file_index);
                }
            }
            DiffSearchTarget::HunkHeader { location }
            | DiffSearchTarget::DiffLine { location, .. } => {
                let target = self
                    .session
                    .diff()
                    .document
                    .files
                    .iter()
                    .enumerate()
                    .find_map(|(file_index, file)| {
                        (file.path == location.path()).then(|| {
                            file.hunks
                                .iter()
                                .position(|hunk| hunk.header == location.hunk_header())
                                .map(|hunk_index| (file_index, hunk_index))
                        })?
                    });
                if let Some((file_index, hunk_index)) = target {
                    self.session.select_hunk(file_index, hunk_index);
                }
            }
        }
        // Landing on a match reselects the hunk, which can strand a thread
        // target; any other focus keeps its place.
        if self.view.focus == FocusArea::Threads {
            self.view.focus = FocusArea::Review;
        }
        let rows = self.row_map(threads);
        let visible_rows = self.visible_rows_for(&rows);
        if let Some(row) = rows.row_for_search_target(target) {
            self.view.reveal(row, visible_rows, TARGET_REVEAL_MARGIN);
            self.view.clamp(rows.total_rows(), visible_rows);
        }
        filter_was_reset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    CycleFilter(i32),
    ShowAllChanges,
    BeginSearch,
    InsertSearchCharacter(char),
    DeleteSearchCharacter,
    FinishSearch,
    CancelSearch,
    MoveSearch(i32),
    CycleFocus,
    PreviousFocus,
    ScrollRows(i16),
    ScrollViewport(i16),
    ScrollHalfViewport(i16),
    JumpToDiffEdge { end: bool },
    MoveHunk(i32),
    MoveFile(i32),
    MoveFileRail(i32),
    MoveFileRailPage(i32),
    JumpToFileRailEdge { end: bool },
    EnterFileRailTarget,
    ToggleFileTree,
    CollapseFileTree,
    ExpandFileTree,
    AdjustContext(i32),
    BeginThread { always_new: bool },
    MoveThread(i32),
    CloseThread,
    ReopenThread,
    ToggleAttention,
    ToggleOutdated,
    ToggleThreadExpansion,
    MoveAttention(i32),
    ShowRollup,
    SetLayout(LayoutMode),
    ToggleSidebar,
    ReloadDiff,
    ToggleHunkHeaders,
    ToggleWrap,
    EffectCompleted(Outcome),
}

impl Event {
    fn blocked_while_operation_pending(&self) -> bool {
        matches!(
            self,
            Self::AdjustContext(_)
                | Self::BeginThread { .. }
                | Self::CloseThread
                | Self::ReopenThread
                | Self::ToggleAttention
                | Self::ToggleOutdated
                | Self::MoveAttention(_)
                | Self::ReloadDiff
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadPurpose {
    ContextChanged,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReloadDiff {
        request: DiffRequest,
        purpose: ReloadPurpose,
    },
    ChangeThreads(ThreadOperation),
    ResolveThread {
        id: ThreadId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    DiffReloaded {
        purpose: ReloadPurpose,
        result: Result<LoadedDiff, String>,
    },
    ThreadsChanged {
        result: Result<ThreadChange, String>,
    },
    ThreadResolved {
        id: ThreadId,
        result: Result<HunkLocation, String>,
    },
}

pub struct UpdateInput<'a> {
    pub threads: &'a Threads,
    pub operation_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    SetStatus(String),
    ReplaceThreads(Threads),
    OpenComposer { reply_to: Option<ThreadId> },
    OpenRollup,
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

/// Keys that mean something different while the file rail holds focus. The
/// bindings mirror LazyGit's list and file-tree navigation where those actions
/// remain read-only. Returning `None` hands the key back to the shared table.
fn file_rail_binding(key: Key) -> Option<Event> {
    match key {
        Key::Char('j') | Key::Down => Some(Event::MoveFileRail(1)),
        Key::Char('k') | Key::Up => Some(Event::MoveFileRail(-1)),
        Key::Char('.') | Key::PageDown => Some(Event::MoveFileRailPage(1)),
        Key::Char(',') | Key::PageUp => Some(Event::MoveFileRailPage(-1)),
        Key::Char('>') | Key::End => Some(Event::JumpToFileRailEdge { end: true }),
        Key::Char('<') | Key::Home => Some(Event::JumpToFileRailEdge { end: false }),
        Key::Enter => Some(Event::EnterFileRailTarget),
        Key::Left | Key::Right => Some(Event::CycleFocus),
        Key::Char('`') => Some(Event::ToggleFileTree),
        Key::Char('-') => Some(Event::CollapseFileTree),
        Key::Char('=') => Some(Event::ExpandFileTree),
        _ => None,
    }
}

pub fn bindings(model: &Model, input: PhysicalInput) -> BindingResolution<Event> {
    if model.search.as_ref().is_some_and(|search| search.editing) {
        if input.phase == crate::app::input::KeyPhase::Release
            || input.phase == crate::app::input::KeyPhase::Repeat
                && matches!(input.key, Key::Esc | Key::Enter)
        {
            return BindingResolution::Consume;
        }
        return match input.key {
            Key::Esc => BindingResolution::Override(Event::CancelSearch),
            Key::Enter => BindingResolution::Handle(Event::FinishSearch),
            Key::Backspace => BindingResolution::Handle(Event::DeleteSearchCharacter),
            Key::Char(character) => {
                BindingResolution::Override(Event::InsertSearchCharacter(character))
            }
            _ => BindingResolution::Consume,
        };
    }
    if model.search.is_some()
        && matches!(input.key, Key::Esc)
        && input.phase == crate::app::input::KeyPhase::Press
    {
        return BindingResolution::Override(Event::CancelSearch);
    }
    if input.phase == crate::app::input::KeyPhase::Release {
        return BindingResolution::Consume;
    }
    if input.phase == crate::app::input::KeyPhase::Repeat
        && !matches!(
            input.key,
            Key::Char('j')
                | Key::Down
                | Key::Char('k')
                | Key::Up
                | Key::Char('f')
                | Key::PageDown
                | Key::Char('b')
                | Key::PageUp
                | Key::Char(' ')
                | Key::Char('d')
                | Key::Char('u')
                | Key::Char('g')
                | Key::Char('G')
                | Key::Home
                | Key::End
                | Key::Char(']')
                | Key::Char('[')
                | Key::Char('.')
                | Key::Char(',')
                | Key::Char('t')
                | Key::Char('T')
                | Key::Char('}')
                | Key::Char('{')
                | Key::Char('n')
                | Key::Char('N')
        )
    {
        return BindingResolution::Consume;
    }
    // Focus-scoped layer.  It is consulted before the shared table so a region
    // can claim a key without renaming the diff binding behind it; anything the
    // layer does not claim keeps its global meaning.
    if model.focus() == FocusArea::Files
        && let Some(event) = file_rail_binding(input.key)
    {
        return BindingResolution::Handle(event);
    }
    match input.key {
        Key::Esc if model.focus() == FocusArea::Threads => {
            BindingResolution::Handle(Event::CycleFocus)
        }
        Key::Char('q') | Key::Esc | Key::Char('?') => BindingResolution::Delegate,
        Key::Char('F') => BindingResolution::Handle(Event::CycleFilter(1)),
        Key::Char('A') => BindingResolution::Handle(Event::ShowAllChanges),
        Key::Char('/') => BindingResolution::Handle(Event::BeginSearch),
        Key::Char('n') => BindingResolution::Handle(Event::MoveSearch(1)),
        Key::Char('N') => BindingResolution::Handle(Event::MoveSearch(-1)),
        Key::Tab => BindingResolution::Handle(Event::CycleFocus),
        Key::BackTab => BindingResolution::Handle(Event::PreviousFocus),
        Key::Char('j') | Key::Down => BindingResolution::Handle(Event::ScrollRows(1)),
        Key::Char('k') | Key::Up => BindingResolution::Handle(Event::ScrollRows(-1)),
        Key::Char('f') | Key::PageDown => BindingResolution::Handle(Event::ScrollViewport(1)),
        Key::Char('b') | Key::PageUp => BindingResolution::Handle(Event::ScrollViewport(-1)),
        Key::Char(' ') if input.shift => BindingResolution::Handle(Event::ScrollViewport(-1)),
        Key::Char(' ') => BindingResolution::Handle(Event::ScrollViewport(1)),
        Key::Char('d') => BindingResolution::Handle(Event::ScrollHalfViewport(1)),
        Key::Char('u') => BindingResolution::Handle(Event::ScrollHalfViewport(-1)),
        Key::Char('g') | Key::Home => {
            BindingResolution::Handle(Event::JumpToDiffEdge { end: false })
        }
        Key::Char('G') | Key::End => BindingResolution::Handle(Event::JumpToDiffEdge { end: true }),
        Key::Char(']') => BindingResolution::Handle(Event::MoveHunk(1)),
        Key::Char('[') => BindingResolution::Handle(Event::MoveHunk(-1)),
        Key::Char('.') => BindingResolution::Handle(Event::MoveFile(1)),
        Key::Char(',') => BindingResolution::Handle(Event::MoveFile(-1)),
        Key::Char('=') => BindingResolution::Handle(Event::AdjustContext(1)),
        Key::Char('-') => BindingResolution::Handle(Event::AdjustContext(-1)),
        Key::Char('c') => BindingResolution::Handle(Event::BeginThread { always_new: false }),
        Key::Char('C') => BindingResolution::Handle(Event::BeginThread { always_new: true }),
        Key::Char('t') => BindingResolution::Handle(Event::MoveThread(1)),
        Key::Char('T') => BindingResolution::Handle(Event::MoveThread(-1)),
        Key::Char('x') => BindingResolution::Handle(Event::CloseThread),
        Key::Char('R') => BindingResolution::Handle(Event::ReopenThread),
        Key::Char('a') => BindingResolution::Handle(Event::ToggleAttention),
        Key::Char('o') => BindingResolution::Handle(Event::ToggleOutdated),
        Key::Char('e') => BindingResolution::Handle(Event::ToggleThreadExpansion),
        Key::Char('}') => BindingResolution::Handle(Event::MoveAttention(1)),
        Key::Char('{') => BindingResolution::Handle(Event::MoveAttention(-1)),
        Key::Char('v') => BindingResolution::Handle(Event::ShowRollup),
        Key::Char('1') => BindingResolution::Handle(Event::SetLayout(LayoutMode::Split)),
        Key::Char('2') => BindingResolution::Handle(Event::SetLayout(LayoutMode::Stack)),
        Key::Char('0') => BindingResolution::Handle(Event::SetLayout(LayoutMode::Auto)),
        Key::Char('s') => BindingResolution::Handle(Event::ToggleSidebar),
        Key::Char('r') => BindingResolution::Handle(Event::ReloadDiff),
        Key::Char('m') => BindingResolution::Handle(Event::ToggleHunkHeaders),
        Key::Char('w') => BindingResolution::Handle(Event::ToggleWrap),
        _ => BindingResolution::Unbound,
    }
}

pub fn update(model: &mut Model, event: Event, input: UpdateInput<'_>) -> Update {
    let mut result = Update::default();
    if input.operation_pending && event.blocked_while_operation_pending() {
        status(
            &mut result,
            "cannot start another action while an operation is pending",
        );
        return result;
    }
    match event {
        Event::CycleFilter(direction) => {
            let filter = model.filter.next(direction);
            let cleared_search = model.search.take().is_some();
            model.set_filter(filter, input.threads);
            status(
                &mut result,
                format!(
                    "filter: {}; F cycles filters, A shows all changes{}",
                    filter.label(),
                    if cleared_search {
                        "; cleared search"
                    } else {
                        ""
                    }
                ),
            );
        }
        Event::ShowAllChanges => {
            let cleared_search = model.search.take().is_some();
            model.set_filter(ReviewFilter::AllChanges, input.threads);
            status(
                &mut result,
                if cleared_search {
                    "filter: All changes; cleared search"
                } else {
                    "filter: All changes"
                },
            );
        }
        Event::BeginSearch => {
            let rows = model.row_map(input.threads);
            let visible_rows = model.visible_rows_for(&rows);
            let scroll = model.view.resolved_scroll(rows.total_rows(), visible_rows);
            model.search = Some(SearchState {
                query: String::new(),
                entries: diff_search_entries(model.session.diff()),
                matches: Vec::new(),
                selected: None,
                editing: true,
                origin: SearchOrigin {
                    cursor: model.session.cursor(),
                    focus: model.focus(),
                    viewport_anchor: rows.anchor_at(scroll),
                    scroll: model.view.scroll,
                    scroll_from_end: model.view.scroll_from_end,
                    geometry: model.search_geometry(),
                    filter: model.filter,
                },
            });
            status(&mut result, "search: type a query");
        }
        Event::InsertSearchCharacter(character) => {
            if let Some(search) = model.search.as_mut() {
                search.query.push(character);
                refresh_search(model, input.threads, &mut result);
            }
        }
        Event::DeleteSearchCharacter => {
            if let Some(search) = model.search.as_mut() {
                if let Some((index, _)) = search.query.grapheme_indices(true).next_back() {
                    search.query.truncate(index);
                }
                refresh_search(model, input.threads, &mut result);
            }
        }
        Event::FinishSearch => {
            let Some(search) = model.search.as_mut() else {
                status(&mut result, "no active search; press / to search");
                return result;
            };
            if search.query.is_empty() {
                status(
                    &mut result,
                    "search query is empty; type text or press Esc to cancel",
                );
            } else {
                search.editing = false;
                status(
                    &mut result,
                    search_position_status(search, "search ready; n/N wrap through matches"),
                );
            }
        }
        Event::CancelSearch => {
            let Some(search) = model.search.take() else {
                status(&mut result, "no active search to cancel");
                return result;
            };
            restore_search_origin(model, search.origin, input.threads);
            status(
                &mut result,
                "cancelled search; restored previous review position",
            );
        }
        Event::MoveSearch(direction) => {
            move_search(model, direction, input.threads, &mut result);
        }
        Event::CycleFocus | Event::PreviousFocus => {
            move_focus(model, input.threads, &mut result);
        }
        Event::ScrollRows(delta) => {
            model.view.scroll_by(delta);
            let rows = model.row_map(input.threads);
            let visible_rows = model.visible_rows_for(&rows);
            model.view.clamp(rows.total_rows(), visible_rows);
            model.follow_scroll_target(input.threads);
            status(&mut result, scroll_status(&model.view));
        }
        Event::ScrollViewport(direction) => {
            let rows = model.row_map(input.threads);
            let visible_rows = model.visible_rows_for(&rows);
            let viewport = visible_rows.min(i16::MAX as usize) as i16;
            model.view.scroll_by(direction.saturating_mul(viewport));
            model.view.clamp(rows.total_rows(), visible_rows);
            model.follow_scroll_target(input.threads);
            status(&mut result, scroll_status(&model.view));
        }
        Event::ScrollHalfViewport(direction) => {
            let rows = model.row_map(input.threads);
            let visible_rows = model.visible_rows_for(&rows);
            let half_viewport = (visible_rows / 2).max(1).min(i16::MAX as usize) as i16;
            model
                .view
                .scroll_by(direction.saturating_mul(half_viewport));
            model.view.clamp(rows.total_rows(), visible_rows);
            model.follow_scroll_target(input.threads);
            status(&mut result, scroll_status(&model.view));
        }
        Event::JumpToDiffEdge { end } => {
            if end {
                model.view.jump_to_end();
            } else {
                model.view.set_scroll(0);
            }
            model.follow_scroll_target(input.threads);
            status(&mut result, if end { "diff end" } else { "diff start" });
        }
        Event::MoveHunk(direction) => {
            let hunks = model.visible_hunks(input.threads);
            let current = hunks.iter().position(|(file, hunk)| {
                *file == model.session.cursor().selected_file()
                    && *hunk == model.session.cursor().selected_hunk()
            });
            if let Some(target) = (!hunks.is_empty()).then(|| {
                current.map_or_else(
                    || {
                        if direction < 0 {
                            hunks
                                .iter()
                                .rposition(|(file, _)| {
                                    *file < model.session.cursor().selected_file()
                                })
                                .unwrap_or(hunks.len() - 1)
                        } else {
                            hunks
                                .iter()
                                .position(|(file, _)| {
                                    *file > model.session.cursor().selected_file()
                                })
                                .unwrap_or(0)
                        }
                    },
                    |current| wrapped_index(current, hunks.len(), direction),
                )
            }) {
                let (file, hunk) = hunks[target];
                model.session.select_hunk(file, hunk);
                model.center_selected_hunk(input.threads);
                status(
                    &mut result,
                    format!(
                        "hunk target: {}",
                        model
                            .selected_target_label(input.threads)
                            .unwrap_or_else(|| "unavailable".into())
                    ),
                );
            } else {
                status(
                    &mut result,
                    if model.filter == ReviewFilter::AllChanges {
                        "cannot move hunks: this diff has no hunks".into()
                    } else {
                        format!(
                            "cannot move hunks: no targets in {}; press A for All changes",
                            model.filter.label()
                        )
                    },
                );
            }
        }
        Event::MoveFile(direction) => {
            let files = model.visible_files(input.threads);
            let current = files
                .iter()
                .position(|file| *file == model.session.cursor().selected_file());
            if let Some(target) = (!files.is_empty()).then(|| {
                current.map_or_else(
                    || if direction < 0 { files.len() - 1 } else { 0 },
                    |current| wrapped_index(current, files.len(), direction),
                )
            }) {
                let file = files[target];
                if let Some((_, hunk)) = model
                    .visible_hunks(input.threads)
                    .into_iter()
                    .find(|(candidate, _)| *candidate == file)
                {
                    model.session.select_hunk(file, hunk);
                } else {
                    model.session.select_file(file);
                }
                model.align_selected_file_to_top(input.threads);
                status(
                    &mut result,
                    format!(
                        "file target: {}",
                        model
                            .selected_file_label(input.threads)
                            .unwrap_or_else(|| "unavailable".into())
                    ),
                );
            } else {
                status(
                    &mut result,
                    if model.filter == ReviewFilter::AllChanges {
                        "cannot move files: this diff has no changed files".into()
                    } else {
                        format!(
                            "cannot move files: no targets in {}; press A for All changes",
                            model.filter.label()
                        )
                    },
                );
            }
        }
        Event::MoveFileRail(direction) => {
            move_file_rail(model, direction, input.threads, &mut result);
        }
        Event::MoveFileRailPage(direction) => {
            let page = usize::from(model.viewport_rows.saturating_sub(5).max(1));
            let distance = i32::try_from(page).unwrap_or(i32::MAX);
            move_file_rail(
                model,
                direction.saturating_mul(distance),
                input.threads,
                &mut result,
            );
        }
        Event::JumpToFileRailEdge { end } => {
            let rows = model.file_rail_rows(input.threads);
            if let Some(row) = if end { rows.last() } else { rows.first() } {
                select_file_rail_node(model, row, input.threads);
                status(&mut result, if end { "files end" } else { "files start" });
            } else {
                status(
                    &mut result,
                    "cannot move files: this diff has no changed files",
                );
            }
        }
        Event::EnterFileRailTarget => {
            let rows = model.file_rail_rows(input.threads);
            let Some((_, target)) = model.resolved_file_rail_target(&rows) else {
                status(
                    &mut result,
                    "cannot enter files: this diff has no changed files",
                );
                return result;
            };
            match target {
                FileRailTarget::Directory(path) => {
                    let collapsed = if model.collapsed_directories.remove(&path) {
                        false
                    } else {
                        model.collapsed_directories.insert(path.clone());
                        true
                    };
                    model.file_rail_target = Some(FileRailTarget::Directory(path.clone()));
                    status(
                        &mut result,
                        format!(
                            "{} directory: {path}",
                            if collapsed { "collapsed" } else { "expanded" }
                        ),
                    );
                }
                FileRailTarget::File(path) => {
                    model.view.focus = FocusArea::Review;
                    status(&mut result, format!("diff focused: {path}"));
                }
            }
        }
        Event::ToggleFileTree => {
            if model.file_tree {
                if let Some(FileRailTarget::Directory(path)) = model.file_rail_target.as_ref() {
                    let prefix = format!("{path}/");
                    if let Some(file) = model
                        .visible_files(input.threads)
                        .into_iter()
                        .filter_map(|index| model.session.diff().document.files.get(index))
                        .find(|file| file.path.starts_with(&prefix))
                    {
                        model.file_rail_target = Some(FileRailTarget::File(file.path.clone()));
                    }
                }
                model.file_tree = false;
                status(&mut result, "files: flat view");
            } else {
                model.file_tree = true;
                status(&mut result, "files: tree view");
            }
        }
        Event::CollapseFileTree => {
            model.file_tree = true;
            model.collapsed_directories = all_file_directories(model, input.threads);
            let rows = model.file_rail_rows(input.threads);
            if model.resolved_file_rail_target(&rows).is_none() {
                model.file_rail_target = rows.first().map(FileRailNode::target);
            }
            status(&mut result, "collapsed all file directories");
        }
        Event::ExpandFileTree => {
            model.file_tree = true;
            model.collapsed_directories.clear();
            status(&mut result, "expanded all file directories");
        }
        Event::AdjustContext(delta) => {
            let context = model.request.context_lines as i32 + delta;
            if context >= 0 {
                model.request.context_lines = context as usize;
                result.effects.push(Effect::ReloadDiff {
                    request: model.request.clone(),
                    purpose: ReloadPurpose::ContextChanged,
                });
            } else {
                status(&mut result, "context already has 0 lines");
            }
        }
        Event::BeginThread { always_new } => {
            if model.projected_location(input.threads).is_none() {
                status(
                    &mut result,
                    if model.filter == ReviewFilter::AllChanges {
                        "select a hunk before posting a thread".into()
                    } else {
                        format!(
                            "cannot post: no target in {}; press A for All changes or F to cycle filters",
                            model.filter.label()
                        )
                    },
                );
            } else {
                let reply_to = if !always_new && model.focus() == FocusArea::Threads {
                    model.current_thread_id(input.threads)
                } else {
                    None
                };
                status(
                    &mut result,
                    reply_to.map_or_else(
                        || "writing a new comment".into(),
                        |id| format!("replying to thread #{id}"),
                    ),
                );
                result.intents.push(Intent::OpenComposer { reply_to });
            }
        }
        Event::MoveThread(direction) => {
            let count = model.selected_threads(input.threads).len();
            if count == 0 {
                status(
                    &mut result,
                    "cannot select a thread: this hunk has no threads",
                );
            } else {
                let delta = if model.view.focus == FocusArea::Threads {
                    direction
                } else {
                    0
                };
                let selected =
                    wrapped_index(model.session.cursor().selected_thread(), count, delta);
                model.session.select_thread(selected);
                model.view.focus = FocusArea::Threads;
                model.reveal_selected_target(input.threads);
                let id = model
                    .current_thread_id(input.threads)
                    .expect("thread count is non-zero");
                status(
                    &mut result,
                    format!("thread target: #{id} ({}/{count})", selected + 1),
                );
            }
        }
        Event::CloseThread => {
            thread_effect(
                model,
                input.threads,
                &mut result,
                "could not close thread",
                |id| ThreadOperation::Close { id },
            );
        }
        Event::ReopenThread => {
            thread_effect(
                model,
                input.threads,
                &mut result,
                "could not reopen thread",
                |id| ThreadOperation::Reopen { id },
            );
        }
        Event::ToggleAttention => {
            let Some(id) = current_thread(model, input.threads, &mut result) else {
                return result;
            };
            let value = !input
                .threads
                .thread(id)
                .is_some_and(|thread| thread.needs_attention);
            result
                .effects
                .push(Effect::ChangeThreads(ThreadOperation::SetAttention {
                    id,
                    value,
                }));
        }
        Event::ToggleOutdated => {
            let Some(id) = current_thread(model, input.threads, &mut result) else {
                return result;
            };
            let value = !input
                .threads
                .thread(id)
                .is_some_and(|thread| thread.outdated);
            result
                .effects
                .push(Effect::ChangeThreads(ThreadOperation::SetOutdated {
                    id,
                    value,
                }));
        }
        Event::ToggleThreadExpansion => {
            let Some(id) = current_thread(model, input.threads, &mut result) else {
                return result;
            };
            if !input
                .threads
                .thread(id)
                .is_some_and(|thread| matches!(thread.resolution, Resolution::Resolved))
            {
                status(&mut result, "only resolved threads can be kept expanded");
                return result;
            }
            let was_expanded = model.expanded_threads.contains(&id);
            model.change_geometry(input.threads, |model| {
                if was_expanded {
                    model.expanded_threads.remove(&id);
                } else {
                    model.expanded_threads.insert(id);
                }
            });
            status(
                &mut result,
                if was_expanded {
                    format!("thread #{id} will collapse when inactive")
                } else {
                    format!("thread #{id} will stay expanded")
                },
            );
        }
        Event::MoveAttention(direction) => {
            let attention = attention_ids_in_git_order(model, input.threads);
            if attention.is_empty() {
                status(&mut result, "no needs-attention threads");
            } else {
                let current = model.current_thread_id(input.threads);
                let target = current
                    .and_then(|id| attention.iter().position(|candidate| *candidate == id))
                    .map_or_else(
                        || {
                            if direction < 0 {
                                attention.len() - 1
                            } else {
                                0
                            }
                        },
                        |index| wrapped_index(index, attention.len(), direction),
                    );
                let wrapped = current
                    .and_then(|id| attention.iter().position(|candidate| *candidate == id))
                    .is_some_and(|index| {
                        direction > 0 && target < index || direction < 0 && target > index
                    });
                let id = attention[target];
                status(
                    &mut result,
                    if wrapped {
                        format!(
                            "attention target: #{id} ({}/{}, wrapped)",
                            target + 1,
                            attention.len()
                        )
                    } else {
                        format!(
                            "attention target: #{id} ({}/{})",
                            target + 1,
                            attention.len()
                        )
                    },
                );
                result.effects.push(Effect::ResolveThread { id });
            }
        }
        Event::ShowRollup => {
            status(&mut result, "opened thread rollup");
            result.intents.push(Intent::OpenRollup);
        }
        Event::SetLayout(layout) => {
            model.change_geometry(input.threads, |model| model.view.layout = layout);
            status(&mut result, format!("layout: {}", layout_label(layout)));
        }
        Event::ToggleSidebar => {
            model.change_geometry(input.threads, |model| {
                model.sidebar_visible = !model.sidebar_visible;
                model.release_file_rail_focus();
            });
            let message = if !model.sidebar_visible {
                "file rail hidden"
            } else if model.viewport_columns < 72 {
                "file rail enabled; hidden below 72 columns"
            } else {
                "file rail shown"
            };
            status(&mut result, message);
        }
        Event::ReloadDiff => result.effects.push(Effect::ReloadDiff {
            request: model.request.clone(),
            purpose: ReloadPurpose::Manual,
        }),
        Event::ToggleHunkHeaders => {
            model.change_geometry(input.threads, |model| {
                model.show_hunk_headers = !model.show_hunk_headers;
            });
            status(
                &mut result,
                if model.show_hunk_headers {
                    "hunk headers shown"
                } else {
                    "hunk headers hidden"
                },
            );
        }
        Event::ToggleWrap => {
            model.change_geometry(input.threads, |model| {
                model.wrap_lines = !model.wrap_lines;
            });
            status(
                &mut result,
                if model.wrap_lines {
                    "line wrapping enabled"
                } else {
                    "line wrapping disabled"
                },
            );
        }
        Event::EffectCompleted(outcome) => {
            apply_outcome(model, input.threads, outcome, &mut result)
        }
    }
    result
}

/// `Tab` switches between the two regions that own a lasting position: the diff
/// and the file rail.  A selected inline thread is a target reached with `t`,
/// not a stop on this route, so leaving it means returning to the diff.
fn move_focus(model: &mut Model, threads: &Threads, result: &mut Update) {
    let next = match model.view.focus {
        FocusArea::Threads | FocusArea::Files => FocusArea::Review,
        FocusArea::Review if model.file_rail_visible() => FocusArea::Files,
        FocusArea::Review => {
            status(
                result,
                "cannot focus the file rail: it is hidden; press s to show it",
            );
            return;
        }
    };
    model.change_geometry(threads, |model| {
        model.view.focus = next;
        if next == FocusArea::Files {
            model.file_rail_target = model
                .session
                .diff()
                .document
                .files
                .get(model.session.cursor().selected_file())
                .map(|file| FileRailTarget::File(file.path.clone()));
        }
    });
    status(
        result,
        if next == FocusArea::Files {
            "file rail focused"
        } else {
            "diff focused"
        },
    );
}

fn move_file_rail(model: &mut Model, direction: i32, threads: &Threads, result: &mut Update) {
    let rows = model.file_rail_rows(threads);
    let Some((current, _)) = model.resolved_file_rail_target(&rows) else {
        status(result, "cannot move files: this diff has no changed files");
        return;
    };
    let target = if direction < 0 {
        current.saturating_sub(direction.unsigned_abs() as usize)
    } else {
        current
            .saturating_add(direction as usize)
            .min(rows.len().saturating_sub(1))
    };
    let row = &rows[target];
    select_file_rail_node(model, row, threads);
    status(
        result,
        match row.file_index {
            Some(_) => format!("file {}/{}: {}", target + 1, rows.len(), row.path),
            None => format!("directory {}/{}: {}", target + 1, rows.len(), row.path),
        },
    );
}

fn select_file_rail_node(model: &mut Model, row: &FileRailNode, threads: &Threads) {
    model.file_rail_target = Some(row.target());
    let Some(file) = row.file_index else {
        return;
    };
    if let Some((_, hunk)) = model
        .visible_hunks(threads)
        .into_iter()
        .find(|(candidate, _)| *candidate == file)
    {
        model.session.select_hunk(file, hunk);
    } else {
        model.session.select_file(file);
    }
    model.align_selected_file_to_top(threads);
}

fn all_file_directories(model: &Model, threads: &Threads) -> BTreeSet<String> {
    let directories = model
        .visible_files(threads)
        .into_iter()
        .filter_map(|index| model.session.diff().document.files.get(index))
        .flat_map(|file| {
            let components = file.path.split('/').collect::<Vec<_>>();
            (1..components.len()).map(move |end| components[..end].join("/"))
        })
        .collect::<BTreeSet<_>>();
    let top_level_items = model
        .visible_files(threads)
        .into_iter()
        .filter_map(|index| model.session.diff().document.files.get(index))
        .filter_map(|file| file.path.split('/').next())
        .collect::<BTreeSet<_>>();
    if top_level_items.len() > 1 {
        directories.into_iter().chain(["/".into()]).collect()
    } else {
        directories
    }
}

fn scroll_status(view: &ViewState) -> String {
    match view.scroll_from_end {
        Some(0) => "diff end".into(),
        Some(1) => "diff 1 row before end".into(),
        Some(offset) => format!("diff {offset} rows before end"),
        None => format!("diff row {}", view.scroll + 1),
    }
}

fn layout_label(layout: LayoutMode) -> &'static str {
    match layout {
        LayoutMode::Auto => "responsive",
        LayoutMode::Split => "split",
        LayoutMode::Stack => "stack",
    }
}

fn apply_outcome(model: &mut Model, threads: &Threads, outcome: Outcome, result: &mut Update) {
    match outcome {
        Outcome::DiffReloaded {
            purpose,
            result: outcome,
        } => match outcome {
            Ok(diff) => {
                let search_was_active = model.search.take().is_some();
                let preserve_end = model.view.scroll_from_end.is_some();
                let viewport_anchor = (!preserve_end).then(|| model.viewport_anchor(threads));
                let selected_was_visible = model.selected_target_is_visible(threads);
                let previous_thread_target = (model.focus() == FocusArea::Threads)
                    .then(|| {
                        Some((
                            model.selected_location()?,
                            model.current_thread_id(threads)?,
                        ))
                    })
                    .flatten();
                model.session.replace_diff(diff);
                model.invalidate_projection();
                let semantic_selection = model.semantic_selection(threads);
                model.reconcile_projection(threads, semantic_selection);
                let restored_thread_target =
                    previous_thread_target.is_some_and(|(location, id)| {
                        let resolved_location = model.resolve_current_location(&location);
                        if model.selected_location() != resolved_location {
                            return false;
                        }
                        let Some(index) = model
                            .selected_threads(threads)
                            .iter()
                            .position(|thread| thread.id == id)
                        else {
                            return false;
                        };
                        model.session.select_thread(index);
                        true
                    });
                if model.focus() == FocusArea::Threads && !restored_thread_target {
                    model.view.focus = FocusArea::Review;
                }
                if let Some(viewport_anchor) = viewport_anchor {
                    model.restore_viewport(viewport_anchor, threads);
                    if selected_was_visible {
                        model.reveal_selected_target(threads);
                    }
                }
                let message = match purpose {
                    ReloadPurpose::ContextChanged => {
                        format!("context: {} lines", model.request.context_lines)
                    }
                    ReloadPurpose::Manual => "reloaded current diff".into(),
                };
                status(
                    result,
                    if search_was_active {
                        format!("{message}; cleared search because the diff changed")
                    } else {
                        message
                    },
                );
            }
            Err(error) => status(result, format!("could not reload diff: {error}")),
        },
        Outcome::ThreadsChanged { result: outcome } => match outcome {
            Ok(change) => {
                status(result, success_status(change.success));
                result.intents.push(Intent::ReplaceThreads(change.state));
            }
            Err(error) => status(result, error),
        },
        Outcome::ThreadResolved {
            id,
            result: outcome,
        } => match outcome {
            Ok(location) => match model.select_thread_location(id, &location, threads) {
                Ok(true) => status(
                    result,
                    format!("thread #{id}; filter reset to All changes to reveal target"),
                ),
                Ok(false) => status(result, format!("thread #{id}")),
                Err(error) => status(result, error),
            },
            Err(error) => status(
                result,
                format!("thread #{id} anchor cannot resolve: {error}"),
            ),
        },
    }
}

fn current_thread(model: &Model, threads: &Threads, result: &mut Update) -> Option<ThreadId> {
    if model.focus() != FocusArea::Threads {
        status(
            result,
            "could not update thread: select a thread with t first",
        );
        return None;
    }
    let id = model.current_thread_id(threads);
    if id.is_none() {
        status(result, "could not update thread: no thread on this hunk");
    }
    id
}

fn thread_effect(
    model: &Model,
    threads: &Threads,
    result: &mut Update,
    error_prefix: &str,
    operation: impl FnOnce(ThreadId) -> ThreadOperation,
) {
    if model.focus() != FocusArea::Threads {
        status(
            result,
            format!("{error_prefix}: select a thread with t first"),
        );
        return;
    }
    let Some(id) = model.current_thread_id(threads) else {
        status(result, format!("{error_prefix}: no thread on this hunk"));
        return;
    };
    result.effects.push(Effect::ChangeThreads(operation(id)));
}

fn status(result: &mut Update, message: impl Into<String>) {
    result.intents.push(Intent::SetStatus(message.into()));
}

fn success_status(success: ThreadSuccess) -> String {
    match success {
        ThreadSuccess::Posted(id) => format!("posted comment #{id}"),
        ThreadSuccess::Replied => "posted reply".into(),
        ThreadSuccess::Closed => "thread closed".into(),
        ThreadSuccess::Reopened => "thread reopened".into(),
        ThreadSuccess::AttentionToggled => "needs-attention toggled".into(),
        ThreadSuccess::OutdatedToggled => "outdated toggled".into(),
    }
}

fn refresh_search(model: &mut Model, threads: &Threads, result: &mut Update) {
    let Some(mut search) = model.search.take() else {
        return;
    };
    let previous = search
        .selected
        .and_then(|index| search.matches.get(index))
        .cloned();
    let query = search.query.to_lowercase();
    search.matches = if query.is_empty() {
        Vec::new()
    } else {
        search
            .entries
            .iter()
            .filter(|entry| entry.normalized.contains(&query))
            .map(|entry| entry.target.clone())
            .collect()
    };
    search.selected = if search.matches.is_empty() {
        None
    } else {
        previous
            .as_ref()
            .and_then(|target| {
                search
                    .matches
                    .iter()
                    .position(|candidate| candidate == target)
            })
            .or(Some(0))
    };
    let target = search
        .selected
        .and_then(|index| search.matches.get(index))
        .cloned();
    let mut message = search_position_status(&search, "search");
    let origin = search.origin.clone();
    model.search = Some(search);
    if let Some(target) = target {
        let filter_was_reset = model.reveal_search_target(&target, threads);
        if filter_was_reset
            || origin.filter != ReviewFilter::AllChanges && model.filter == ReviewFilter::AllChanges
        {
            message.push_str("; filter reset to All changes for full-diff search");
        }
    } else {
        restore_search_origin(model, origin, threads);
    }
    status(result, message);
}

fn move_search(model: &mut Model, direction: i32, threads: &Threads, result: &mut Update) {
    let Some(mut search) = model.search.take() else {
        status(result, "no active search; press / to search");
        return;
    };
    if search.editing {
        status(result, "press Enter to finish the search before using n/N");
        model.search = Some(search);
        return;
    }
    if search.matches.is_empty() {
        status(
            result,
            format!(
                "no matches for “{}”; press / for a new search",
                search.query
            ),
        );
        model.search = Some(search);
        return;
    }
    let current = search.selected.unwrap_or(0);
    let next = wrapped_index(current, search.matches.len(), direction);
    let wrapped = direction > 0 && next < current || direction < 0 && next > current;
    search.selected = Some(next);
    let target = search.matches[next].clone();
    let mut message = if wrapped {
        format!(
            "search “{}”: {}/{} (wrapped)",
            search.query,
            next + 1,
            search.matches.len()
        )
    } else {
        search_position_status(&search, "search")
    };
    let search_origin_filter = search.origin.filter;
    model.search = Some(search);
    let filter_was_reset = model.reveal_search_target(&target, threads);
    if filter_was_reset
        || search_origin_filter != ReviewFilter::AllChanges
            && model.filter == ReviewFilter::AllChanges
    {
        message.push_str("; filter reset to All changes for full-diff search");
    }
    status(result, message);
}

fn search_position_status(search: &SearchState, prefix: &str) -> String {
    if search.query.is_empty() {
        return "search query is empty".into();
    }
    match search.selected {
        Some(index) => format!(
            "{prefix} “{}”: {}/{}",
            search.query,
            index + 1,
            search.matches.len()
        ),
        None => format!("no matches for “{}”", search.query),
    }
}

fn diff_search_entries(diff: &LoadedDiff) -> Vec<SearchEntry> {
    let mut entries = Vec::new();
    for file in &diff.document.files {
        entries.push(SearchEntry {
            normalized: file.path.to_lowercase(),
            target: DiffSearchTarget::FilePath {
                path: file.path.clone(),
            },
        });
        for hunk in &file.hunks {
            let location = HunkLocation::new(&file.path, &hunk.header);
            entries.push(SearchEntry {
                normalized: hunk.header.to_lowercase(),
                target: DiffSearchTarget::HunkHeader {
                    location: location.clone(),
                },
            });
            for (line_index, line) in hunk.lines.iter().enumerate() {
                entries.push(SearchEntry {
                    normalized: line.text.to_lowercase(),
                    target: DiffSearchTarget::DiffLine {
                        location: location.clone(),
                        line_index,
                    },
                });
            }
        }
    }
    entries
}

fn restore_search_origin(model: &mut Model, origin: SearchOrigin, threads: &Threads) {
    model.filter = origin.filter;
    if let Some(file) = model
        .session
        .diff()
        .document
        .files
        .get(origin.cursor.selected_file())
    {
        if origin.cursor.selected_hunk() < file.hunks.len() {
            model
                .session
                .select_hunk(origin.cursor.selected_file(), origin.cursor.selected_hunk());
        } else {
            model.session.select_file(origin.cursor.selected_file());
        }
        model.session.select_thread(origin.cursor.selected_thread());
    }
    model.view.focus = origin.focus;
    if model.search_geometry() == origin.geometry {
        model.view.scroll = origin.scroll;
        model.view.scroll_from_end = origin.scroll_from_end;
        let rows = model.row_map(threads);
        let visible_rows = model.visible_rows_for(&rows);
        model.view.clamp(rows.total_rows(), visible_rows);
    } else {
        model.restore_viewport(origin.viewport_anchor, threads);
    }
}

fn attention_ids_in_git_order(model: &Model, threads: &Threads) -> Vec<ThreadId> {
    model
        .session
        .diff()
        .document
        .files
        .iter()
        .flat_map(|file| {
            file.hunks.iter().flat_map(move |hunk| {
                let location = HunkLocation::new(&file.path, &hunk.header);
                model
                    .threads_at_current(&location, threads)
                    .into_iter()
                    .filter(|thread| thread.needs_attention)
                    .map(|thread| thread.id)
                    .collect::<Vec<_>>()
            })
        })
        .collect()
}

fn hunk_change_overlaps(anchor: HunkCoordinates, hunk: &DiffHunk) -> bool {
    let Some(coordinates) = hunk.coordinates else {
        return false;
    };
    let mut old_line = coordinates.old.start;
    let mut new_line = coordinates.new.start;
    let mut old_change = None;
    let mut new_change = None;
    for line in hunk.lines.iter() {
        match line.kind {
            DiffLineKind::Removed => {
                extend_range(&mut old_change, old_line);
                old_line = old_line.saturating_add(1);
            }
            DiffLineKind::Added => {
                extend_range(&mut new_change, new_line);
                new_line = new_line.saturating_add(1);
            }
            DiffLineKind::Context => {
                old_line = old_line.saturating_add(1);
                new_line = new_line.saturating_add(1);
            }
            DiffLineKind::Meta => {}
        }
    }
    old_change.is_some_and(|range| ranges_overlap(anchor.old, range))
        || new_change.is_some_and(|range| ranges_overlap(anchor.new, range))
}

fn extend_range(range: &mut Option<HunkRange>, line: usize) {
    match range {
        Some(range) => {
            let end = range.start.saturating_add(range.count);
            range.count = end.max(line.saturating_add(1)).saturating_sub(range.start);
        }
        None => {
            *range = Some(HunkRange {
                start: line,
                count: 1,
            });
        }
    }
}

fn ranges_overlap(left: HunkRange, right: HunkRange) -> bool {
    let left_end = left
        .start
        .saturating_add(left.count.max(1).saturating_sub(1));
    let right_end = right
        .start
        .saturating_add(right.count.max(1).saturating_sub(1));
    left.start <= right_end && right.start <= left_end
}

pub struct ViewInput<'a> {
    pub threads: &'a Threads,
}

pub struct View {
    pub body: Body,
    pub file_rail: Option<FileRail>,
    pub layout: LayoutPolicy,
}

pub fn view(model: &Model, input: ViewInput<'_>) -> View {
    let file_rail = model.sidebar_visible.then(|| {
        let rows = model.file_rail_rows(input.threads);
        let selected = model
            .resolved_file_rail_target(&rows)
            .map(|(position, _)| position);
        FileRail {
            selected,
            tree: model.file_tree,
            rows: rows
                .into_iter()
                .filter_map(|row| {
                    let kind = if let Some(file_index) = row.file_index {
                        let file = model.session.diff().document.files.get(file_index)?;
                        let file_threads = file
                            .hunks
                            .iter()
                            .flat_map(|hunk| {
                                model.visible_threads_at(
                                    &HunkLocation::new(&file.path, &hunk.header),
                                    input.threads,
                                )
                            })
                            .collect::<Vec<_>>();
                        let attention = if file_threads.iter().any(|thread| thread.needs_attention)
                        {
                            FileAttention::NeedsAttention
                        } else if file_threads
                            .iter()
                            .any(|thread| matches!(thread.resolution, Resolution::Open))
                        {
                            FileAttention::Open
                        } else if file_threads.is_empty() {
                            FileAttention::None
                        } else {
                            FileAttention::Resolved
                        };
                        FileRailRowKind::File {
                            change: file.change.clone(),
                            attention,
                        }
                    } else {
                        FileRailRowKind::Directory {
                            collapsed: model.collapsed_directories.contains(&row.path),
                        }
                    };
                    Some(FileRailRow {
                        label: row.label,
                        path: row.path,
                        depth: row.depth,
                        kind,
                    })
                })
                .collect(),
        }
    });
    let mut review = review_body(model, input.threads);
    let presentation_width = review_body_width(model.viewport_columns, model.sidebar_visible);
    let rows = model.row_map(input.threads);
    let visible_rows = model.visible_rows_for(&rows);
    let scroll = model.view.resolved_scroll(rows.total_rows(), visible_rows);
    review.scroll = scroll;
    review.viewport = ReviewViewport {
        presentation_width,
        total_rows: rows.total_rows(),
        visible_rows,
        sticky_context: rows.sticky_context(scroll),
        sections: std::sync::Arc::new(
            rows.window_sections(scroll, scroll.saturating_add(visible_rows)),
        ),
    };
    let body = Body::Review(Box::new(review));
    View {
        body,
        file_rail,
        layout: LayoutPolicy {
            focus: model.focus(),
            diff_layout: model.view.layout,
            wrap_lines: model.wrap_lines,
        },
    }
}

fn review_body(model: &Model, threads: &Threads) -> ReviewBody {
    let source_is_empty = model.session.diff().document.files.is_empty();
    let visible_hunks = model.visible_hunks(threads);
    let filtered_is_empty = !source_is_empty && visible_hunks.is_empty();
    ReviewBody {
        scroll: 0,
        viewport: ReviewViewport {
            presentation_width: 0,
            total_rows: 0,
            visible_rows: 0,
            sticky_context: None,
            sections: std::sync::Arc::new(Vec::new()),
        },
        empty_state: source_is_empty
            .then(|| empty_diff_message(&model.request.source))
            .or_else(|| filtered_is_empty.then(|| empty_filter_message(model.filter))),
        search_target: model.current_search_target().cloned(),
        search_query: model.search.as_ref().map(|search| search.query.clone()),
        files: model
            .session
            .diff()
            .document
            .files
            .iter()
            .enumerate()
            .filter_map(|(file_index, file)| {
                let hunks = file
                    .hunks
                    .iter()
                    .enumerate()
                    .filter_map(|(hunk_index, hunk)| {
                        let selected = file_index == model.session.cursor().selected_file()
                            && hunk_index == model.session.cursor().selected_hunk();
                        let location = HunkLocation::new(&file.path, &hunk.header);
                        model
                            .hunk_matches_filter(&location, threads)
                            .then(|| ReviewHunk {
                                anchor: location.clone(),
                                header: model.show_hunk_headers.then(|| hunk.header.clone()),
                                coordinates: hunk.coordinates,
                                selected,
                                lines: hunk.lines.clone(),
                                threads: model
                                    .visible_threads_at(&location, threads)
                                    .iter()
                                    .enumerate()
                                    .map(|(thread_index, thread)| ThreadCard {
                                        id: thread.id,
                                        state: thread_state(thread),
                                        resolved: matches!(thread.resolution, Resolution::Resolved),
                                        outdated: thread.outdated,
                                        active: selected
                                            && thread_index
                                                == model.session.cursor().selected_thread()
                                            && model.focus() == FocusArea::Threads,
                                        expanded: model.expanded_threads.contains(&thread.id),
                                        message_count: thread.messages.len(),
                                        closed_by: thread
                                            .closed_by
                                            .as_ref()
                                            .map(|participant| participant.id.clone()),
                                        latest: thread
                                            .messages
                                            .last()
                                            .map(|message| {
                                                format!("{}: {}", message.author.id, message.body)
                                            })
                                            .unwrap_or_default(),
                                    })
                                    .collect(),
                            })
                    })
                    .collect::<Vec<_>>();
                (!hunks.is_empty() || model.filter == ReviewFilter::AllChanges).then(|| {
                    ReviewFile {
                        path: file.path.clone(),
                        selected: file_index == model.session.cursor().selected_file(),
                        extension: file.extension().map(str::to_owned),
                        magnitude: file.magnitude,
                        notes: crate::presentation::file_change_notes(file),
                        hunks,
                    }
                })
            })
            .collect(),
    }
}

fn empty_filter_message(filter: ReviewFilter) -> String {
    format!(
        "No review targets match Filter: {}.\nThe Git diff is still loaded; this is a filtered view, not an empty changeset.\nPress A for All changes or F to cycle filters.",
        filter.label()
    )
}

fn empty_diff_message(source: &crate::domain::diff::DiffSource) -> String {
    format!(
        "No changes found.\nSelected diff source: {}.\nUpdate the source or make a change, then press r to reload.",
        source.description()
    )
}

fn thread_state(thread: &ReviewThread) -> SemanticThreadState {
    if thread.needs_attention {
        SemanticThreadState::NeedsAttention
    } else if matches!(thread.resolution, Resolution::Open) {
        SemanticThreadState::Open
    } else {
        SemanticThreadState::Resolved
    }
}

fn wrapped_index(current: usize, length: usize, direction: i32) -> usize {
    ((current as i32 + direction).rem_euclid(length as i32)) as usize
}
