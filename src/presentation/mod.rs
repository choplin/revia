//! Pure diff-to-terminal presentation transforms.
//!
//! The renderer owns widget layout. This module owns only the conversion from
//! parsed Git hunks to width-bounded terminal rows. Source coordinates remain
//! attached to the parsed hunk; display rows are deliberately ephemeral.

pub(crate) mod renderer;
pub(crate) mod symbols;
pub(crate) mod syntax;
pub(crate) mod text;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use urushi::{Color, TextStyle};

use crate::{
    app::{
        view::{
            DiffSearchTarget, LayoutPolicy, ReviewBody, ReviewWindowSection, StickyReviewContext,
            Tone,
        },
        view_state::{LayoutMode, fit_width, truncate_end, truncate_start},
    },
    domain::{
        anchor::HunkLocation,
        diff::{DiffFile, DiffLine, DiffLineKind, FileStatus, HunkCoordinates, Magnitude},
    },
};

use self::{
    renderer::SemanticTheme,
    syntax::{HunkSyntax, SyntaxLine, TokenStyle},
    text::{Line, Span},
};

#[cfg(test)]
use crate::app::view::{ThreadCard, ThreadState};

const TAB_WIDTH: usize = 4;
const SPLIT_SEPARATOR: &str = " │ ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewRowMap {
    total_rows: usize,
    files: Vec<FileRows>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileRows {
    path: String,
    selected: bool,
    index: usize,
    magnitude: Magnitude,
    notes: Vec<String>,
    start: usize,
    end: usize,
    hunks: Vec<HunkRows>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HunkRows {
    anchor: HunkLocation,
    header: Option<String>,
    index: usize,
    start: usize,
    end: usize,
    selected: bool,
    active_thread_row: Option<usize>,
    line_rows: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewportAnchor {
    section: ViewportSection,
    offset: usize,
    extent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ViewportSection {
    Start,
    File(String),
    Hunk {
        location: HunkLocation,
        index: usize,
    },
}

impl ReviewRowMap {
    pub(crate) fn total_rows(&self) -> usize {
        self.total_rows
    }

    pub(crate) fn hunk_at(&self, row: usize) -> Option<&HunkLocation> {
        let file = self
            .files
            .iter()
            .find(|file| row >= file.start && row < file.end)
            .or_else(|| self.files.last())?;
        file.hunks
            .iter()
            .find(|hunk| row >= hunk.start && row < hunk.end)
            .or_else(|| file.hunks.iter().find(|hunk| hunk.start >= row))
            .or_else(|| file.hunks.last())
            .map(|hunk| &hunk.anchor)
    }

    pub(crate) fn window_sections(&self, start: usize, end: usize) -> Vec<ReviewWindowSection> {
        let overlaps =
            |section_start: usize, section_end: usize| section_start < end && section_end > start;
        let mut sections = Vec::new();
        for file in &self.files {
            let header_end = file.hunks.first().map_or(file.end, |hunk| hunk.start);
            if overlaps(file.start, header_end) {
                sections.push(ReviewWindowSection::FileHeader {
                    file_index: file.index,
                    start: file.start,
                    end: header_end,
                });
            }
            sections.extend(file.hunks.iter().filter_map(|hunk| {
                overlaps(hunk.start, hunk.end).then_some(ReviewWindowSection::Hunk {
                    file_index: file.index,
                    hunk_index: hunk.index,
                    start: hunk.start,
                    end: hunk.end,
                })
            }));
        }
        sections
    }

    /// Physical rows the sticky position block occupies.
    ///
    /// The count is per-diff rather than per-scroll-position so the code
    /// viewport does not resize as the reviewer moves between files.
    pub(crate) fn sticky_rows(&self) -> usize {
        if self.files.is_empty() {
            0
        } else {
            STICKY_ROWS
        }
    }

    pub(crate) fn anchor_at(&self, row: usize) -> ViewportAnchor {
        for file in &self.files {
            if row < file.start || row >= file.end {
                continue;
            }
            if let Some(hunk) = file
                .hunks
                .iter()
                .find(|hunk| row >= hunk.start && row < hunk.end)
            {
                return ViewportAnchor {
                    section: ViewportSection::Hunk {
                        location: hunk.anchor.clone(),
                        index: hunk.index,
                    },
                    offset: row.saturating_sub(hunk.start),
                    extent: hunk.end.saturating_sub(hunk.start).max(1),
                };
            }
            return ViewportAnchor {
                section: ViewportSection::File(file.path.clone()),
                offset: row.saturating_sub(file.start),
                extent: file.end.saturating_sub(file.start).max(1),
            };
        }
        ViewportAnchor {
            section: ViewportSection::Start,
            offset: 0,
            extent: 1,
        }
    }

    pub(crate) fn row_for_anchor(&self, anchor: &ViewportAnchor) -> usize {
        let (start, extent) = match &anchor.section {
            ViewportSection::Start => return 0,
            ViewportSection::File(path) => self
                .files
                .iter()
                .find(|file| &file.path == path)
                .map(|file| (file.start, file.end.saturating_sub(file.start))),
            ViewportSection::Hunk { location, .. } => self
                .files
                .iter()
                .flat_map(|file| &file.hunks)
                .find(|hunk| &hunk.anchor == location)
                .map(|hunk| (hunk.start, hunk.end.saturating_sub(hunk.start))),
        }
        .or_else(|| self.closest_section(anchor))
        .unwrap_or((0, self.total_rows));
        let relative = anchor.offset.saturating_mul(extent.max(1)) / anchor.extent.max(1);
        start.saturating_add(relative.min(extent.saturating_sub(1)))
    }

    fn closest_section(&self, anchor: &ViewportAnchor) -> Option<(usize, usize)> {
        let ViewportSection::Hunk { location, index } = &anchor.section else {
            return None;
        };
        let file = self
            .files
            .iter()
            .find(|file| file.path == location.path())?;
        let hunk = file
            .hunks
            .get((*index).min(file.hunks.len().saturating_sub(1)))?;
        Some((hunk.start, hunk.end.saturating_sub(hunk.start)))
    }

    pub(crate) fn selected_target_row(&self) -> Option<usize> {
        self.files
            .iter()
            .flat_map(|file| &file.hunks)
            .find(|hunk| hunk.selected)
            .map(|hunk| hunk.active_thread_row.unwrap_or(hunk.start))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|file| file.selected)
                    .map(|file| file.start.saturating_add(1))
            })
    }

    pub(crate) fn selected_file_start_row(&self) -> Option<usize> {
        self.files
            .iter()
            .find(|file| file.selected)
            .map(|file| file.start)
    }

    pub(crate) fn selected_hunk_range(&self) -> Option<std::ops::Range<usize>> {
        self.files
            .iter()
            .flat_map(|file| &file.hunks)
            .find(|hunk| hunk.selected)
            .map(|hunk| hunk.start..hunk.end)
    }

    pub(crate) fn row_for_search_target(&self, target: &DiffSearchTarget) -> Option<usize> {
        match target {
            DiffSearchTarget::FilePath { path } => self
                .files
                .iter()
                .find(|file| &file.path == path)
                .map(|file| file.start.saturating_add(1)),
            DiffSearchTarget::HunkHeader { location } => self
                .files
                .iter()
                .flat_map(|file| &file.hunks)
                .find(|hunk| &hunk.anchor == location)
                .map(|hunk| hunk.start),
            DiffSearchTarget::DiffLine {
                location,
                line_index,
            } => self
                .files
                .iter()
                .flat_map(|file| &file.hunks)
                .find(|hunk| &hunk.anchor == location)
                .and_then(|hunk| hunk.line_rows.get(*line_index).copied()),
        }
    }

    pub(crate) fn sticky_context(&self, row: usize) -> Option<StickyReviewContext> {
        let file = self
            .files
            .iter()
            .find(|file| row >= file.start && row < file.end)
            .or_else(|| self.files.last())?;
        let hunk = file
            .hunks
            .iter()
            .find(|hunk| row >= hunk.start && row < hunk.end)
            .or_else(|| file.hunks.iter().rev().find(|hunk| hunk.start <= row))
            .or_else(|| file.hunks.first());
        Some(StickyReviewContext {
            file: file.path.clone(),
            file_index: file.index,
            file_count: self.files.len(),
            magnitude: file.magnitude,
            notes: file.notes.clone(),
            hunk_header: hunk.and_then(|hunk| hunk.header.clone()),
            hunk_index: hunk.map(|hunk| hunk.index),
            hunk_count: file.hunks.len(),
        })
    }
}

