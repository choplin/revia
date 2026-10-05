use std::{cell::RefCell, collections::VecDeque, sync::Arc};

use super::{
    symbols,
    syntax::SyntaxHighlighter,
    text::{Document, Line, Span, patch_style},
};
use crate::{
    app::{
        view::{
            DiffSearchTarget, ReviewBody, ReviewWindowSection, RollupBody, StickyReviewContext,
            ThreadState, Tone, View,
        },
        view_state::{FocusArea, LayoutMode, ShellSize, truncate_end},
    },
    presentation,
};
use unicode_width::UnicodeWidthStr;
use urushi::{Color, TextStyle};
const HUNK_TEXT_CACHE_CAPACITY: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SemanticTheme {
    colors_enabled: bool,
}

/// Keeps the current file and hunk identity, and that file's magnitude, visible
/// while the diff scrolls past its own boundary rows.
///
/// The file and the hunk get a row each: sharing one row forced a long hunk
/// header to compete with the path, and both were clipped.
pub(crate) fn sticky_context_lines(context: &StickyReviewContext, width: u16) -> [String; 2] {
    let file = presentation::right_aligned_row(
        &format!(
            "{} {}/{} ",
            symbols::FILE,
            context.file_index + 1,
            context.file_count
        ),
        &context.file,
        &presentation::format_magnitude(context.magnitude),
        usize::from(width),
    );
    let hunk = match (context.hunk_index, context.hunk_header.as_deref()) {
        (Some(index), Some(header)) => {
            format!("   hunk {}/{}  {header}", index + 1, context.hunk_count)
        }
        (Some(index), None) => format!("   hunk {}/{}", index + 1, context.hunk_count),
        // A file with no hunks still has something worth keeping in view.
        (None, _) => format!("   {}", context.notes.join(" · ")),
    };
    [file, truncate_end(&hunk, usize::from(width))]
}

impl SemanticTheme {
    fn from_environment() -> Self {
        let no_color = std::env::var_os("NO_COLOR");
        Self::from_no_color(no_color.as_deref())
    }

    pub(crate) fn from_no_color(no_color: Option<&std::ffi::OsStr>) -> Self {
        Self {
            colors_enabled: no_color.is_none_or(std::ffi::OsStr::is_empty),
        }
    }

    #[cfg(test)]
    pub(crate) fn no_color() -> Self {
        Self {
            colors_enabled: false,
        }
    }

    pub(crate) fn style(self, tone: Tone) -> TextStyle {
        let base = match tone {
            Tone::FocusSelection | Tone::Attention => TextStyle::new().bold(),
            Tone::MutedResolved => TextStyle::new().dim(),
            Tone::ChangeAdded | Tone::ChangeRemoved => TextStyle::new(),
        };
        if !self.colors_enabled {
            return base;
        }
        base.foreground(match tone {
            Tone::ChangeAdded => Color::BRIGHT_GREEN,
            Tone::ChangeRemoved => Color::BRIGHT_RED,
            Tone::FocusSelection => Color::BRIGHT_CYAN,
            Tone::Attention => Color::BRIGHT_YELLOW,
            Tone::MutedResolved => Color::WHITE,
        })
    }

    pub(crate) fn header(self) -> TextStyle {
        let style = TextStyle::new().bold();
        if self.colors_enabled {
            style.foreground(Color::BRIGHT_CYAN)
        } else {
            style
        }
    }

    pub(crate) fn file_selection(self) -> TextStyle {
        TextStyle::new().bold()
    }

    pub(crate) fn file_status(self) -> TextStyle {
        if self.colors_enabled {
            TextStyle::new().foreground(Color::RED)
        } else {
            TextStyle::new()
        }
    }

    pub(crate) fn file_icon(self, color: Color) -> TextStyle {
        if self.colors_enabled {
            TextStyle::new().foreground(color)
        } else {
            TextStyle::new()
        }
    }

    pub(crate) fn selection(self) -> TextStyle {
        self.style(Tone::FocusSelection).reverse()
    }

