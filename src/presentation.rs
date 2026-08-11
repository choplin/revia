//! Pure diff-to-terminal presentation transforms.
//!
//! The renderer owns widget layout. This module owns only the conversion from
//! parsed Git hunks to width-bounded terminal rows. Source coordinates remain
//! attached to the parsed hunk; display rows are deliberately ephemeral.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use syntect::{easy::HighlightLines, highlighting::Style as SyntectStyle, parsing::SyntaxSet};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    anchor::HunkLocation,
    diff::{DiffLine, DiffLineKind, HunkCoordinates},
    renderer::SemanticTheme,
    semantic::{DiffSearchTarget, LayoutPolicy, ReviewBody, StickyReviewContext, Tone},
    ui::{LayoutMode, fit_width},
};

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
        cursor = cursor.saturating_add(2 + file.metadata.len());
        let number_width = line_number_width(
            file.hunks
                .iter()
                .map(|hunk| (hunk.lines.as_slice(), hunk.coordinates)),
        );
        let mut hunks = Vec::with_capacity(file.hunks.len());
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let start = cursor;
            cursor = cursor.saturating_add(usize::from(hunk.header.is_some()));
            let (diff_rows, line_rows) = hunk_line_rows(
                &hunk.lines,
                hunk.coordinates,
                cursor,
                available_width,
                number_width,
                layout,
            );
            cursor = cursor.saturating_add(diff_rows);
            let active_thread_row = hunk
                .threads
                .iter()
                .position(|thread| thread.active)
                .map(|index| cursor.saturating_add(index.saturating_mul(3)));
            cursor = cursor.saturating_add(hunk.threads.len().saturating_mul(3));
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

fn hunk_line_rows(
    lines: &[DiffLine],
    coordinates: Option<HunkCoordinates>,
    start: usize,
    available_width: u16,
    number_width: usize,
    layout: LayoutPolicy,
) -> (usize, Vec<usize>) {
    match layout.diff_layout.resolved(available_width) {
        LayoutMode::Split => {
            let numbered = numbered_lines(lines, coordinates);
            let rows = split_rows(&numbered);
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
            (rows.len(), line_rows)
        }
        LayoutMode::Stack | LayoutMode::Auto => {
            let content_width = usize::from(available_width)
                .saturating_sub(number_width.saturating_mul(2).saturating_add(7));
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
                        wrapped_row_count(&expand_tabs(&line.text), content_width)
                    };
                    cursor = cursor.saturating_add(height);
                    row
                })
                .collect();
            (cursor.saturating_sub(start), line_rows)
        }
    }
}

pub(crate) fn search_line_row(
    lines: &[DiffLine],
    coordinates: Option<HunkCoordinates>,
    line_index: usize,
    available_width: u16,
    number_width: usize,
    layout: LayoutPolicy,
) -> Option<usize> {
    hunk_line_rows(lines, coordinates, 0, available_width, number_width, layout)
        .1
        .get(line_index)
        .copied()
}