pub(crate) fn review_row_map(
    review: &ReviewBody,
    available_width: u16,
    layout: LayoutPolicy,
) -> ReviewRowMap {
    let mut cursor = review
        .empty_state
        .as_deref()
        .map_or(0, |message| message.lines().count());
    let mut files = Vec::with_capacity(review.files.len());
    for (file_index, file) in review.files.iter().enumerate() {
        let file_start = cursor;
        cursor = cursor
            .saturating_add(1 + file_boundary_for(file, &review.files, available_width).rows());
        let number_width = line_number_width(
            file.hunks
                .iter()
                .map(|hunk| (hunk.lines.as_slice(), hunk.coordinates)),
        );
        let mut hunks = Vec::with_capacity(file.hunks.len());
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let start = cursor;
            let hunk_content_width = available_width.saturating_sub(2);
            cursor = cursor.saturating_add(1);
            let (diff_rows, line_rows) = hunk_line_rows(
                &hunk.lines,
                hunk.coordinates,
                cursor,
                hunk_content_width,
                number_width,
                layout,
            );
            cursor = cursor.saturating_add(diff_rows);
            let active_thread_row = hunk
                .threads
                .iter()
                .any(|thread| thread.active)
                .then_some(cursor);
            cursor = cursor.saturating_add(usize::from(!hunk.threads.is_empty()));
            cursor = cursor.saturating_add(1);
            hunks.push(HunkRows {
                anchor: hunk.anchor.clone(),
                header: hunk.header.clone(),
                index: hunk_index,
                start,
                end: cursor.max(start + 1),
                selected: hunk.selected,
                active_thread_row,
                line_rows,
            });
        }
        files.push(FileRows {
            path: file.path.clone(),
            selected: file.selected,
            index: file_index,
            magnitude: file.magnitude,
            notes: file.notes.clone(),
            start: file_start,
            end: cursor.max(file_start + 1),
            hunks,
        });
    }
    ReviewRowMap {
        total_rows: cursor,
        files,
    }
}

/// The file boundary as the viewport row map sees it.
///
/// The renderer swaps in a search marker of the same cell width, so both agree
/// on how many physical rows the boundary occupies.
pub(crate) fn file_boundary_for(
    file: &crate::app::view::ReviewFile,
    files: &[crate::app::view::ReviewFile],
    available_width: u16,
) -> FileBoundary {
    file_boundary(
        symbols::FILE,
        &file.path,
        unique_prefix_segments(&file.path, files.iter().map(|file| file.path.as_str())),
        &file.notes,
        file.magnitude,
        available_width,
    )
}

/// Reduces one file's patch headers to the review facts that change how the
/// change is interpreted. Everything else is Git transport and is dropped.
pub(crate) fn file_change_notes(file: &DiffFile) -> Vec<String> {
    let mut notes = Vec::new();
    match file.change.status {
        FileStatus::Added => notes.push("new file".to_owned()),
        FileStatus::Deleted => notes.push("deleted file".to_owned()),
        FileStatus::Modified => {}
    }
    if let Some(moved) = file.change.moved {
        let verb = if moved.copied { "copy" } else { "rename" };
        let similarity = moved
            .similarity
            .map(|value| format!(" {value}%"))
            .unwrap_or_default();
        let previous = file.previous_path.as_deref().unwrap_or("(unknown)");
        notes.push(format!("{verb}{similarity} · {previous} → {}", file.path));
    }
    if let Some(mode) = &file.change.mode {
        notes.push(format!("mode {} → {}", mode.old, mode.new));
    }
    if file.change.binary {
        notes.push("binary".to_owned());
    }
    notes
}

pub(crate) fn format_magnitude(magnitude: Magnitude) -> String {
    format!("+{} -{}", magnitude.additions, magnitude.deletions)
}

/// The physical rows of one file boundary.
///
/// Row count is width-dependent, so viewport row mapping and rendering must
/// derive it from this single transform rather than agreeing by convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileBoundary {
    pub(crate) primary: String,
    pub(crate) detail: Option<String>,
}

impl FileBoundary {
    pub(crate) fn rows(&self) -> usize {
        1 + usize::from(self.detail.is_some())
    }
}

/// Lays out a file boundary as `<marker> <fitted path> … <+adds -dels>`.
///
/// The magnitude is never dropped: the path label is shortened first, and
/// semantic notes move to their own row before they can displace either.
pub(crate) fn file_boundary(
    marker: &str,
    path: &str,
    required_prefix: usize,
    notes: &[String],
    magnitude: Magnitude,
    available_width: u16,
) -> FileBoundary {
    let width = usize::from(available_width);
    let prefix = if marker.is_empty() {
        String::new()
    } else {
        format!("{marker} ")
    };
    let stat = format_magnitude(magnitude);
    let reserved = UnicodeWidthStr::width(prefix.as_str())
        .saturating_add(UnicodeWidthStr::width(stat.as_str()))
        .saturating_add(GUTTER_GAP);
    let label_width = width.saturating_sub(reserved);
    let notes_text = notes.join(" · ");
    let path_label = fit_path_label(path, required_prefix, label_width);
    let inline = (!notes_text.is_empty())
        .then(|| format!("{path_label}  {notes_text}"))
        .filter(|value| UnicodeWidthStr::width(value.as_str()) <= label_width);
    let detail = (!notes_text.is_empty() && inline.is_none())
        .then(|| truncate_end(&format!("  {notes_text}"), width));
    FileBoundary {
        primary: right_aligned_row(
            &prefix,
            inline.as_deref().unwrap_or(&path_label),
            &stat,
            width,
        ),
        detail,
    }
}

const GUTTER_GAP: usize = 2;

/// The sticky position block: one row for the file, one for the hunk.
///
/// Splitting them keeps a long hunk header readable instead of competing with
/// the path for the same row.
pub(crate) const STICKY_ROWS: usize = 2;

/// Places `value` after `prefix` and pins `right` to the last cell of `width`.
pub(crate) fn right_aligned_row(prefix: &str, value: &str, right: &str, width: usize) -> String {
    let prefix_width = UnicodeWidthStr::width(prefix);
    let right_width = UnicodeWidthStr::width(right);
    let value_width = width
        .saturating_sub(prefix_width)
        .saturating_sub(right_width)
        .saturating_sub(GUTTER_GAP);
    if value_width == 0 {
        return truncate_end(&format!("{prefix}{right}"), width);
    }
    let fitted = fit_width(value, value_width);
    format!("{prefix}{fitted}{}{right}", " ".repeat(GUTTER_GAP))
}

/// How many leading segments `path` must keep to stay distinguishable from the
/// other files in the same changeset.
///
/// Only files sharing a filename can collide, so the answer is zero for almost
/// every path and the scan stops as soon as the prefix separates them.
pub(crate) fn unique_prefix_segments<'a>(
    path: &str,
    siblings: impl Iterator<Item = &'a str>,
) -> usize {
    let name = file_name(path);
    let rivals = siblings
        .filter(|other| *other != path && file_name(other) == name)
        .collect::<Vec<_>>();
    if rivals.is_empty() {
        return 0;
    }
    let segments = path.split('/').count();
    for keep in 1..segments {
        let prefix = leading_segments(path, keep);
        if rivals
            .iter()
            .all(|other| leading_segments(other, keep) != prefix)
        {
            return keep;
        }
    }
    segments.saturating_sub(1)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn leading_segments(path: &str, count: usize) -> String {
    path.split('/').take(count).collect::<Vec<_>>().join("/")
}

/// How an over-long path label is shortened.
///
/// Shortening is a presentation judgement that is expected to change, so the
/// strategy is named and selected in one place rather than being inlined at
/// each call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathFit {
    /// Keep the filename and the leading segments; collapse the middle to `…`.
    MiddleElision,
    /// Keep as much of the path's tail as fits.
    TailOnly,
}

/// The strategy every path label currently uses. Change this one binding to
/// switch shortening behaviour across the rail and the file boundaries.
pub(crate) const PATH_FIT: PathFit = PathFit::MiddleElision;

/// Fits a repository path into `width` using the configured [`PATH_FIT`]
/// strategy, keeping at least `required_prefix` leading segments so two files
/// with the same name stay distinguishable by where they live.
pub(crate) fn fit_path_label(path: &str, required_prefix: usize, width: usize) -> String {
    fit_path_label_with(PATH_FIT, path, required_prefix, width)
}