    pub(crate) fn border(self, tone: Tone) -> TextStyle {
        if !self.colors_enabled {
            return TextStyle::new();
        }
        TextStyle::new().foreground(match tone {
            Tone::ChangeAdded => Color::BRIGHT_GREEN,
            Tone::ChangeRemoved => Color::BRIGHT_RED,
            Tone::FocusSelection => Color::BRIGHT_CYAN,
            Tone::Attention => Color::BRIGHT_YELLOW,
            Tone::MutedResolved => Color::BRIGHT_BLACK,
        })
    }

    pub(crate) fn modal_backdrop(self) -> TextStyle {
        let style = TextStyle::new().dim();
        if self.colors_enabled {
            style.foreground(Color::BRIGHT_BLACK)
        } else {
            style
        }
    }

    /// Row kind owns a quiet background only.  Source-token foregrounds and
    /// modifiers are applied later by the presentation layer.
    pub(crate) fn diff_row_style(self, tone: Tone) -> TextStyle {
        if !self.colors_enabled {
            return TextStyle::new();
        }
        let background = match tone {
            Tone::ChangeAdded => Color::Rgb(20, 46, 32),
            Tone::ChangeRemoved => Color::Rgb(54, 27, 32),
            Tone::FocusSelection | Tone::Attention | Tone::MutedResolved => {
                return TextStyle::new();
            }
        };
        TextStyle::new().background(background)
    }

    pub(crate) fn colors_enabled(self) -> bool {
        self.colors_enabled
    }
}
pub struct Renderer {
    syntax: SyntaxHighlighter,
    semantic_theme: SemanticTheme,
    hunk_text_cache: RefCell<VecDeque<HunkTextCache>>,
}

#[derive(Debug, Clone)]
struct HunkTextKey {
    available_width: u16,
    anchor: crate::domain::anchor::HunkLocation,
    header: Option<String>,
    coordinates: Option<crate::domain::diff::HunkCoordinates>,
    lines: Arc<Vec<crate::domain::diff::PatchLine>>,
    threads: Vec<crate::app::view::ThreadCard>,
    number_width: usize,
    layout: LayoutMode,
    wrap_lines: bool,
    header_match: bool,
    search: Option<(usize, String)>,
}

impl HunkTextKey {
    fn matches(&self, hunk: &crate::app::view::ReviewHunk, lookup: HunkTextLookup<'_>) -> bool {
        self.available_width == lookup.available_width
            && self.anchor == hunk.anchor
            && self.header == hunk.header
            && self.coordinates == hunk.coordinates
            && Arc::ptr_eq(&self.lines, &hunk.lines)
            && self.threads == hunk.threads
            && self.number_width == lookup.number_width
            && self.layout == lookup.layout
            && self.wrap_lines == lookup.wrap_lines
            && self.header_match == lookup.header_match
            && &self.search == lookup.search
    }
}

#[derive(Debug, Clone, Copy)]
struct HunkTextLookup<'a> {
    available_width: u16,
    number_width: usize,
    layout: LayoutMode,
    wrap_lines: bool,
    header_match: bool,
    search: &'a Option<(usize, String)>,
}

struct HunkTextCache {
    key: HunkTextKey,
    lines: Vec<Line>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self {
            syntax: SyntaxHighlighter::default(),
            semantic_theme: SemanticTheme::from_environment(),
            hunk_text_cache: RefCell::new(VecDeque::new()),
        }
    }
}

impl Renderer {
    pub(crate) fn semantic_theme(&self) -> SemanticTheme {
        self.semantic_theme
    }