fn wrapped_row_count(value: &str, width: usize) -> usize {
    chunk_graphemes(
        value
            .graphemes(true)
            .map(|grapheme| (grapheme.to_owned(), None))
            .collect(),
        width,
        true,
    )
    .len()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumberedLine {
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
        .map(|line| {
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

pub(crate) fn split_hunk_lines(
    lines: &[DiffLine],
    coordinates: Option<HunkCoordinates>,
    available_width: u16,
    number_width: usize,
    selected: bool,
    theme: SemanticTheme,
) -> Vec<Line<'static>> {
    let available_width = usize::from(available_width);
    let selection_width = 2;
    let columns_width = available_width
        .saturating_sub(selection_width)
        .saturating_sub(UnicodeWidthStr::width(SPLIT_SEPARATOR));
    let old_width = columns_width / 2;
    let new_width = columns_width.saturating_sub(old_width);
    let numbered = numbered_lines(lines, coordinates);

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
                selected,
                theme,
            );
            let new = split_cell(
                row.new.as_ref(),
                Side::New,
                new_width,
                number_width,
                selected,
                theme,
            );
            let mut spans = vec![selection_span(selected, theme)];
            spans.extend(old);
            spans.push(Span::styled(
                SPLIT_SEPARATOR,
                selected_style(selected, theme),
            ));
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
    highlighter: &mut HighlightLines<'_>,
    syntax_set: &SyntaxSet,
    theme: SemanticTheme,
) -> Vec<Line<'static>> {
    let available_width = usize::from(available_width);
    let prefix_width = number_width.saturating_mul(2).saturating_add(7);
    let content_width = available_width.saturating_sub(prefix_width);
    let mut rendered = Vec::new();

    for line in numbered_lines(lines, coordinates) {
        if line.line.kind == DiffLineKind::Meta {
            rendered.push(metadata_row(
                &line.line.text,
                available_width,
                selected,
                theme,
            ));
            continue;
        }
        let base = selected_style(selected, theme).patch(theme.style(line_tone(line.line.kind)));
        let content_rows = highlighted_content_rows(
            &line.line.text,
            content_width,
            wrap,
            base,
            theme.colors_enabled() && !selected && line.line.kind == DiffLineKind::Context,
            highlighter,
            syntax_set,
        );
        for (index, content) in content_rows.into_iter().enumerate() {
            let first = index == 0;
            let old_number = first.then_some(line.old_number).flatten();
            let new_number = first.then_some(line.new_number).flatten();
            let marker = if first {
                change_marker(line.line.kind)
            } else {
                '↪'
            };
            let mut spans = vec![selection_span(selected, theme)];
            spans.push(Span::styled(
                format!(
                    "{:>width$} {:>width$} │ {marker}",
                    format_number(old_number),
                    format_number(new_number),
                    width = number_width,
                ),
                base,
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

fn split_cell(
    line: Option<&NumberedLine>,
    side: Side,
    width: usize,
    number_width: usize,
    selected: bool,
    theme: SemanticTheme,
) -> Vec<Span<'static>> {
    let Some(line) = line else {
        return vec![Span::styled(
            " ".repeat(width),
            selected_style(selected, theme),
        )];
    };
    let number = match side {
        Side::Old => line.old_number,
        Side::New => line.new_number,
    };
    let base = selected_style(selected, theme).patch(theme.style(line_tone(line.line.kind)));
    let prefix = format!(
        "{:>number_width$} {}",
        format_number(number),
        change_marker(line.line.kind),
    );
    let prefix_width = UnicodeWidthStr::width(prefix.as_str()).min(width);
    let content = expand_tabs(&line.line.text);
    vec![
        Span::styled(fit_width(&prefix, prefix_width), base),
        Span::styled(
            fit_width(&content, width.saturating_sub(prefix_width)),
            base,
        ),
    ]
}

fn metadata_row(
    text: &str,
    available_width: usize,
    selected: bool,
    theme: SemanticTheme,
) -> Line<'static> {
    let style = selected_style(selected, theme).patch(theme.style(Tone::Attention));
    let mut spans = vec![selection_span(selected, theme)];
    spans.push(Span::styled(
        fit_width(text, available_width.saturating_sub(2)),
        style,
    ));
    Line::from(spans)
}

#[allow(clippy::too_many_arguments)]
fn highlighted_content_rows(
    text: &str,
    width: usize,
    wrap: bool,
    base: Style,
    allow_syntax: bool,
    highlighter: &mut HighlightLines<'_>,
    syntax_set: &SyntaxSet,
) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        if highlighter.highlight_line(text, syntax_set).is_err() {
            // Syntax highlighting is optional presentation; the diff row and
            // highlighter state remain safe to render with semantic styling.
        }
        return vec![Vec::new()];
    }
    let expanded = expand_tabs(text);
    let ranges = highlighter.highlight_line(&expanded, syntax_set).ok();
    let mut graphemes = Vec::new();
    if let Some(ranges) = ranges.filter(|ranges| !ranges.is_empty()) {
        let mut range_index = 0;
        let mut range_end = ranges.first().map_or(0, |(_, text)| text.len());
        for (byte_index, grapheme) in expanded.grapheme_indices(true) {
            while byte_index >= range_end && range_index + 1 < ranges.len() {
                range_index += 1;
                range_end = range_end.saturating_add(ranges[range_index].1.len());
            }
            graphemes.push((grapheme.to_owned(), Some(ranges[range_index].0)));
        }
    } else {
        graphemes.extend(
            expanded
                .graphemes(true)
                .map(|grapheme| (grapheme.to_owned(), None)),
        );
    }

    let chunks = chunk_graphemes(graphemes, width, wrap);
    chunks
        .into_iter()
        .map(|chunk| {
            let mut spans = chunk
                .into_iter()
                .map(|(grapheme, syntax_style)| {
                    let style = if allow_syntax {
                        syntax_style.map_or(base, |source| merge_syntect_style(base, source))
                    } else {
                        base
                    };
                    Span::styled(grapheme, style)
                })
                .collect::<Vec<_>>();
            let used = spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), base));
            spans
        })
        .collect()
}

fn chunk_graphemes(
    graphemes: Vec<(String, Option<SyntectStyle>)>,
    width: usize,
    wrap: bool,
) -> Vec<Vec<(String, Option<SyntectStyle>)>> {
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

fn selection_span(selected: bool, theme: SemanticTheme) -> Span<'static> {
    Span::styled(
        if selected { "┃ " } else { "  " },
        selected_style(selected, theme),
    )
}

fn selected_style(selected: bool, theme: SemanticTheme) -> Style {
    if selected {
        theme.selection()
    } else {
        Style::default()
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

fn merge_syntect_style(base: Style, source: SyntectStyle) -> Style {
    let foreground = source.foreground;
    let maximum = foreground.r.max(foreground.g).max(foreground.b);
    let minimum = foreground.r.min(foreground.g).min(foreground.b);
    let color = if maximum.saturating_sub(minimum) < 24 {
        Color::Gray
    } else if foreground.r == maximum {
        if foreground.g > maximum / 2 {
            Color::Yellow
        } else if foreground.b > maximum / 2 {
            Color::Magenta
        } else {
            Color::Red
        }
    } else if foreground.g == maximum {
        if foreground.b > maximum / 2 {
            Color::Cyan
        } else {
            Color::Green
        }
    } else if foreground.r > maximum / 2 {
        Color::Magenta
    } else {
        Color::Blue
    };
    base.fg(color)
}

#[cfg(test)]
mod tests {
    use super::{
        chunk_graphemes, expand_tabs, line_number_width, numbered_lines, split_hunk_lines,
        split_rows,
    };
    use crate::diff::{DiffLine, DiffLineKind, HunkCoordinates, HunkRange};
    use crate::renderer::SemanticTheme;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    fn line(kind: DiffLineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.into(),
        }
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
        let rendered = split_hunk_lines(
            &added,
            Some(HunkCoordinates {
                old: HunkRange { start: 0, count: 0 },
                new: HunkRange { start: 5, count: 2 },
            }),
            48,
            2,
            false,
            SemanticTheme::no_color(),
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
        assert_eq!(rendered[2].matches("\\ No newline").count(), 1);
        assert_eq!(UnicodeWidthStr::width(rendered[2].as_str()), 48);

        let removed = [line(DiffLineKind::Removed, "gone")];
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
            SemanticTheme::no_color(),
        );
        let text = rendered[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("12 -gone"));
        assert!(!text.contains("20"));
    }
}