pub(crate) fn fit_path_label_with(
    strategy: PathFit,
    path: &str,
    required_prefix: usize,
    width: usize,
) -> String {
    if UnicodeWidthStr::width(path) <= width {
        return path.to_owned();
    }
    if strategy == PathFit::TailOnly {
        return truncate_start(path, width);
    }
    let segments = path.split('/').collect::<Vec<_>>();
    let head_count = required_prefix.max(1);
    if segments.len() > head_count.saturating_add(1) {
        let head = segments[..head_count].join("/");
        for keep in (1..segments.len().saturating_sub(head_count)).rev() {
            let tail = segments[segments.len().saturating_sub(keep)..].join("/");
            let candidate = format!("{head}/…/{tail}");
            if UnicodeWidthStr::width(candidate.as_str()) <= width {
                return candidate;
            }
        }
    }
    if segments.len() > 1 {
        let candidate = format!("…/{}", file_name(path));
        if UnicodeWidthStr::width(candidate.as_str()) <= width {
            return candidate;
        }
    }
    truncate_start(path, width)
}

/// Produces the complete, width-bounded physical rows for one inline thread card.
///
/// Keeping this transform shared by row mapping and rendering makes card height
/// changes safe for viewport anchoring and selected-target navigation.
#[cfg(test)]
pub(crate) fn thread_card_rows(
    thread: &ThreadCard,
    available_width: u16,
    is_last: bool,
) -> Vec<String> {
    let width = usize::from(available_width);
    let narrow = width < 72;
    let state = match (thread.state, thread.resolved, narrow) {
        (ThreadState::NeedsAttention, true, true) => " ATTENTION ·  RESOLVED",
        (ThreadState::NeedsAttention, false, true) => " ATTENTION ·  OPEN",
        (ThreadState::NeedsAttention, true, false) => " NEEDS ATTENTION ·  RESOLVED",
        (ThreadState::NeedsAttention, false, false) => " NEEDS ATTENTION ·  OPEN",
        (ThreadState::Open, _, _) => " OPEN",
        (ThreadState::Resolved, _, _) => " RESOLVED",
    };
    let active = if thread.active { "▶ ACTIVE · " } else { "" };
    let outdated = if thread.outdated {
        " · ~ OUTDATED"
    } else {
        ""
    };
    let connector = if is_last { "╰─" } else { "├─" };
    let provenance = thread
        .closed_by
        .as_ref()
        .map_or_else(String::new, |actor| format!(" · closed by {actor}"));
    let messages = match thread.message_count {
        1 => "1 message".to_owned(),
        count => format!("{count} messages"),
    };
    let expanded = thread.active
        || thread.expanded
        || matches!(
            thread.state,
            ThreadState::NeedsAttention | ThreadState::Open
        );

    if !expanded {
        let summary = format!(
            "  {connector} #{:03} {state}{outdated}{provenance} · {messages}",
            thread.id
        );
        if UnicodeWidthStr::width(summary.as_str()) <= width || thread.closed_by.is_none() {
            return vec![truncate_end(&summary, width)];
        }
        let mut rows = vec![truncate_end(
            &format!(
                "  {connector} #{:03} {state}{outdated} · {messages}",
                thread.id
            ),
            width,
        )];
        rows.extend(wrap_thread_message(
            &format!(
                "closed by {}",
                thread.closed_by.as_deref().unwrap_or_default()
            ),
            width,
            usize::MAX,
        ));
        return rows;
    }

    let mut rows = if narrow && (thread.active || thread.state == ThreadState::NeedsAttention) {
        let narrow_active = if thread.active { "▶ ACTIVE · " } else { "" };
        let mut rows = vec![truncate_end(
            &format!(
                "  {connector} {narrow_active}#{:03} · {messages}",
                thread.id
            ),
            width,
        )];
        rows.push(truncate_end(&format!("  │ {state}{outdated}"), width));
        if let Some(actor) = thread.closed_by.as_deref() {
            rows.extend(wrap_thread_message(
                &format!("closed by {actor}"),
                width,
                usize::MAX,
            ));
        }
        rows
    } else {
        vec![truncate_end(
            &format!(
                "  {connector} {active}#{:03} {state}{outdated}{provenance} · {messages}",
                thread.id
            ),
            width,
        )]
    };
    let max_message_rows = if thread.active || thread.state == ThreadState::NeedsAttention {
        3
    } else {
        2
    };
    rows.extend(wrap_thread_message(&thread.latest, width, max_message_rows));
    let actions = if thread.active && narrow {
        if thread.resolved {
            vec![
                "  │ c reply · R reopen · a attention",
                "  └─ o outdated · e keep/fold",
            ]
        } else {
            vec!["  │ c reply · x resolve · a attention", "  └─ o outdated"]
        }
    } else if thread.active {
        if thread.resolved {
            vec!["  └─ c reply · R reopen · a attention · o outdated · e keep/fold"]
        } else {
            vec!["  └─ c reply · x resolve · a attention · o outdated"]
        }
    } else if thread.state == ThreadState::NeedsAttention {
        vec!["  └─ t/T select · action required"]
    } else {
        vec!["  └─ t/T select for thread actions"]
    };
    rows.extend(
        actions
            .into_iter()
            .map(|action| truncate_end(action, width)),
    );
    rows
}

#[cfg(test)]
fn wrap_thread_message(value: &str, width: usize, max_rows: usize) -> Vec<String> {
    const PREFIX: &str = "  │ ";
    let prefix_width = UnicodeWidthStr::width(PREFIX);
    let content_width = width.saturating_sub(prefix_width);
    if content_width == 0 {
        return vec![truncate_end(PREFIX, width)];
    }

    let mut chunks = Vec::new();
    for source_line in value.lines() {
        let mut chunk = String::new();
        let mut chunk_width: usize = 0;
        for grapheme in source_line.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if !chunk.is_empty() && chunk_width.saturating_add(grapheme_width) > content_width {
                chunks.push(std::mem::take(&mut chunk));
                chunk_width = 0;
            }
            if grapheme_width <= content_width {
                chunk.push_str(grapheme);
                chunk_width = chunk_width.saturating_add(grapheme_width);
            }
        }
        chunks.push(chunk);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }

    let truncated = chunks.len() > max_rows;
    chunks.truncate(max_rows);
    if truncated && let Some(last) = chunks.last_mut() {
        *last = truncate_end(&format!("{last}…"), content_width);
    }
    chunks
        .into_iter()
        .map(|chunk| truncate_end(&format!("{PREFIX}{chunk}"), width))
        .collect()
}

fn hunk_line_rows(
    lines: &[DiffLine],
    _coordinates: Option<HunkCoordinates>,
    start: usize,
    available_width: u16,
    number_width: usize,
    layout: LayoutPolicy,
) -> (usize, Vec<usize>) {
    match layout.diff_layout.resolved(available_width) {
        LayoutMode::Split => {
            let mut line_rows = vec![start; lines.len()];
            let mut line_index = 0;
            let mut row_index = 0;
            while line_index < lines.len() {
                if matches!(
                    lines[line_index].kind,
                    DiffLineKind::Removed | DiffLineKind::Added
                ) {
                    let removed_start = line_index;
                    while line_index < lines.len()
                        && lines[line_index].kind == DiffLineKind::Removed
                    {
                        line_index += 1;
                    }
                    let added_start = line_index;
                    while line_index < lines.len() && lines[line_index].kind == DiffLineKind::Added
                    {
                        line_index += 1;
                    }
                    let removed_count = added_start.saturating_sub(removed_start);
                    let added_count = line_index.saturating_sub(added_start);
                    for offset in 0..removed_count {
                        line_rows[removed_start + offset] =
                            start.saturating_add(row_index + offset);
                    }
                    for offset in 0..added_count {
                        line_rows[added_start + offset] = start.saturating_add(row_index + offset);
                    }
                    row_index = row_index.saturating_add(removed_count.max(added_count));
                } else {
                    line_rows[line_index] = start.saturating_add(row_index);
                    line_index += 1;
                    row_index += 1;
                }
            }
            (row_index, line_rows)
        }
        LayoutMode::Stack | LayoutMode::Auto => {
            let content_width = usize::from(available_width)
                .saturating_sub(number_width.saturating_mul(2).saturating_add(8));
            let mut cursor = start;
            let line_rows = lines
                .iter()
                .map(|line| {
                    let row = cursor;
                    let height = if line.kind == DiffLineKind::Meta
                        || !layout.wrap_lines
                        || content_width == 0
                    {
                        1
                    } else {
                        wrapped_row_count(&line.text, content_width)
                    };
                    cursor = cursor.saturating_add(height);
                    row
                })
                .collect();
            (cursor.saturating_sub(start), line_rows)
        }
    }
}