    pub(crate) fn review_window(
        &self,
        review: &ReviewBody,
        available_width: u16,
        view: &View,
    ) -> Document {
        if let Some(message) = &review.empty_state {
            return Document::from(
                message
                    .lines()
                    .skip(review.scroll)
                    .take(review.viewport.visible_rows)
                    .map(|line| Line::raw(line.to_owned()))
                    .collect::<Vec<_>>(),
            );
        }

        let is_focused = view.layout.focus == FocusArea::Review;
        let window_start = review.scroll;
        let window_end = window_start.saturating_add(review.viewport.visible_rows);
        let mut lines = Vec::new();
        for section in review.viewport.sections.iter().copied() {
            let mut section_lines = match section {
                ReviewWindowSection::FileHeader { file_index, .. } => {
                    self.file_header_lines(file_index, review, available_width)
                }
                ReviewWindowSection::Hunk {
                    file_index,
                    hunk_index,
                    ..
                } => {
                    let file = &review.files[file_index];
                    let hunk = &file.hunks[hunk_index];
                    let number_width = presentation::line_number_width(
                        file.hunks
                            .iter()
                            .map(|hunk| (hunk.lines.as_slice(), hunk.coordinates)),
                    );
                    let mut rendered = self.cached_hunk_lines(
                        file,
                        hunk,
                        review,
                        available_width,
                        number_width,
                        view,
                    );
                    if hunk.selected {
                        // The selected hunk stays boxed wherever focus is, but
                        // it only claims the focus tone while the diff owns the
                        // keys.  That is what makes a focus switch visible on
                        // this side of the shell too.
                        highlight_hunk_box(
                            &mut rendered,
                            section.start(),
                            section.start(),
                            section.end(),
                            self.semantic_theme.border(if is_focused {
                                Tone::FocusSelection
                            } else {
                                Tone::MutedResolved
                            }),
                            is_focused,
                        );
                    }
                    rendered
                }
            };
            let local_start = window_start.saturating_sub(section.start());
            let local_end = section_lines
                .len()
                .min(window_end.saturating_sub(section.start()));
            if local_start < local_end {
                lines.extend(section_lines.drain(local_start..local_end));
            }
        }
        Document::from(lines)
    }

    fn file_header_lines(
        &self,
        file_index: usize,
        review: &ReviewBody,
        available_width: u16,
    ) -> Vec<Line> {
        let file = &review.files[file_index];
        let path_match = matches!(
            review.search_target.as_ref(),
            Some(DiffSearchTarget::FilePath { path }) if path == &file.path
        );
        let boundary = presentation::file_boundary(
            if path_match {
                symbols::SEARCH
            } else {
                symbols::FILE
            },
            &file.path,
            presentation::unique_prefix_segments(
                &file.path,
                review.files.iter().map(|file| file.path.as_str()),
            ),
            &file.notes,
            file.magnitude,
            available_width,
        );
        let separator_rows = presentation::file_separator_rows(file_index);
        let mut lines =
            Vec::with_capacity(separator_rows + 1 + usize::from(boundary.detail.is_some()));
        if separator_rows > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            boundary.primary,
            if path_match {
                self.semantic_theme.selection()
            } else {
                self.semantic_theme.style(Tone::Attention)
            },
        ));
        lines.extend(
            boundary
                .detail
                .map(|detail| Line::styled(detail, self.semantic_theme.style(Tone::MutedResolved))),
        );
        lines
    }

    #[allow(clippy::too_many_arguments)]
    fn cached_hunk_lines(
        &self,
        file: &crate::app::view::ReviewFile,
        hunk: &crate::app::view::ReviewHunk,
        review: &ReviewBody,
        available_width: u16,
        number_width: usize,
        view: &View,
    ) -> Vec<Line> {
        let content_width = available_width.saturating_sub(2);
        let layout = view.layout.diff_layout.resolved(content_width);
        let header_match = matches!(
            review.search_target.as_ref(),
            Some(DiffSearchTarget::HunkHeader { location }) if location == &hunk.anchor
        );
        let search = match (&review.search_target, review.search_query.as_deref()) {
            (
                Some(DiffSearchTarget::PatchLine {
                    location,
                    line_index,
                }),
                Some(query),
            ) if location == &hunk.anchor => Some((*line_index, query.to_owned())),
            _ => None,
        };
        if let Some(hit) = self.hunk_text_cache.borrow().iter().find(|entry| {
            entry.key.matches(
                hunk,
                HunkTextLookup {
                    available_width,
                    number_width,
                    layout,
                    wrap_lines: view.layout.wrap_lines,
                    header_match,
                    search: &search,
                },
            )
        }) {
            return hit.lines.clone();
        }

        let border_style = if header_match {
            self.semantic_theme.border(Tone::FocusSelection)
        } else {
            self.semantic_theme.border(Tone::MutedResolved)
        };
        let hunk_title = hunk.header.as_ref().map_or_else(
            || header_match.then(|| symbols::SEARCH.to_owned()),
            |header| {
                Some(if header_match {
                    format!("{} {header}", symbols::SEARCH)
                } else {
                    header.clone()
                })
            },
        );
        let syntax = self
            .syntax
            .highlight_hunk(file.extension.as_deref(), &hunk.lines);
        let source_search = search
            .as_ref()
            .map(|(line_index, query)| (*line_index, query.as_str()));
        let hunk_width = usize::from(available_width);
        let mut lines = vec![hunk_box_top(
            hunk_title.as_deref(),
            hunk_width,
            &border_style,
        )];
        let source_lines = if layout == LayoutMode::Split {
            presentation::split_hunk_lines(
                &hunk.lines,
                hunk.coordinates,
                content_width,
                number_width,
                false,
                source_search,
                &syntax,
                self.semantic_theme,
            )
        } else {
            presentation::stack_hunk_lines(
                &hunk.lines,
                hunk.coordinates,
                content_width,
                number_width,
                view.layout.wrap_lines,
                false,
                source_search,
                &syntax,
                self.semantic_theme,
            )
        };
        lines.extend(
            source_lines
                .into_iter()
                .map(|line| hunk_box_content(line, hunk_width, &border_style)),
        );
        if !hunk.threads.is_empty() {
            let active = hunk.threads.iter().any(|thread| thread.active);
            let style = if active {
                self.semantic_theme.selection()
            } else if hunk
                .threads
                .iter()
                .any(|thread| thread.state == ThreadState::NeedsAttention)
            {
                self.semantic_theme.style(Tone::Attention)
            } else {
                self.semantic_theme.style(Tone::MutedResolved)
            };
            let summary = format!(
                "  Comments ({}) · t/T open{}",
                hunk.threads.len(),
                if active { " · selected" } else { "" }
            );
            lines.push(hunk_box_content(
                Line::styled(summary, style),
                hunk_width,
                &border_style,
            ));
        }
        lines.push(hunk_box_bottom(hunk_width, &border_style));

        let key = HunkTextKey {
            available_width,
            anchor: hunk.anchor.clone(),
            header: hunk.header.clone(),
            coordinates: hunk.coordinates,
            lines: Arc::clone(&hunk.lines),
            threads: hunk.threads.clone(),
            number_width,
            layout,
            wrap_lines: view.layout.wrap_lines,
            header_match,
            search,
        };
        let mut cache = self.hunk_text_cache.borrow_mut();
        if cache.len() == HUNK_TEXT_CACHE_CAPACITY {
            cache.pop_front();
        }
        cache.push_back(HunkTextCache {
            key,
            lines: lines.clone(),
        });
        lines
    }
}