fn wrapped_row_count(value: &str, width: usize) -> usize {
    if value.is_empty() || width == 0 {
        return 1;
    }
    let mut rows = 1usize;
    let mut row_width = 0usize;
    let mut source_column = 0usize;
    for grapheme in value.graphemes(true) {
        let cell_width = if grapheme == "\t" {
            TAB_WIDTH - source_column % TAB_WIDTH
        } else {
            UnicodeWidthStr::width(grapheme)
        };
        source_column = source_column.saturating_add(cell_width);
        let parts = if grapheme == "\t" { cell_width } else { 1 };
        let part_width = if grapheme == "\t" { 1 } else { cell_width };
        for _ in 0..parts {
            if row_width > 0 && row_width.saturating_add(part_width) > width {
                rows = rows.saturating_add(1);
                row_width = 0;
            }
            if part_width <= width {
                row_width = row_width.saturating_add(part_width);
            }
        }
    }
    rows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumberedLine {
    source_index: usize,
    pub(crate) line: DiffLine,
    pub(crate) old_number: Option<usize>,
    pub(crate) new_number: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SplitRow {
    pub(crate) old: Option<NumberedLine>,
    pub(crate) new: Option<NumberedLine>,
}

pub(crate) fn numbered_lines(
    lines: &[DiffLine],
    coordinates: Option<HunkCoordinates>,
) -> Vec<NumberedLine> {
    let (mut old_number, mut new_number) = coordinates
        .map(|coordinates| (Some(coordinates.old.start), Some(coordinates.new.start)))
        .unwrap_or((None, None));

    lines
        .iter()
        .enumerate()
        .map(|(source_index, line)| {
            let (displayed_old, displayed_new) = match line.kind {
                DiffLineKind::Added => (None, new_number),
                DiffLineKind::Removed => (old_number, None),
                DiffLineKind::Context => (old_number, new_number),
                DiffLineKind::Meta => (None, None),
            };
            if matches!(line.kind, DiffLineKind::Removed | DiffLineKind::Context) {
                old_number = old_number.map(|number| number.saturating_add(1));
            }
            if matches!(line.kind, DiffLineKind::Added | DiffLineKind::Context) {
                new_number = new_number.map(|number| number.saturating_add(1));
            }
            NumberedLine {
                source_index,
                line: line.clone(),
                old_number: displayed_old,
                new_number: displayed_new,
            }
        })
        .collect()
}

/// Pair adjacent removed and added runs for side-by-side presentation without
/// claiming that Git supplied an exact line-level correspondence.
pub(crate) fn split_rows(lines: &[NumberedLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        match lines[index].line.kind {
            DiffLineKind::Removed | DiffLineKind::Added => {
                let mut removed = Vec::new();
                while index < lines.len() && lines[index].line.kind == DiffLineKind::Removed {
                    removed.push(lines[index].clone());
                    index += 1;
                }
                let mut added = Vec::new();
                while index < lines.len() && lines[index].line.kind == DiffLineKind::Added {
                    added.push(lines[index].clone());
                    index += 1;
                }
                for row in 0..removed.len().max(added.len()) {
                    rows.push(SplitRow {
                        old: removed.get(row).cloned(),
                        new: added.get(row).cloned(),
                    });
                }
            }
            DiffLineKind::Context => {
                let line = lines[index].clone();
                rows.push(SplitRow {
                    old: Some(line.clone()),
                    new: Some(line),
                });
                index += 1;
            }
            DiffLineKind::Meta => {
                rows.push(SplitRow {
                    old: Some(lines[index].clone()),
                    new: None,
                });
                index += 1;
            }
        }
    }
    rows
}

pub(crate) fn line_number_width<'a>(
    hunks: impl Iterator<Item = (&'a [DiffLine], Option<HunkCoordinates>)>,
) -> usize {
    hunks
        .flat_map(|(lines, coordinates)| numbered_lines(lines, coordinates))
        .flat_map(|line| [line.old_number, line.new_number])
        .flatten()
        .map(decimal_width)
        .max()
        .unwrap_or(1)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn split_hunk_lines(
    lines: &[DiffLine],
    coordinates: Option<HunkCoordinates>,
    available_width: u16,
    number_width: usize,
    selected: bool,
    search: Option<(usize, &str)>,
    syntax: &HunkSyntax,
    theme: SemanticTheme,
) -> Vec<Line> {
    let available_width = usize::from(available_width);
    let state_width = 3;
    let columns_width = available_width
        .saturating_sub(state_width)
        .saturating_sub(UnicodeWidthStr::width(SPLIT_SEPARATOR));
    let old_width = columns_width / 2;
    let new_width = columns_width.saturating_sub(old_width);
    let numbered = numbered_lines(lines, coordinates);
    let evidence = hunk_evidence(lines, search);
    split_rows(&numbered)
        .into_iter()
        .map(|row| {
            if let Some(metadata) = row
                .old
                .as_ref()
                .filter(|line| line.line.kind == DiffLineKind::Meta)
            {
                return metadata_row(&metadata.line.text, available_width, selected, theme);
            }
            let old = split_cell(
                row.old.as_ref(),
                Side::Old,
                old_width,
                number_width,
                row.old
                    .as_ref()
                    .and_then(|line| evidence.get(line.source_index)),
                syntax.old(row.old.as_ref().map_or(0, |line| line.source_index)),
                theme,
            );
            let new = split_cell(
                row.new.as_ref(),
                Side::New,
                new_width,
                number_width,
                row.new
                    .as_ref()
                    .and_then(|line| evidence.get(line.source_index)),
                syntax.new_side(row.new.as_ref().map_or(0, |line| line.source_index)),
                theme,
            );
            let old_evidence = row
                .old
                .as_ref()
                .and_then(|line| evidence.get(line.source_index));
            let new_evidence = row
                .new
                .as_ref()
                .and_then(|line| evidence.get(line.source_index));
            let combined_evidence = merge_evidence(old_evidence, new_evidence);
            let mut spans = vec![state_gutter(selected, combined_evidence.as_ref(), theme)];
            spans.extend(old);
            spans.push(Span::raw(SPLIT_SEPARATOR));
            spans.extend(new);
            Line::from(spans)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stack_hunk_lines(
    lines: &[DiffLine],
    coordinates: Option<HunkCoordinates>,
    available_width: u16,
    number_width: usize,
    wrap: bool,
    selected: bool,
    search: Option<(usize, &str)>,
    syntax: &HunkSyntax,
    theme: SemanticTheme,
) -> Vec<Line> {
    let available_width = usize::from(available_width);
    let prefix_width = number_width.saturating_mul(2).saturating_add(8);
    let content_width = available_width.saturating_sub(prefix_width);
    let mut rendered = Vec::new();
    let evidence = hunk_evidence(lines, search);
    for (line_index, line) in numbered_lines(lines, coordinates).into_iter().enumerate() {
        if line.line.kind == DiffLineKind::Meta {
            rendered.push(metadata_row(
                &line.line.text,
                available_width,
                selected,
                theme,
            ));
            continue;
        }
        let base = theme.diff_row_style(line_tone(line.line.kind));
        let content_rows = match line.line.kind {
            DiffLineKind::Removed => highlighted_content_rows(
                &line.line.text,
                content_width,
                wrap,
                &base,
                evidence.get(line_index),
                syntax.old(line.source_index),
                theme,
            ),
            DiffLineKind::Added => highlighted_content_rows(
                &line.line.text,
                content_width,
                wrap,
                &base,
                evidence.get(line_index),
                syntax.new_side(line.source_index),
                theme,
            ),
            DiffLineKind::Context => highlighted_content_rows(
                &line.line.text,
                content_width,
                wrap,
                &base,
                evidence.get(line_index),
                syntax.old(line.source_index),
                theme,
            ),
            DiffLineKind::Meta => unreachable!("metadata rows return above"),
        };
        for (index, content) in content_rows.into_iter().enumerate() {
            let first = index == 0;
            let old_number = first.then_some(line.old_number).flatten();
            let new_number = first.then_some(line.new_number).flatten();
            let marker = if first {
                change_marker(line.line.kind)
            } else {
                '↪'
            };
            let mut spans = vec![state_gutter(selected, evidence.get(line_index), theme)];
            spans.push(Span::styled(
                format!(
                    "{:>width$} {:>width$} │ {marker}",
                    format_number(old_number),
                    format_number(new_number),
                    width = number_width,
                ),
                base.clone(),
            ));
            spans.extend(content);
            rendered.push(Line::from(spans));
        }
    }
    rendered
}

#[derive(Debug, Clone, Copy)]
enum Side {
    Old,
    New,
}

#[allow(clippy::too_many_arguments)]
fn split_cell(
    line: Option<&NumberedLine>,
    side: Side,
    width: usize,
    number_width: usize,
    evidence: Option<&LineEvidence>,
    syntax: Option<&SyntaxLine>,
    theme: SemanticTheme,
) -> Vec<Span> {
    let Some(line) = line else {
        return vec![Span::raw(" ".repeat(width))];
    };
    let number = match side {
        Side::Old => line.old_number,
        Side::New => line.new_number,
    };
    let base = theme.diff_row_style(line_tone(line.line.kind));
    let prefix = format!(
        "{:>number_width$} {}",
        format_number(number),
        change_marker(line.line.kind),
    );
    let prefix_width = UnicodeWidthStr::width(prefix.as_str()).min(width);
    let mut spans = vec![Span::styled(fit_width(&prefix, prefix_width), base.clone())];
    spans.extend(
        highlighted_content_rows(
            &line.line.text,
            width.saturating_sub(prefix_width),
            false,
            &base,
            evidence,
            syntax,
            theme,
        )
        .into_iter()
        .next()
        .unwrap_or_default(),
    );
    spans
}

fn metadata_row(text: &str, available_width: usize, selected: bool, theme: SemanticTheme) -> Line {
    let style = theme.style(Tone::Attention);
    let mut spans = vec![state_gutter(selected, None, theme)];
    spans.push(Span::styled(
        fit_width(&patch_note(text), available_width.saturating_sub(3)),
        style,
    ));
    Line::from(spans)
}

/// Restates an in-hunk patch note as a review fact rather than patch syntax.
fn patch_note(text: &str) -> String {
    match text.strip_prefix("\\ ") {
        Some("No newline at end of file") => "no newline at EOF".to_owned(),
        Some(rest) => rest.to_owned(),
        None => text.to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn highlighted_content_rows(
    text: &str,
    width: usize,
    wrap: bool,
    base: &TextStyle,
    evidence: Option<&LineEvidence>,
    syntax: Option<&SyntaxLine>,
    theme: SemanticTheme,
) -> Vec<Vec<Span>> {
    if width == 0 {
        return vec![Vec::new()];
    }
    let expanded = expand_tabs(text);
    let mut range_index = 0usize;
    let graphemes = expanded
        .grapheme_indices(true)
        .map(|(byte_index, grapheme)| {
            let style = syntax.and_then(|ranges| {
                while range_index < ranges.len() && byte_index >= ranges[range_index].bytes.end {
                    range_index += 1;
                }
                ranges
                    .get(range_index)
                    .filter(|range| range.bytes.contains(&byte_index))
                    .map(|range| range.style)
            });
            (grapheme.to_owned(), style)
        })
        .collect();

    let chunks = chunk_graphemes(graphemes, width, wrap);
    let mut grapheme_offset: usize = 0;
    chunks
        .into_iter()
        .map(|chunk| {
            let chunk_start = grapheme_offset;
            grapheme_offset = grapheme_offset.saturating_add(chunk.len());
            let mut spans = chunk
                .into_iter()
                .enumerate()
                .map(|(index, (grapheme, syntax_style))| {
                    let index = chunk_start.saturating_add(index);
                    let mut style = syntax_style.map_or_else(
                        || base.clone(),
                        |source| merge_syntax_style(base.clone(), source, theme),
                    );
                    if evidence.is_some_and(|line| line.intraline.contains(&index)) {
                        // Preserve the syntax foreground and quiet row
                        // background. A bright span background can make token
                        // colors unreadable; underline and the pair gutter
                        // carry the local change evidence instead.
                        style = style.underlined();
                    }
                    if evidence.is_some_and(|line| line.search.contains(&index)) {
                        if theme.colors_enabled() {
                            style = style.background(Color::BLUE);
                        }
                        style = style.reverse();
                    }
                    Span::styled(grapheme, style)
                })
                .collect::<Vec<_>>();
            let used = spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_str()))
                .sum::<usize>();
            spans.push(Span::styled(
                " ".repeat(width.saturating_sub(used)),
                base.clone(),
            ));
            spans
        })
        .collect()
}

fn chunk_graphemes(
    graphemes: Vec<(String, Option<TokenStyle>)>,
    width: usize,
    wrap: bool,
) -> Vec<Vec<(String, Option<TokenStyle>)>> {
    if graphemes.is_empty() {
        return vec![Vec::new()];
    }
    if !wrap {
        let total_width = graphemes
            .iter()
            .map(|(grapheme, _)| UnicodeWidthStr::width(grapheme.as_str()))
            .sum::<usize>();
        let truncated = total_width > width;
        let limit = width.saturating_sub(usize::from(truncated));
        let mut used: usize = 0;
        let mut row = Vec::new();
        for (grapheme, style) in graphemes {
            let grapheme_width = UnicodeWidthStr::width(grapheme.as_str());
            if used.saturating_add(grapheme_width) > limit {
                break;
            }
            used = used.saturating_add(grapheme_width);
            row.push((grapheme, style));
        }
        if truncated {
            row.push(("…".into(), None));
        }
        return vec![row];
    }

    let mut rows = vec![Vec::new()];
    let mut used: usize = 0;
    for (grapheme, style) in graphemes {
        let grapheme_width = UnicodeWidthStr::width(grapheme.as_str());
        if used > 0 && used.saturating_add(grapheme_width) > width {
            rows.push(Vec::new());
            used = 0;
        }
        if grapheme_width > width {
            continue;
        }
        used = used.saturating_add(grapheme_width);
        if let Some(row) = rows.last_mut() {
            row.push((grapheme, style));
        }
    }
    rows
}

fn expand_tabs(value: &str) -> String {
    let mut expanded = String::new();
    let mut column = 0;
    for grapheme in value.graphemes(true) {
        if grapheme == "\t" {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.push_str(&" ".repeat(spaces));
            column = column.saturating_add(spaces);
        } else {
            expanded.push_str(grapheme);
            column = column.saturating_add(UnicodeWidthStr::width(grapheme));
        }
    }
    expanded
}

#[derive(Debug, Clone, Default)]
struct LineEvidence {
    intraline: std::ops::Range<usize>,
    search: std::ops::Range<usize>,
    paired: bool,
    current_search: bool,
}

fn hunk_evidence(lines: &[DiffLine], search: Option<(usize, &str)>) -> Vec<LineEvidence> {
    let mut evidence = vec![LineEvidence::default(); lines.len()];
    let mut index = 0;
    while index < lines.len() {
        if lines[index].kind != DiffLineKind::Removed {
            index += 1;
            continue;
        }
        let removed_start = index;
        while index < lines.len() && lines[index].kind == DiffLineKind::Removed {
            index += 1;
        }
        let added_start = index;
        while index < lines.len() && lines[index].kind == DiffLineKind::Added {
            index += 1;
        }
        for offset in 0..(added_start - removed_start).min(index - added_start) {
            let old_index = removed_start + offset;
            let new_index = added_start + offset;
            let (old_range, new_range) = replacement_ranges(
                &expand_tabs(&lines[old_index].text),
                &expand_tabs(&lines[new_index].text),
            );
            evidence[old_index].paired = true;
            evidence[new_index].paired = true;
            evidence[old_index].intraline = old_range;
            evidence[new_index].intraline = new_range;
        }
    }
    if let Some((line_index, query)) = search
        && !query.is_empty()
        && let Some(line) = lines.get(line_index)
    {
        let text = expand_tabs(&line.text);
        evidence[line_index].search = search_range(&text, query);
        evidence[line_index].current_search = true;
    }
    evidence
}

fn replacement_ranges(old: &str, new: &str) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let old = old.graphemes(true).collect::<Vec<_>>();
    let new = new.graphemes(true).collect::<Vec<_>>();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let old_suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_suffix
        .min(old.len().saturating_sub(prefix))
        .min(new.len().saturating_sub(prefix));
    (
        prefix..old.len().saturating_sub(suffix),
        prefix..new.len().saturating_sub(suffix),
    )
}

fn search_range(text: &str, query: &str) -> std::ops::Range<usize> {
    let mut lowered = String::new();
    let mut offsets = Vec::new();
    for grapheme in text.graphemes(true) {
        offsets.push(lowered.len());
        lowered.push_str(&grapheme.to_lowercase());
    }
    let query = query.to_lowercase();
    let Some(start) = lowered.find(&query) else {
        return 0..0;
    };
    let end = start.saturating_add(query.len());
    let first = offsets
        .partition_point(|offset| *offset <= start)
        .saturating_sub(1);
    let last = offsets
        .iter()
        .position(|offset| *offset >= end)
        .unwrap_or(offsets.len());
    first..last
}

fn state_gutter(selected: bool, evidence: Option<&LineEvidence>, theme: SemanticTheme) -> Span {
    let member = if selected { "┃" } else { " " };
    let search = if evidence.is_some_and(|line| line.current_search) {
        symbols::SEARCH_CHAR
    } else {
        ' '
    };
    Span::styled(
        format!("{member}{search} "),
        if selected {
            theme.style(Tone::FocusSelection)
        } else {
            TextStyle::new()
        },
    )
}

fn merge_evidence<'a>(
    old: Option<&'a LineEvidence>,
    new: Option<&'a LineEvidence>,
) -> Option<LineEvidence> {
    match (old, new) {
        (None, None) => None,
        (Some(line), None) | (None, Some(line)) => Some(line.clone()),
        (Some(old), Some(new)) => Some(LineEvidence {
            paired: old.paired || new.paired,
            current_search: old.current_search || new.current_search,
            ..LineEvidence::default()
        }),
    }
}

fn line_tone(kind: DiffLineKind) -> Tone {
    match kind {
        DiffLineKind::Added => Tone::ChangeAdded,
        DiffLineKind::Removed => Tone::ChangeRemoved,
        DiffLineKind::Context => Tone::MutedResolved,
        DiffLineKind::Meta => Tone::Attention,
    }
}

fn change_marker(kind: DiffLineKind) -> char {
    match kind {
        DiffLineKind::Added => '+',
        DiffLineKind::Removed => '-',
        DiffLineKind::Context => ' ',
        DiffLineKind::Meta => '\\',
    }
}

fn format_number(number: Option<usize>) -> String {
    number.map_or_else(String::new, |number| number.to_string())
}

fn decimal_width(number: usize) -> usize {
    number.to_string().len()
}

fn merge_syntax_style(base: TextStyle, source: TokenStyle, theme: SemanticTheme) -> TextStyle {
    let (red, green, blue) = source.foreground;
    let mut style = if theme.colors_enabled() {
        base.foreground(Color::Rgb(red, green, blue))
    } else {
        base
    };
    if source.bold {
        style = style.bold();
    }
    if source.italic {
        style = style.italic();
    }
    style
}

#[cfg(test)]
mod tests {
    use super::{
        PATH_FIT, PathFit, chunk_graphemes, expand_tabs, file_boundary, file_change_notes,
        fit_path_label, fit_path_label_with, line_number_width, numbered_lines, search_range,
        split_hunk_lines, split_rows, stack_hunk_lines, thread_card_rows, unique_prefix_segments,
    };
    use crate::app::view::{ThreadCard, ThreadState as SemanticThreadState};
    use crate::domain::anchor::{Anchor, HunkLocation};
    use crate::domain::diff::{
        DiffDocument, DiffLine, DiffLineKind, EDGE_FIXTURE, HunkCoordinates, HunkRange, Magnitude,
    };
    use crate::domain::thread::{Participant, ParticipantKind, ThreadState};
    use crate::presentation::renderer::SemanticTheme;
    use crate::presentation::syntax::{HunkSyntax, SyntaxHighlighter};
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    use urushi::{Color, TextAttribute};

    fn line(kind: DiffLineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.into(),
        }
    }

    fn hunk_syntax(lines: &[DiffLine], extension: &str) -> HunkSyntax {
        SyntaxHighlighter::default().highlight_hunk(Some(extension), lines)
    }

    fn edge_notes(path: &str) -> Vec<String> {
        let document = DiffDocument::parse(EDGE_FIXTURE);
        let file = document
            .files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("{path} is part of the edge fixture"));
        file_change_notes(file)
    }

    #[test]
    fn semantic_file_facts_replace_git_transport_headers() {
        assert_eq!(edge_notes("src/lib.rs"), Vec::<String>::new());
        assert_eq!(edge_notes("docs/added.md"), ["new file"]);
        assert_eq!(edge_notes("docs/removed.md"), ["deleted file"]);
        assert_eq!(edge_notes("assets/logo.png"), ["binary"]);
        assert_eq!(edge_notes("scripts/review.sh"), ["mode 100644 → 100755"]);
        assert_eq!(
            edge_notes("src/new_name.rs"),
            ["rename 92% · src/old_name.rs → src/new_name.rs"]
        );
    }

    #[test]
    fn file_boundaries_keep_magnitude_and_move_notes_to_their_own_row_when_narrow() {
        let magnitude = Magnitude {
            additions: 3,
            deletions: 1,
        };
        let notes = edge_notes("src/new_name.rs");

        let wide = file_boundary("F", "src/new_name.rs", 0, &notes, magnitude, 80);
        assert_eq!(wide.rows(), 1);
        assert!(wide.primary.starts_with("F src/new_name.rs  rename 92%"));
        assert!(wide.primary.ends_with("+3 -1"));
        assert_eq!(UnicodeWidthStr::width(wide.primary.as_str()), 80);

        let narrow = file_boundary("F", "src/new_name.rs", 0, &notes, magnitude, 48);
        assert_eq!(narrow.rows(), 2);
        assert!(narrow.primary.starts_with("F src/new_name.rs"));
        assert!(narrow.primary.ends_with("+3 -1"));
        let detail = narrow.detail.expect("the rename survives on its own row");
        assert!(detail.contains("rename 92%"));
        assert!(detail.contains("src/old_name.rs"));

        // The magnitude is never the first thing given up.
        let tiny = file_boundary(
            "F",
            "src/very/deep/module/handler.rs",
            0,
            &[],
            magnitude,
            24,
        );
        assert!(tiny.primary.ends_with("+3 -1"));
        assert_eq!(UnicodeWidthStr::width(tiny.primary.as_str()), 24);
    }

    #[test]
    fn path_labels_keep_the_filename_and_the_leading_segment() {
        assert_eq!(
            fit_path_label("docs/design/limits/boundaries.md", 0, 40),
            "docs/design/limits/boundaries.md"
        );
        assert_eq!(
            fit_path_label("docs/design/limits/boundaries.md", 0, 24),
            "docs/…/boundaries.md"
        );
        assert_eq!(
            fit_path_label("docs/design/limits/boundaries.md", 0, 28),
            "docs/…/limits/boundaries.md"
        );
        // With no room for any prefix the filename still survives intact.
        assert_eq!(
            fit_path_label("docs/design/limits/boundaries.md", 0, 15),
            "…/boundaries.md"
        );
    }

    #[test]
    fn path_shortening_is_selected_in_one_place() {
        let path = "docs/design/limits/boundaries.md";
        assert_eq!(
            fit_path_label(path, 0, 24),
            fit_path_label_with(PATH_FIT, path, 0, 24)
        );
        // The strategies are genuinely different, so swapping PATH_FIT is a
        // real switch rather than a decorative one.
        assert_ne!(
            fit_path_label_with(PathFit::MiddleElision, path, 0, 24),
            fit_path_label_with(PathFit::TailOnly, path, 0, 24)
        );
        assert_eq!(
            fit_path_label_with(PathFit::TailOnly, path, 0, 24),
            "…gn/limits/boundaries.md".to_owned()
        );
    }

    #[test]
    fn same_filename_in_different_directories_stays_distinguishable() {
        let paths = [
            "src/alpha/deeply/nested/mod.rs",
            "src/beta/deeply/nested/mod.rs",
        ];
        let prefix = |path: &str| unique_prefix_segments(path, paths.iter().copied());
        // "src" is shared, so both labels must keep two segments.
        assert_eq!(prefix(paths[0]), 2);

        let alpha = fit_path_label(paths[0], prefix(paths[0]), 20);
        let beta = fit_path_label(paths[1], prefix(paths[1]), 20);
        assert_ne!(alpha, beta);
        assert!(alpha.ends_with("mod.rs") && beta.ends_with("mod.rs"));
        assert!(UnicodeWidthStr::width(alpha.as_str()) <= 20);
        assert!(UnicodeWidthStr::width(beta.as_str()) <= 20);
    }

    #[test]
    fn lifecycle_cards_are_width_bounded_and_resolved_cards_fold_with_provenance() {
        let human = Participant {
            id: "レビュー担当".into(),
            kind: ParticipantKind::Human,
        };
        let mut threads = ThreadState::default();
        let id = threads.post(
            Anchor::new("deadbeef", HunkLocation::new("画面.rs", "@@ -1 +1 @@")),
            human,
            "message".into(),
            1,
        );
        let compact = ThreadCard {
            id,
            state: SemanticThreadState::Resolved,
            resolved: true,
            outdated: true,
            active: false,
            expanded: false,
            message_count: 4,
            closed_by: Some("レビュー担当".into()),
            latest: "human: 長い画面メッセージ👨‍👩‍👧‍👦 that must wrap without crossing the card edge"
                .into(),
        };

        let compact_rows = thread_card_rows(&compact, 48, true);
        assert_eq!(compact_rows.len(), 2);
        assert!(compact_rows[0].contains("#000"));
        assert!(compact_rows[0].contains("RESOLVED"));
        assert!(compact_rows.join("").contains("closed by レビュー担当"));

        let expanded_rows = thread_card_rows(
            &ThreadCard {
                active: true,
                ..compact
            },
            28,
            true,
        );
        assert!(expanded_rows.len() >= 3);
        assert!(expanded_rows[0].contains("▶ ACTIVE"));
        assert!(
            expanded_rows
                .iter()
                .all(|row| UnicodeWidthStr::width(row.as_str()) <= 28)
        );
        assert!(expanded_rows.iter().any(|row| row.contains('…')));
    }

    #[test]
    fn pairs_replacement_runs_and_retains_one_sided_rows() {
        let numbered = numbered_lines(
            &[
                line(DiffLineKind::Removed, "old one"),
                line(DiffLineKind::Removed, "old two"),
                line(DiffLineKind::Added, "new one"),
            ],
            Some(HunkCoordinates {
                old: HunkRange {
                    start: 10,
                    count: 2,
                },
                new: HunkRange {
                    start: 20,
                    count: 1,
                },
            }),
        );
        let rows = split_rows(&numbered);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].old.as_ref().unwrap().line.text, "old one");
        assert_eq!(rows[0].new.as_ref().unwrap().line.text, "new one");
        assert_eq!(rows[0].old.as_ref().unwrap().old_number, Some(10));
        assert_eq!(rows[0].new.as_ref().unwrap().new_number, Some(20));
        assert_eq!(rows[1].old.as_ref().unwrap().line.text, "old two");
        assert!(rows[1].new.is_none());
    }

    #[test]
    fn numbers_each_side_from_the_parsed_hunk_coordinates() {
        let lines = numbered_lines(
            &[
                line(DiffLineKind::Context, "before"),
                line(DiffLineKind::Removed, "old"),
                line(DiffLineKind::Added, "new"),
                line(DiffLineKind::Context, "after"),
                line(DiffLineKind::Meta, "\\ No newline at end of file"),
            ],
            Some(HunkCoordinates {
                old: HunkRange {
                    start: 40,
                    count: 3,
                },
                new: HunkRange {
                    start: 90,
                    count: 3,
                },
            }),
        );
        let numbers = lines
            .iter()
            .map(|line| (line.old_number, line.new_number))
            .collect::<Vec<_>>();
        assert_eq!(
            numbers,
            [
                (Some(40), Some(90)),
                (Some(41), None),
                (None, Some(91)),
                (Some(42), Some(92)),
                (None, None),
            ]
        );
    }

    #[test]
    fn gutter_width_spans_multiple_hunks() {
        let first = [line(DiffLineKind::Context, "first")];
        let second = [line(DiffLineKind::Added, "second")];
        assert_eq!(
            line_number_width(
                [
                    (
                        first.as_slice(),
                        Some(HunkCoordinates {
                            old: HunkRange { start: 9, count: 1 },
                            new: HunkRange { start: 9, count: 1 },
                        }),
                    ),
                    (
                        second.as_slice(),
                        Some(HunkCoordinates {
                            old: HunkRange {
                                start: 99,
                                count: 0
                            },
                            new: HunkRange {
                                start: 100,
                                count: 1
                            },
                        }),
                    ),
                ]
                .into_iter(),
            ),
            3
        );
    }

    #[test]
    fn tabs_expand_at_deterministic_four_column_stops() {
        assert_eq!(expand_tabs("a\tb\t画"), "a   b   画");
        assert_eq!(expand_tabs("画\tX"), "画  X");
    }

    #[test]
    fn truncation_and_wrapping_preserve_graphemes_at_cell_boundaries() {
        let cases = [
            ("abcd", 3, false, vec!["ab…"]),
            ("abcd", 3, true, vec!["abc", "d"]),
            ("a画b", 3, false, vec!["a…"]),
            ("a画b", 4, false, vec!["a画b"]),
            ("a画b", 3, true, vec!["a画", "b"]),
            ("e\u{301}x", 1, false, vec!["…"]),
            ("e\u{301}x", 2, false, vec!["e\u{301}x"]),
            ("e\u{301}x", 1, true, vec!["e\u{301}", "x"]),
            ("a👨‍👩‍👧‍👦b", 3, false, vec!["a…"]),
            ("a👨‍👩‍👧‍👦b", 4, false, vec!["a👨‍👩‍👧‍👦b"]),
            ("a👨‍👩‍👧‍👦b", 3, true, vec!["a👨‍👩‍👧‍👦", "b"]),
        ];

        for (value, width, wrap, expected) in cases {
            let graphemes = value
                .graphemes(true)
                .map(|grapheme| (grapheme.to_owned(), None))
                .collect();
            let actual = chunk_graphemes(graphemes, width, wrap)
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|(grapheme, _)| grapheme)
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "value={value:?}, width={width}, wrap={wrap}"
            );
            assert!(
                actual
                    .iter()
                    .all(|row| UnicodeWidthStr::width(row.as_str()) <= width)
            );
        }
    }

    #[test]
    fn zero_length_and_one_sided_split_ranges_keep_their_canonical_side() {
        let added = [
            line(DiffLineKind::Added, "first"),
            line(DiffLineKind::Added, "second"),
            line(DiffLineKind::Meta, "\\ No newline at end of file"),
        ];
        let syntax = hunk_syntax(&added, "txt");
        let rendered = split_hunk_lines(
            &added,
            Some(HunkCoordinates {
                old: HunkRange { start: 0, count: 0 },
                new: HunkRange { start: 5, count: 2 },
            }),
            48,
            2,
            false,
            None,
            &syntax,
            SemanticTheme::from_no_color(None),
        )
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content)
                .collect::<String>()
        })
        .collect::<Vec<_>>();

        assert!(rendered[0].contains(" │  5 +first"));
        assert!(rendered[1].contains(" │  6 +second"));
        assert!(!rendered[0].contains(" 0 "));
        assert!(!rendered[2].contains('\\'));
        assert_eq!(rendered[2].matches("no newline at EOF").count(), 1);
        assert_eq!(UnicodeWidthStr::width(rendered[2].as_str()), 48);

        let removed = [line(DiffLineKind::Removed, "gone")];
        let syntax = hunk_syntax(&removed, "txt");
        let rendered = split_hunk_lines(
            &removed,
            Some(HunkCoordinates {
                old: HunkRange {
                    start: 12,
                    count: 1,
                },
                new: HunkRange {
                    start: 20,
                    count: 0,
                },
            }),
            48,
            2,
            false,
            None,
            &syntax,
            SemanticTheme::from_no_color(None),
        );
        let text = rendered[0]
            .spans
            .iter()
            .map(|span| span.content.as_str())
            .collect::<String>();
        assert!(text.contains("12 -gone"));
        assert!(!text.contains("20"));
    }

    #[test]
    fn split_and_stack_compose_syntax_pair_search_and_selected_hunk_evidence() {
        let lines = [
            line(DiffLineKind::Context, "let stable = 1;"),
            line(DiffLineKind::Removed, "let timeout = 30;"),
            line(DiffLineKind::Added, "let timeout = 60;"),
        ];
        let coordinates = Some(HunkCoordinates {
            old: HunkRange {
                start: 10,
                count: 2,
            },
            new: HunkRange {
                start: 10,
                count: 2,
            },
        });
        let syntax = hunk_syntax(&lines, "rs");
        let split = split_hunk_lines(
            &lines,
            coordinates,
            100,
            2,
            true,
            Some((2, "60")),
            &syntax,
            SemanticTheme::from_no_color(None),
        );
        let split_text = split
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(split_text.iter().any(|row| row.contains("┃ ")));
        let changed = split
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == "6")
            .expect("replacement token is rendered");
        assert!(changed.style.get_underline().is_some());
        assert!(
            changed
                .style
                .get_attributes()
                .contains(TextAttribute::Reversed)
        );

        let stack = stack_hunk_lines(
            &lines,
            coordinates,
            80,
            2,
            false,
            false,
            Some((2, "60")),
            &syntax,
            SemanticTheme::from_no_color(None),
        );
        let stack_text = stack
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(stack_text.iter().any(|row| row.starts_with("   ")));
        let changed = stack
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == "6")
            .expect("replacement token is rendered");
        assert!(changed.style.get_underline().is_some());
        assert!(
            changed
                .style
                .get_attributes()
                .contains(TextAttribute::Reversed)
        );
    }

    #[test]
    fn search_ranges_follow_case_insensitive_matches_without_splitting_graphemes() {
        assert_eq!(search_range("INSERTED", "inserted"), 0..8);
        assert_eq!(search_range("x画INSERTED", "inserted"), 2..10);
        assert_eq!(search_range("e\u{301}INSERTED", "inserted"), 1..9);
        assert_eq!(search_range("aB", "b"), 1..2);
    }

    #[test]
    fn split_uses_independent_syntax_state_for_old_and_new_sides() {
        let changed = [
            line(DiffLineKind::Removed, "/* old side remains open"),
            line(DiffLineKind::Added, "let replacement = 1;"),
        ];
        let clean = [line(DiffLineKind::Added, "let replacement = 1;")];
        let semantic_theme = SemanticTheme::from_no_color(None);
        let changed_syntax = hunk_syntax(&changed, "rs");
        let clean_syntax = hunk_syntax(&clean, "rs");
        let changed_rows = split_hunk_lines(
            &changed,
            None,
            100,
            1,
            false,
            None,
            &changed_syntax,
            semantic_theme,
        );
        let clean_rows = split_hunk_lines(
            &clean,
            None,
            100,
            1,
            false,
            None,
            &clean_syntax,
            semantic_theme,
        );
        let changed_style = changed_rows
            .iter()
            .find(|row| row.spans.iter().any(|span| span.content == "r"))
            .and_then(|row| {
                row.spans
                    .iter()
                    .skip_while(|span| span.content != " │ ")
                    .find(|span| span.content == "l")
            })
            .map(|span| span.style.clone())
            .expect("new-side keyword is rendered");
        let clean_style = clean_rows
            .iter()
            .flat_map(|row| &row.spans)
            .find(|span| span.content == "l")
            .map(|span| span.style.clone())
            .expect("clean keyword is rendered");
        assert_eq!(changed_style.get_foreground(), clean_style.get_foreground());

        let changed_stack = stack_hunk_lines(
            &changed,
            None,
            100,
            1,
            false,
            false,
            None,
            &changed_syntax,
            semantic_theme,
        );
        let clean_stack = stack_hunk_lines(
            &clean,
            None,
            100,
            1,
            false,
            false,
            None,
            &clean_syntax,
            semantic_theme,
        );
        let changed_style = changed_stack
            .get(1)
            .and_then(|row| row.spans.iter().find(|span| span.content == "l"))
            .map(|span| span.style.clone())
            .expect("new-side stack syntax is rendered");
        let clean_style = clean_stack
            .first()
            .and_then(|row| row.spans.iter().find(|span| span.content == "l"))
            .map(|span| span.style.clone())
            .expect("clean stack syntax is rendered");
        assert_eq!(changed_style.get_foreground(), clean_style.get_foreground());
    }

    #[test]
    fn layouts_compose_selection_syntax_search_and_intraline_evidence() {
        let lines = [
            line(DiffLineKind::Context, "let stable = 1;"),
            line(DiffLineKind::Removed, "let timeout = 30;"),
            line(DiffLineKind::Added, "let timeout = 60;"),
        ];
        let coordinates = Some(HunkCoordinates {
            old: HunkRange {
                start: 10,
                count: 2,
            },
            new: HunkRange {
                start: 10,
                count: 2,
            },
        });
        let syntax = hunk_syntax(&lines, "rs");

        for (layout, selected) in [
            ("split", false),
            ("split", true),
            ("stack", false),
            ("stack", true),
        ] {
            let rendered = if layout == "split" {
                split_hunk_lines(
                    &lines,
                    coordinates,
                    100,
                    2,
                    selected,
                    Some((2, "60")),
                    &syntax,
                    SemanticTheme::from_no_color(None),
                )
            } else {
                stack_hunk_lines(
                    &lines,
                    coordinates,
                    100,
                    2,
                    false,
                    selected,
                    Some((2, "60")),
                    &syntax,
                    SemanticTheme::from_no_color(None),
                )
            };
            let expected_member = if selected { '┃' } else { ' ' };
            let replacement = rendered
                .iter()
                .find(|row| {
                    row.spans
                        .iter()
                        .map(|span| span.content.as_str())
                        .collect::<String>()
                        .contains("60")
                })
                .expect("replacement row is rendered");
            let gutter = replacement.spans.first().expect("state gutter");
            assert!(gutter.content.starts_with(expected_member));
            assert!(
                gutter
                    .content
                    .contains(crate::presentation::symbols::SEARCH_CHAR)
            );
            assert!(!gutter.content.contains('≈'));
            let changed = replacement
                .spans
                .iter()
                .find(|span| span.content == "6")
                .expect("changed grapheme is rendered");
            assert!(changed.style.get_underline().is_some());
            assert_ne!(changed.style.get_background(), Some(Color::YELLOW));
            for source_grapheme in ["l", "1", ";"] {
                assert!(
                    rendered.iter().flat_map(|row| &row.spans).any(|span| {
                        span.content == source_grapheme && span.style.get_foreground().is_some()
                    }),
                    "{layout}, selected={selected}: syntax foreground missing for {source_grapheme:?}"
                );
            }
        }
    }

    #[test]
    fn no_color_and_narrow_split_keep_non_color_change_and_search_evidence() {
        let lines = [
            line(DiffLineKind::Removed, "let timeout = 30;"),
            line(DiffLineKind::Added, "let timeout = 60;"),
        ];
        let syntax = hunk_syntax(&lines, "rs");
        for layout in ["split", "stack"] {
            let rendered = if layout == "split" {
                split_hunk_lines(
                    &lines,
                    None,
                    48,
                    2,
                    true,
                    Some((1, "60")),
                    &syntax,
                    SemanticTheme::no_color(),
                )
            } else {
                stack_hunk_lines(
                    &lines,
                    None,
                    48,
                    2,
                    false,
                    true,
                    Some((1, "60")),
                    &syntax,
                    SemanticTheme::no_color(),
                )
            };
            let changed = rendered
                .iter()
                .flat_map(|row| &row.spans)
                .find(|span| span.content == "6")
                .expect("searched changed grapheme is rendered");
            assert!(changed.style.get_underline().is_some());
            assert!(
                changed
                    .style
                    .get_attributes()
                    .contains(TextAttribute::Reversed)
            );
            let replacement = rendered
                .iter()
                .find(|row| {
                    row.spans
                        .iter()
                        .map(|span| span.content.as_str())
                        .collect::<String>()
                        .contains("60")
                })
                .expect("replacement row is rendered");
            let gutter = replacement.spans.first().expect("state gutter");
            assert!(
                gutter
                    .content
                    .contains(crate::presentation::symbols::SEARCH_CHAR)
            );
            assert!(!gutter.content.contains('≈'));
            if layout == "split" {
                let text = replacement
                    .spans
                    .iter()
                    .map(|span| span.content.as_str())
                    .collect::<String>();
                assert!(text.contains('-'));
                assert!(text.contains('+'));
                assert!(UnicodeWidthStr::width(text.as_str()) <= 48);
            }
        }
    }
}