fn highlight_hunk_box(
    lines: &mut [Line],
    window_start: usize,
    range_start: usize,
    range_end: usize,
    style: TextStyle,
    thick: bool,
) {
    for (offset, line) in lines.iter_mut().enumerate() {
        let row = window_start.saturating_add(offset);
        if row < range_start || row >= range_end {
            continue;
        }
        if thick {
            let border_row = if row == range_start {
                HunkBorderRow::Top
            } else if row + 1 == range_end {
                HunkBorderRow::Bottom
            } else {
                HunkBorderRow::Middle
            };
            thicken_hunk_border(line, border_row);
        }
        if row == range_start || row + 1 == range_end {
            for span in &mut line.spans {
                span.style = patch_style(span.style.clone(), &style);
            }
        } else {
            if let Some(border) = line.spans.first_mut() {
                border.style = patch_style(border.style.clone(), &style);
            }
            if let Some(border) = line.spans.last_mut() {
                border.style = patch_style(border.style.clone(), &style);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HunkBorderRow {
    Top,
    Middle,
    Bottom,
}

fn thicken_hunk_border(line: &mut Line, row: HunkBorderRow) {
    let replace = |content: &str, left: char, horizontal: char, right: char| {
        content
            .chars()
            .map(|character| match character {
                '╭' | '╰' => left,
                '─' => horizontal,
                '╮' | '╯' => right,
                other => other,
            })
            .collect()
    };
    match row {
        HunkBorderRow::Top => {
            for span in &mut line.spans {
                span.content = replace(&span.content, '┏', '━', '┓');
            }
        }
        HunkBorderRow::Bottom => {
            for span in &mut line.spans {
                span.content = replace(&span.content, '┗', '━', '┛');
            }
        }
        HunkBorderRow::Middle => {
            if let Some(border) = line.spans.first_mut()
                && border.content == "│"
            {
                border.content = "┃".into();
            }
            if let Some(border) = line.spans.last_mut()
                && border.content == "│"
            {
                border.content = "┃".into();
            }
        }
    }
}

fn hunk_box_top(title: Option<&str>, width: usize, style: &TextStyle) -> Line {
    if width < 2 {
        return Line::styled("─".repeat(width), style.clone());
    }
    let title_width = width.saturating_sub(4);
    let title = title
        .filter(|title| !title.is_empty())
        .map(|title| truncate_end(title, title_width));
    let prefix = title
        .as_ref()
        .map_or_else(|| "╭".to_owned(), |title| format!("╭─ {title} "));
    let fill = width
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
        .saturating_sub(1);
    Line::styled(format!("{prefix}{}╮", "─".repeat(fill)), style.clone())
}

fn hunk_box_content(mut line: Line, width: usize, style: &TextStyle) -> Line {
    if width < 2 {
        return line;
    }
    let inner_width = width.saturating_sub(2);
    line = fit_line(line, inner_width);
    let padding = inner_width.saturating_sub(line.width());
    line.spans.insert(0, Span::styled("│", style.clone()));
    line.spans.push(Span::raw(" ".repeat(padding)));
    line.spans.push(Span::styled("│", style.clone()));
    line
}

fn fit_line(line: Line, width: usize) -> Line {
    let mut remaining = width;
    let mut spans = Vec::new();
    for span in line.spans {
        if remaining == 0 {
            break;
        }
        let span_width = span.width();
        if span_width <= remaining {
            remaining -= span_width;
            spans.push(span);
        } else {
            spans.push(Span::styled(
                truncate_end(span.content.as_str(), remaining),
                span.style,
            ));
            break;
        }
    }
    Line::from(spans)
}

fn hunk_box_bottom(width: usize, style: &TextStyle) -> Line {
    if width < 2 {
        return Line::styled("─".repeat(width), style.clone());
    }
    Line::styled(format!("╰{}╯", "─".repeat(width - 2)), style.clone())
}

pub(crate) fn rollup_text(
    rollup: &RollupBody,
    available_width: u16,
    theme: SemanticTheme,
) -> Document {
    let mut lines = vec![Line::styled(
        rollup.summary.clone(),
        TextStyle::new().bold(),
    )];
    for item in &rollup.items {
        let state = match item.state {
            ThreadState::NeedsAttention => "NEEDS ATTENTION",
            ThreadState::Open => "OPEN",
            ThreadState::Resolved => "RESOLVED",
        };
        let provenance = item
            .closed_by
            .as_ref()
            .map_or(String::new(), |actor| format!(" — closed by {actor}"));
        lines.push(Line::styled(
            truncate_end(
                &format!(
                    "{}#{id} [{state}] {} {}{provenance}",
                    if item.selected { " " } else { "  " },
                    item.path,
                    item.hunk_header,
                    id = item.id
                ),
                usize::from(available_width),
            ),
            if item.selected {
                theme.selection()
            } else if item.state == ThreadState::NeedsAttention {
                theme.style(Tone::Attention)
            } else if item.state == ThreadState::Resolved {
                theme.style(Tone::MutedResolved)
            } else {
                TextStyle::new()
            },
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        if rollup.items.is_empty() {
            "No thread targets • v/Esc return"
        } else {
            "j/k select • Enter jump • v/Esc return"
        },
        theme.style(Tone::MutedResolved),
    ));
    Document::from(lines)
}

/// The changeset header: comparison identity, file count, and total magnitude.
///
/// Every width keeps all three, shortening the comparison label before any
/// number is given up.
pub(crate) fn header_text(view: &View, width: u16, size: ShellSize) -> String {
    let header = &view.header;
    let magnitude = presentation::format_magnitude(header.magnitude);
    let (files, separator) = match size {
        ShellSize::Narrow => (format!("{}f", header.file_count), " "),
        _ => (format!("{} files", header.file_count), "  "),
    };
    let facts = format!("{files}{separator}{magnitude}");
    let width = usize::from(width);
    let separator_width = UnicodeWidthStr::width(separator);
    let comparison_width = width
        .saturating_sub(UnicodeWidthStr::width(facts.as_str()))
        .saturating_sub(separator_width);
    let comparison = truncate_end(&header.comparison, comparison_width);
    truncate_end(&format!("{comparison}{separator}{facts}"), width)
}

#[cfg(test)]
mod tests {
    use urushi::{Color, TextAttribute, TextStyle};

    use super::*;

    #[test]
    fn hunk_box_preserves_width_and_urushi_styles() {
        let line = Line::from(vec![
            Span::styled("画面", TextStyle::new().bold()),
            Span::raw("x".repeat(40)),
        ]);
        let boxed = hunk_box_content(line, 20, &TextStyle::new().foreground(Color::CYAN));

        assert_eq!(boxed.width(), 20);
        assert_eq!(
            boxed.spans.first().map(|span| span.content.as_str()),
            Some("│")
        );
        assert_eq!(
            boxed.spans.last().map(|span| span.content.as_str()),
            Some("│")
        );
        assert_eq!(
            boxed
                .spans
                .first()
                .and_then(|span| span.style.get_foreground()),
            Some(Color::CYAN)
        );
        assert!(
            boxed.spans[1]
                .style
                .get_attributes()
                .contains(TextAttribute::Bold)
        );
    }

    #[test]
    fn focused_hunk_uses_the_same_thick_border_weight_as_focused_panels() {
        let style = TextStyle::new().foreground(Color::CYAN);
        let mut lines = vec![
            hunk_box_top(Some("header"), 20, &TextStyle::new()),
            hunk_box_content(Line::raw("content"), 20, &TextStyle::new()),
            hunk_box_bottom(20, &TextStyle::new()),
        ];

        highlight_hunk_box(&mut lines, 0, 0, 3, style, true);

        assert!(lines[0].spans[0].content.starts_with('┏'));
        assert!(lines[0].spans[0].content.ends_with('┓'));
        assert_eq!(lines[1].spans.first().unwrap().content, "┃");
        assert_eq!(lines[1].spans.last().unwrap().content, "┃");
        assert!(lines[2].spans[0].content.starts_with('┗'));
        assert!(lines[2].spans[0].content.ends_with('┛'));
    }

    #[test]
    fn no_color_theme_keeps_semantic_attributes_without_colors() {
        let theme = SemanticTheme::no_color();

        assert_eq!(theme.selection().get_foreground(), None);
        assert!(
            theme
                .selection()
                .get_attributes()
                .contains(TextAttribute::Reversed)
        );
        assert_eq!(
            theme.diff_row_style(Tone::ChangeAdded).get_background(),
            None
        );
    }

    #[test]
    fn file_tree_styles_match_lazygits_element_roles() {
        let theme = SemanticTheme::from_no_color(None);

        assert_eq!(theme.file_status().get_foreground(), Some(Color::RED));
        assert_eq!(
            theme
                .file_icon(Color::Rgb(0xff, 0x70, 0x43))
                .get_foreground(),
            Some(Color::Rgb(0xff, 0x70, 0x43))
        );
        assert!(
            theme
                .file_selection()
                .get_attributes()
                .contains(TextAttribute::Bold)
        );
        assert!(
            !theme
                .file_selection()
                .get_attributes()
                .contains(TextAttribute::Reversed)
        );
        assert_eq!(theme.file_selection().get_foreground(), None);
    }
}
