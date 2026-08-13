use std::{cell::RefCell, collections::VecDeque, sync::Arc};

use crate::{
    presentation,
    semantic::{
        Body, DiffSearchTarget, FileAttention, Overlay, ReviewBody, ReviewWindowSection,
        RollupBody, StickyReviewContext, ThreadState, Tone, View,
    },
    symbols,
    syntax::SyntaxHighlighter,
    ui::{LayoutMode, ShellSize, truncate_end, truncate_start},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use unicode_width::UnicodeWidthStr;

const MINIMUM_WIDTH: u16 = 48;
const MINIMUM_HEIGHT: u16 = 8;
const HUNK_TEXT_CACHE_CAPACITY: usize = 96;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SemanticTheme {
    colors_enabled: bool,
}

fn sticky_context_text(context: &StickyReviewContext, width: u16) -> String {
    let file = format!(
        "{} {}/{} {}",
        symbols::FILE,
        context.file_index + 1,
        context.file_count,
        context.file
    );
    let value = match (context.hunk_index, context.hunk_header.as_deref()) {
        (Some(index), Some(header)) => format!(
            "{file}  •  hunk {}/{} {header}",
            index + 1,
            context.hunk_count
        ),
        (Some(index), None) => format!("{file}  •  hunk {}/{}", index + 1, context.hunk_count),
        (None, _) => file,
    };
    truncate_end(&value, usize::from(width))
}

#[cfg(test)]
fn logical_review_window(body: Text<'static>, top: usize, visible_rows: usize) -> Text<'static> {
    Text::from(
        body.lines
            .into_iter()
            .skip(top)
            .take(visible_rows)
            .collect::<Vec<_>>(),
    )
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

    pub(crate) fn style(self, tone: Tone) -> Style {
        let base = match tone {
            Tone::FocusSelection | Tone::Attention => Style::default().add_modifier(Modifier::BOLD),
            Tone::MutedResolved => Style::default().add_modifier(Modifier::DIM),
            Tone::ChangeAdded | Tone::ChangeRemoved => Style::default(),
        };
        if !self.colors_enabled {
            return base;
        }
        base.fg(match tone {
            Tone::ChangeAdded => Color::Green,
            Tone::ChangeRemoved => Color::Red,
            Tone::FocusSelection => Color::Cyan,
            Tone::Attention => Color::Yellow,
            Tone::MutedResolved => Color::Gray,
        })
    }

    pub(crate) fn selection(self) -> Style {
        self.style(Tone::FocusSelection)
            .add_modifier(Modifier::REVERSED)
    }

    pub(crate) fn border(self, tone: Tone) -> Style {
        self.style(tone)
            .remove_modifier(Modifier::BOLD | Modifier::DIM | Modifier::REVERSED)
    }

    fn modal_backdrop(self) -> Style {
        let style = Style::default().add_modifier(Modifier::DIM);
        if self.colors_enabled {
            style.fg(Color::DarkGray)
        } else {
            style
        }
    }

    /// Row kind owns a quiet background only.  Source-token foregrounds and
    /// modifiers are applied later by the presentation layer.
    pub(crate) fn diff_row_style(self, tone: Tone) -> Style {
        if !self.colors_enabled {
            return Style::default();
        }
        let background = match tone {
            Tone::ChangeAdded => Color::Rgb(20, 46, 32),
            Tone::ChangeRemoved => Color::Rgb(54, 27, 32),
            Tone::FocusSelection | Tone::Attention | Tone::MutedResolved => Color::Reset,
        };
        Style::default().bg(background)
    }

    pub(crate) fn colors_enabled(self) -> bool {
        self.colors_enabled
    }
}

#[derive(Debug, Clone, Copy)]
struct ShellAreas {
    header: Rect,
    navigation_rail: Option<Rect>,
    review_body: Rect,
    current_context: Rect,
    contextual_keys: Rect,
    size: ShellSize,
}

impl ShellAreas {
    fn resolve(area: Rect, show_rail: bool) -> Self {
        let [header, content, current_context, contextual_keys] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let size = ShellSize::for_width(area.width);
        let (navigation_rail, review_body) = match (show_rail, size.rail_width(content.width)) {
            (true, Some(rail_width)) => {
                let [rail, body] = Layout::horizontal([
                    Constraint::Length(rail_width),
                    Constraint::Min(MINIMUM_WIDTH),
                ])
                .areas(content);
                (Some(rail), body)
            }
            _ => (None, content),
        };
        Self {
            header,
            navigation_rail,
            review_body,
            current_context,
            contextual_keys,
            size,
        }
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
    anchor: crate::anchor::HunkLocation,
    header: Option<String>,
    coordinates: Option<crate::diff::HunkCoordinates>,
    lines: Arc<Vec<crate::diff::DiffLine>>,
    threads: Vec<crate::semantic::ThreadCard>,
    number_width: usize,
    layout: LayoutMode,
    wrap_lines: bool,
    header_match: bool,
    search: Option<(usize, String)>,
}

impl HunkTextKey {
    fn matches(&self, hunk: &crate::semantic::ReviewHunk, lookup: HunkTextLookup<'_>) -> bool {
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
    lines: Vec<Line<'static>>,
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
    pub fn render(&self, frame: &mut Frame, view: &View) {
        if frame.area().width < MINIMUM_WIDTH || frame.area().height < MINIMUM_HEIGHT {
            self.render_too_small(frame);
            return;
        }
        let areas = ShellAreas::resolve(frame.area(), view.file_rail.is_some());

        frame.render_widget(
            Paragraph::new(header_text(view, areas.size))
                .style(Style::default().add_modifier(Modifier::BOLD)),
            areas.header,
        );

        if let (Some(area), Some(rail)) = (areas.navigation_rail, &view.file_rail) {
            let inner_width = usize::from(area.width.saturating_sub(2));
            let items = rail
                .items
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    let marker = match file.attention {
                        FileAttention::NeedsAttention => symbols::NEEDS_ATTENTION,
                        FileAttention::Open => symbols::OPEN,
                        FileAttention::Resolved => symbols::RESOLVED,
                        FileAttention::None => " ",
                    };
                    let selection = if rail.selected == Some(index) {
                        symbols::NEXT
                    } else {
                        " "
                    };
                    let suffix = format!(" {}h/{}t", file.hunk_count, file.thread_count);
                    let path_width = inner_width
                        .saturating_sub(UnicodeWidthStr::width(marker))
                        .saturating_sub(UnicodeWidthStr::width(selection))
                        .saturating_sub(UnicodeWidthStr::width(suffix.as_str()))
                        .saturating_sub(2);
                    let path = truncate_start(&file.path, path_width);
                    let style = match file.attention {
                        FileAttention::NeedsAttention => self.semantic_theme.style(Tone::Attention),
                        FileAttention::Resolved => self.semantic_theme.style(Tone::MutedResolved),
                        FileAttention::Open | FileAttention::None => Style::default(),
                    };
                    ListItem::new(Line::styled(
                        format!("{selection} {marker} {path}{suffix}"),
                        style,
                    ))
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default();
            state.select(rail.selected);
            let block = region_block(
                "Files",
                false,
                rail.selected.map(|_| "CURRENT FILE"),
                self.semantic_theme,
            );
            frame.render_stateful_widget(
                List::new(items)
                    .block(block)
                    .highlight_style(self.semantic_theme.selection()),
                area,
                &mut state,
            );
        }

        let review_content = Rect::new(
            areas.review_body.x,
            areas.review_body.y,
            areas.review_body.width.saturating_sub(1),
            areas.review_body.height,
        );
        let body_inner_width = match view.body {
            Body::Review(_) => review_content.width,
            Body::Rollup(_) => areas.review_body.width.saturating_sub(2),
        };
        let (body, scroll) = match &view.body {
            Body::Review(review) => (
                self.review_window(review, review.viewport.presentation_width, view),
                0,
            ),
            Body::Rollup(rollup) => {
                let body = rollup_text(rollup, body_inner_width, self.semantic_theme);
                let viewport_height = usize::from(areas.review_body.height.saturating_sub(2));
                let max_scroll = body
                    .height()
                    .saturating_sub(viewport_height)
                    .min(usize::from(u16::MAX)) as u16;
                (body, rollup.scroll.min(max_scroll))
            }
        };
        let inner = match view.body {
            Body::Review(_) => review_content,
            Body::Rollup(_) => {
                frame.render_widget(
                    region_block(
                        "Thread rollup",
                        view.overlay.is_none(),
                        None,
                        self.semantic_theme,
                    ),
                    areas.review_body,
                );
                Rect::new(
                    areas.review_body.x.saturating_add(1),
                    areas.review_body.y.saturating_add(1),
                    areas.review_body.width.saturating_sub(2),
                    areas.review_body.height.saturating_sub(2),
                )
            }
        };
        let content = if let Body::Review(review) = &view.body {
            if let Some(context) = &review.viewport.sticky_context {
                let [sticky, content] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
                frame.render_widget(
                    Paragraph::new(sticky_context_text(context, body_inner_width))
                        .style(self.semantic_theme.style(Tone::FocusSelection)),
                    sticky,
                );
                content
            } else {
                inner
            }
        } else {
            inner
        };
        frame.render_widget(Paragraph::new(body).scroll((scroll, 0)), content);
        if let Body::Review(review) = &view.body
            && review.viewport.total_rows > review.viewport.visible_rows
        {
            let scroll_positions = review
                .viewport
                .total_rows
                .saturating_sub(review.viewport.visible_rows)
                .saturating_add(1);
            let mut state = ScrollbarState::new(scroll_positions)
                .position(review.scroll)
                .viewport_content_length(review.viewport.visible_rows);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(None)
                    .thumb_symbol(symbols::SCROLL_THUMB),
                areas.review_body,
                &mut state,
            );
        }

        frame.render_widget(
            Paragraph::new(view.footer.current_context.text.as_str())
                .style(self.semantic_theme.style(Tone::FocusSelection)),
            areas.current_context,
        );
        frame.render_widget(
            Paragraph::new(format!("Keys: {}", view.footer.contextual_keys.text))
                .style(self.semantic_theme.style(Tone::MutedResolved)),
            areas.contextual_keys,
        );
        if let Some(overlay) = &view.overlay {
            if matches!(overlay, Overlay::Composer(_)) {
                let area = frame.area();
                frame
                    .buffer_mut()
                    .set_style(area, self.semantic_theme.modal_backdrop());
            }
            self.render_overlay(frame, overlay);
        }
    }

    fn review_window(
        &self,
        review: &ReviewBody,
        available_width: u16,
        view: &View,
    ) -> Text<'static> {
        if let Some(message) = &review.empty_state {
            return Text::from(
                message
                    .lines()
                    .skip(review.scroll)
                    .take(review.viewport.visible_rows)
                    .map(|line| Line::raw(line.to_owned()))
                    .collect::<Vec<_>>(),
            );
        }

        let window_start = review.scroll;
        let window_end = window_start.saturating_add(review.viewport.visible_rows);
        let mut lines = Vec::new();
        for section in review.viewport.sections.iter().copied() {
            let mut section_lines = match section {
                ReviewWindowSection::FileHeader { file_index, .. } => {
                    self.file_header_lines(&review.files[file_index], review, available_width)
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
                        highlight_hunk_box(
                            &mut rendered,
                            section.start(),
                            section.start(),
                            section.end(),
                            self.semantic_theme.border(Tone::FocusSelection),
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
        Text::from(lines)
    }

    fn file_header_lines(
        &self,
        file: &crate::semantic::ReviewFile,
        review: &ReviewBody,
        available_width: u16,
    ) -> Vec<Line<'static>> {
        let path_width = usize::from(available_width).saturating_sub(6);
        let path_match = matches!(
            review.search_target.as_ref(),
            Some(DiffSearchTarget::FilePath { path }) if path == &file.path
        );
        let mut lines = vec![
            Line::raw(""),
            Line::styled(
                format!(
                    "{} {}",
                    if path_match {
                        symbols::SEARCH
                    } else {
                        symbols::FILE
                    },
                    truncate_start(&file.path, path_width)
                ),
                if path_match {
                    self.semantic_theme.selection()
                } else {
                    self.semantic_theme.style(Tone::Attention)
                },
            ),
        ];
        lines.extend(
            file.metadata
                .iter()
                .filter(|line| presentation::show_file_metadata(line))
                .map(|line| {
                    Line::styled(
                        truncate_end(&format!("· {line}"), usize::from(available_width)),
                        self.semantic_theme.style(Tone::MutedResolved),
                    )
                }),
        );
        lines
    }

    #[allow(clippy::too_many_arguments)]
    fn cached_hunk_lines(
        &self,
        file: &crate::semantic::ReviewFile,
        hunk: &crate::semantic::ReviewHunk,
        review: &ReviewBody,
        available_width: u16,
        number_width: usize,
        view: &View,
    ) -> Vec<Line<'static>> {
        let content_width = available_width.saturating_sub(2);
        let layout = view.layout.diff_layout.resolved(content_width);
        let header_match = matches!(
            review.search_target.as_ref(),
            Some(DiffSearchTarget::HunkHeader { location }) if location == &hunk.anchor
        );
        let search = match (&review.search_target, review.search_query.as_deref()) {
            (
                Some(DiffSearchTarget::DiffLine {
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
            border_style,
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
                .map(|line| hunk_box_content(line, hunk_width, border_style)),
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
                border_style,
            ));
        }
        lines.push(hunk_box_bottom(hunk_width, border_style));

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

    fn render_too_small(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal is too small for revia ({0}×{1}).\nResize to at least 48×8, then continue.\nPress q to quit.",
                area.width, area.height
            ))
            .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_overlay(&self, frame: &mut Frame, overlay: &Overlay) {
        match overlay {
            Overlay::Composer(composer) => {
                let area = centered_rect(70, composer.height, frame.area());
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(self.semantic_theme.border(Tone::FocusSelection))
                        .title(format!(" {} ", composer.context)),
                    area,
                );
                let inner = Rect::new(
                    area.x.saturating_add(1),
                    area.y.saturating_add(1),
                    area.width.saturating_sub(2),
                    area.height.saturating_sub(2),
                );
                let [editor, feedback] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
                frame.render_widget(
                    Paragraph::new(Text::from(
                        composer
                            .lines
                            .iter()
                            .cloned()
                            .map(Line::raw)
                            .collect::<Vec<_>>(),
                    ))
                    .scroll((u16::try_from(composer.scroll).unwrap_or(u16::MAX), 0)),
                    editor,
                );
                frame.render_widget(
                    Paragraph::new(
                        composer
                            .message
                            .as_deref()
                            .unwrap_or(&composer.instructions),
                    )
                    .style(self.semantic_theme.style(Tone::MutedResolved)),
                    feedback,
                );
                if composer.cursor_row >= composer.scroll {
                    let cursor_row = composer.cursor_row - composer.scroll;
                    if cursor_row < usize::from(editor.height) {
                        let cursor_x = u16::try_from(composer.cursor_column)
                            .unwrap_or(u16::MAX)
                            .min(editor.width.saturating_sub(1));
                        let cursor_y = u16::try_from(cursor_row)
                            .unwrap_or(u16::MAX)
                            .min(editor.height.saturating_sub(1));
                        frame.set_cursor_position((
                            editor.x.saturating_add(cursor_x),
                            editor.y.saturating_add(cursor_y),
                        ));
                    }
                }
            }
            Overlay::Help(help) => {
                let area = centered_rect(86, 20, frame.area());
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Double)
                        .border_style(self.semantic_theme.style(Tone::FocusSelection))
                        .title("Keyboard help — Esc/? to close"),
                    area,
                );
                let inner = Rect::new(
                    area.x.saturating_add(1),
                    area.y.saturating_add(1),
                    area.width.saturating_sub(2),
                    area.height.saturating_sub(2),
                );
                let [content, hint] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
                frame.render_widget(
                    Paragraph::new(Text::from(
                        help.lines
                            .iter()
                            .cloned()
                            .map(Line::raw)
                            .collect::<Vec<_>>(),
                    ))
                    .scroll((u16::try_from(help.scroll).unwrap_or(u16::MAX), 0)),
                    content,
                );
                frame.render_widget(
                    Paragraph::new(truncate_end(&help.position_hint, usize::from(hint.width)))
                        .style(self.semantic_theme.style(Tone::MutedResolved)),
                    hint,
                );
            }
            Overlay::Thread(thread) => {
                let area = centered_rect(78, 18, frame.area());
                frame.render_widget(Clear, area);
                let state = match thread.state {
                    ThreadState::NeedsAttention => "NEEDS ATTENTION",
                    ThreadState::Open => "OPEN",
                    ThreadState::Resolved => "RESOLVED",
                };
                let outdated = if thread.outdated { " · OUTDATED" } else { "" };
                frame.render_widget(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(self.semantic_theme.border(Tone::FocusSelection))
                        .title(format!(
                            " Conversation #{} · {state}{outdated} · {} ",
                            thread.id, thread.position
                        )),
                    area,
                );
                let inner = Rect::new(
                    area.x.saturating_add(1),
                    area.y.saturating_add(1),
                    area.width.saturating_sub(2),
                    area.height.saturating_sub(2),
                );
                let [context, messages, actions] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .areas(inner);
                frame.render_widget(
                    Paragraph::new(truncate_end(&thread.context, usize::from(context.width)))
                        .style(self.semantic_theme.style(Tone::MutedResolved)),
                    context,
                );
                let mut lines = Vec::new();
                for (index, message) in thread.messages.iter().enumerate() {
                    if index > 0 {
                        lines.push(Line::raw(""));
                    }
                    lines.push(Line::styled(
                        message.author.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    lines.extend(message.body.lines().map(|line| Line::raw(line.to_owned())));
                }
                frame.render_widget(
                    Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
                    messages,
                );
                frame.render_widget(
                    Paragraph::new(if thread.state == ThreadState::Resolved {
                        if actions.width >= 48 {
                            "t/T switch · c reply · R reopen · Esc close"
                        } else {
                            "t/T · c reply · R reopen · Esc"
                        }
                    } else if actions.width >= 48 {
                        "t/T switch · c reply · x resolve · Esc close"
                    } else {
                        "t/T · c reply · x resolve · Esc"
                    })
                    .style(self.semantic_theme.style(Tone::MutedResolved)),
                    actions,
                );
            }
        }
    }
}

fn highlight_hunk_box(
    lines: &mut [Line<'static>],
    window_start: usize,
    range_start: usize,
    range_end: usize,
    style: Style,
) {
    for (offset, line) in lines.iter_mut().enumerate() {
        let row = window_start.saturating_add(offset);
        if row < range_start || row >= range_end {
            continue;
        }
        if row == range_start || row + 1 == range_end {
            for span in &mut line.spans {
                span.style = span.style.patch(style);
            }
        } else {
            if let Some(border) = line.spans.first_mut() {
                border.style = border.style.patch(style);
            }
            if let Some(border) = line.spans.last_mut() {
                border.style = border.style.patch(style);
            }
        }
    }
}

fn hunk_box_top(title: Option<&str>, width: usize, style: Style) -> Line<'static> {
    if width < 2 {
        return Line::styled("─".repeat(width), style);
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
    Line::styled(format!("{prefix}{}╮", "─".repeat(fill)), style)
}

fn hunk_box_content(mut line: Line<'static>, width: usize, style: Style) -> Line<'static> {
    if width < 2 {
        return line;
    }
    let inner_width = width.saturating_sub(2);
    line = fit_line(line, inner_width);
    let padding = inner_width.saturating_sub(line.width());
    line.spans.insert(0, Span::styled("│", style));
    line.spans.push(Span::raw(" ".repeat(padding)));
    line.spans.push(Span::styled("│", style));
    line
}

fn fit_line(line: Line<'static>, width: usize) -> Line<'static> {
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
                truncate_end(span.content.as_ref(), remaining),
                span.style,
            ));
            break;
        }
    }
    Line::from(spans)
}

fn hunk_box_bottom(width: usize, style: Style) -> Line<'static> {
    if width < 2 {
        return Line::styled("─".repeat(width), style);
    }
    Line::styled(format!("╰{}╯", "─".repeat(width - 2)), style)
}

fn rollup_text(rollup: &RollupBody, available_width: u16, theme: SemanticTheme) -> Text<'static> {
    let mut lines = vec![Line::styled(
        rollup.summary.clone(),
        Style::default().add_modifier(Modifier::BOLD),
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
                Style::default()
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
    Text::from(lines)
}

fn header_text(view: &View, size: ShellSize) -> String {
    match size {
        ShellSize::Wide => format!(
            "revia  •  Filter: {}  •  {} files  •  {} need you  •  {} open  •  {} resolved",
            view.header.active_filter,
            view.header.file_count,
            view.header.needs_attention,
            view.header.open,
            view.header.resolved
        ),
        ShellSize::Medium => format!(
            "revia  •  Filter: {}  •  {} files  •  {} need you  •  {} open",
            view.header.active_filter,
            view.header.file_count,
            view.header.needs_attention,
            view.header.open
        ),
        ShellSize::Narrow => format!(
            "revia • F:{} • {} files",
            view.header.active_filter, view.header.file_count
        ),
    }
}

fn region_block<'a>(
    title: &'a str,
    focused: bool,
    focus_label: Option<&'a str>,
    theme: SemanticTheme,
) -> Block<'a> {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(match focus_label {
            Some(label) => format!(" {title}  {label} "),
            None => format!(" {title} "),
        });
    if focused {
        block.border_style(theme.border(Tone::FocusSelection))
    } else {
        block.border_style(theme.border(Tone::MutedResolved))
    }
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let width = area
        .width
        .saturating_mul(width_percent)
        .saturating_div(100)
        .max(24)
        .min(area.width);
    let height = height.min(area.height.saturating_sub(2)).max(3);
    Rect {
        x: area.x.saturating_add(area.width.saturating_sub(width) / 2),
        y: area
            .y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{
        Terminal,
        backend::TestBackend,
        buffer::Buffer,
        layout::Rect,
        style::{Color, Modifier, Style},
        text::{Line, Span},
    };

    use crate::{
        anchor::{Anchor, HunkLocation},
        app::Model,
        diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff},
        mode::{help, review},
        semantic::Body,
        symbols,
        thread::{Participant, ParticipantKind, ThreadState},
        ui::LayoutMode,
    };

    use super::{Renderer, SemanticTheme, ShellAreas, logical_review_window};
    use crate::ui::truncate_start;

    const RESPONSIVE_DIFF: &str = "diff --git a/src/components/review/navigation.rs b/src/components/review/navigation.rs\n--- a/src/components/review/navigation.rs\n+++ b/src/components/review/navigation.rs\n@@ -1 +1 @@\n-old_navigation\n+new_navigation\ndiff --git a/src/画面/とても長いレビュー項目.rs b/src/画面/とても長いレビュー項目.rs\n--- a/src/画面/とても長いレビュー項目.rs\n+++ b/src/画面/とても長いレビュー項目.rs\n@@ -1 +1 @@\n-old_wide\n+new_wide\n";
    const READABLE_DIFF: &str = "diff --git a/src/readable.rs b/src/readable.rs\nindex 111..222 100644\n--- a/src/readable.rs\n+++ b/src/readable.rs\n@@ -8,3 +18,3 @@ fn first()\n context\n-old_ascii_line_that_is_far_too_long_for_the_available_terminal_region\n+new\twide_画面_👨‍👩‍👧‍👦_that_is_also_far_too_long_for_the_available_terminal_region\n tail\n@@ -100,2 +200,3 @@ fn second()\n next\n+inserted\n last\n";
    const HIDDEN_SELECTION_DIFF: &str = "diff --git a/selection.rs b/selection.rs\n--- a/selection.rs\n+++ b/selection.rs\n@@ -8,3 +18,3 @@\n before\n-old_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n+new\n after\n";

    fn model_with_diff(raw: &str) -> Model {
        Model::new(
            DiffRequest {
                target: DiffTarget::WorkingTree,
                context_lines: 3,
            },
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
            ThreadState::default(),
        )
    }

    fn render(renderer: &Renderer, model: &mut Model, width: u16, height: u16) -> Buffer {
        crate::app::update(
            model,
            crate::app::global::Event::ViewportResized {
                rows: height.saturating_sub(5),
                columns: width,
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let semantic = crate::app::view(model);
        terminal
            .draw(|frame| renderer.render(frame, &semantic))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &Buffer) -> Vec<String> {
        buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    #[test]
    fn semantic_view_projects_to_existing_split_and_stack_contract() {
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old_a\n+new_a\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old_b\n+new_b\n";
        let mut threads = ThreadState::default();
        let human = Participant {
            id: "human".into(),
            kind: ParticipantKind::Human,
        };
        let attention = threads.post(
            Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@")),
            human.clone(),
            "Needs a human decision.".into(),
            1,
        );
        threads.set_needs_attention(attention, true).unwrap();
        let resolved = threads.post(
            Anchor::new("deadbeef", HunkLocation::new("b.rs", "@@ -1 +1 @@")),
            human.clone(),
            "Already addressed.".into(),
            2,
        );
        threads.close(resolved, &human).unwrap();
        let mut model = Model::new(
            DiffRequest {
                target: DiffTarget::WorkingTree,
                context_lines: 3,
            },
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
            threads,
        );
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };

        let split = render(&renderer, &mut model, 120, 48)
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(split.contains("1 need you"));
        let semantic = crate::app::view(&model);
        let Body::Review(review) = semantic.body else {
            panic!("review body")
        };
        assert!(!review.files[0].hunks[0].threads.is_empty());
        assert!(split.contains(" │ "));

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        let stack = render(&renderer, &mut model, 80, 32)
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(stack.contains("-old_a"));
        assert!(stack.contains("+new_a"));
    }

    #[test]
    fn lifecycle_cards_remain_legible_at_wide_narrow_and_no_color_sizes() {
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let human = Participant {
            id: "reviewer".into(),
            kind: ParticipantKind::Human,
        };
        let location = HunkLocation::new("a.rs", "@@ -1 +1 @@");
        let mut threads = ThreadState::default();
        threads.post(
            Anchor::new("deadbeef", location.clone()),
            human.clone(),
            "open context".into(),
            1,
        );
        let resolved = threads.post(
            Anchor::new("deadbeef", location.clone()),
            human.clone(),
            "resolved context".into(),
            2,
        );
        threads.close(resolved, &human).unwrap();
        let attention = threads.post(
            Anchor::new("deadbeef", location),
            human,
            "reviewer: 長い画面メッセージ👨‍👩‍👧‍👦 that wraps across a narrow card without corrupting the next row".into(),
            3,
        );
        threads.set_needs_attention(attention, true).unwrap();
        threads.set_outdated(attention, true).unwrap();
        let mut model = Model::new(
            DiffRequest {
                target: DiffTarget::WorkingTree,
                context_lines: 3,
            },
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
            threads,
        );
        crate::app::update(&mut model, review::Event::MoveThread(1));
        crate::app::update(&mut model, review::Event::MoveThread(1));
        crate::app::update(&mut model, review::Event::MoveThread(1));
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };

        let wide = render(&renderer, &mut model, 120, 40);
        let wide_text = rows(&wide).join("\n");
        assert!(wide_text.contains("Conversation #2 · NEEDS ATTENTION · OUTDATED · 3/3"));
        assert!(wide_text.contains("reviewer:"));
        assert!(wide_text.contains("t/T switch · c reply · x resolve · Esc close"));
        assert!(wide.content().iter().all(|cell| cell.fg == Color::Reset));

        let narrow = render(&renderer, &mut model, 48, 40);
        let narrow_rows = rows(&narrow);
        let narrow_text = narrow_rows.join("\n");
        assert!(narrow_text.contains("Conversation #2"));
        assert!(narrow_text.contains("NEEDS ATTENTION"));
        let semantic = crate::app::view(&model);
        let Some(crate::semantic::Overlay::Thread(thread)) = semantic.overlay else {
            panic!("thread overlay")
        };
        assert!(thread.outdated);
        assert!(narrow.content().iter().all(|cell| cell.fg == Color::Reset));

        crate::app::update(&mut model, review::Event::MoveThread(-1));
        let resolved = render(&renderer, &mut model, 48, 40);
        let resolved_text = rows(&resolved).join("\n");
        for action in ["c reply", "R reopen", "Esc"] {
            assert!(
                resolved_text.contains(action),
                "missing {action}: {resolved_text}"
            );
        }
    }

    #[test]
    fn renders_a_stable_explanation_below_the_minimum_layout_size() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };
        let model = Model::new(
            DiffRequest {
                target: DiffTarget::WorkingTree,
                context_lines: 3,
            },
            LoadedDiff {
                text: String::new(),
                document: DiffDocument::default(),
            },
            ThreadState::default(),
        );
        let semantic = crate::app::view(&model);

        for (width, height) in [(1, 1), (12, 7), (47, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| renderer.render(frame, &semantic))
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Terminal") || width < 8);
        }
    }

    #[test]
    fn renders_empty_states_with_the_selected_diff_target() {
        let renderer = Renderer::default();
        for (target, expected) in [
            (DiffTarget::WorkingTree, "working tree"),
            (DiffTarget::Staged, "staged changes"),
            (DiffTarget::Commit("abc123".into()), "commit abc123"),
            (DiffTarget::Range("main...HEAD".into()), "range main...HEAD"),
        ] {
            let model = Model::new(
                DiffRequest {
                    target,
                    context_lines: 3,
                },
                LoadedDiff {
                    text: String::new(),
                    document: DiffDocument::default(),
                },
                ThreadState::default(),
            );
            let semantic = crate::app::view(&model);
            let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
            terminal
                .draw(|frame| renderer.render(frame, &semantic))
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("No changes found."));
            assert!(rendered.contains(expected));
        }
    }

    #[test]
    fn filtered_empty_state_names_the_filter_and_recovery_without_color_at_narrow_width() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::CycleFilter(1));

        let buffer = render(&renderer, &mut model, 48, 20);
        let rendered = rows(&buffer).join("\n");
        assert!(rendered.contains("F:Needs attention"));
        assert!(rendered.contains("No review targets match Filter:"));
        assert!(rendered.contains("Git diff is still loaded"));
        assert!(rendered.contains("Press A for All changes"));
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );

        let minimum = render(&renderer, &mut model, 48, 8);
        let minimum_rows = rows(&minimum).join("\n");
        assert!(minimum_rows.contains("No review targets match Filter:"));
        assert!(minimum_rows.contains("Git diff is still loaded"));
        assert!(minimum_rows.contains("Press A for All changes"));
        assert!((1..6).all(|y| minimum[(47, y)].symbol() != symbols::SCROLL_THUMB));

        let semantic = crate::app::view(&model);
        let Body::Review(review) = semantic.body else {
            panic!("review body")
        };
        assert_eq!(review.viewport.total_rows, 3);
        assert_eq!(review.viewport.visible_rows, 3);
        assert_eq!(review.viewport.sticky_context, None);
    }

    #[test]
    fn help_scroll_reaches_the_exit_section_at_supported_viewports() {
        let renderer = Renderer::default();
        for (width, height) in [(120, 24), (48, 20), (48, 8)] {
            let mut model = model_with_diff(RESPONSIVE_DIFF);
            crate::app::update(&mut model, crate::app::global::Event::OpenHelp);

            let top = rows(&render(&renderer, &mut model, width, height)).join("\n");
            assert!(top.contains("Keyboard help — Esc/? to close"), "{top}");
            assert!(top.contains("Navigation"), "{top}");
            assert!(top.contains("rows "), "{top}");

            crate::app::update(&mut model, help::Event::JumpToEdge { end: true });
            let bottom = rows(&render(&renderer, &mut model, width, height)).join("\n");
            assert!(
                bottom.contains("Esc/? closes"),
                "{width}x{height}: {bottom}"
            );
            assert!(bottom.contains("rows "), "{width}x{height}: {bottom}");
        }
    }

    #[test]
    fn wide_medium_narrow_and_minimum_shells_have_intentional_roles() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);

        let wide = rows(&render(&renderer, &mut model, 120, 24));
        assert!(wide[0].contains("resolved"));
        assert!(wide.iter().any(|row| row.contains("Files")));
        assert!(wide.iter().any(|row| row.contains("Diff")));
        assert!(wide[22].contains("Context: Diff"));
        assert!(wide[23].starts_with("Keys:"));
        let wide_areas = ShellAreas::resolve(Rect::new(0, 0, 120, 24), true);
        assert_eq!(wide_areas.header, Rect::new(0, 0, 120, 1));
        assert_eq!(wide_areas.navigation_rail, Some(Rect::new(0, 1, 30, 21)));
        assert_eq!(wide_areas.review_body, Rect::new(30, 1, 90, 21));
        assert_eq!(wide_areas.current_context, Rect::new(0, 22, 120, 1));
        assert_eq!(wide_areas.contextual_keys, Rect::new(0, 23, 120, 1));

        let medium = rows(&render(&renderer, &mut model, 88, 20));
        assert!(medium[0].contains("open"));
        assert!(!medium[0].contains("resolved"));
        assert!(medium.iter().any(|row| row.contains("Files")));
        assert!(medium.iter().any(|row| row.contains("-old_navigation")));
        let medium_areas = ShellAreas::resolve(Rect::new(0, 0, 88, 20), true);
        assert_eq!(medium_areas.navigation_rail, Some(Rect::new(0, 1, 28, 17)));
        assert_eq!(medium_areas.review_body, Rect::new(28, 1, 60, 17));
        assert_eq!(medium_areas.current_context.y, 18);
        assert_eq!(medium_areas.contextual_keys.y, 19);

        let narrow = rows(&render(&renderer, &mut model, 64, 16));
        assert_eq!(narrow[0].trim_end(), "revia • F:All changes • 2 files");
        assert!(!narrow.iter().any(|row| row.contains(" Files ")));
        assert!(
            narrow
                .iter()
                .any(|row| row.contains("review/navigation.rs"))
        );
        assert!(narrow[14].contains("Review"), "{}", narrow[14]);
        let narrow_areas = ShellAreas::resolve(Rect::new(0, 0, 64, 16), true);
        assert_eq!(narrow_areas.navigation_rail, None);
        assert_eq!(narrow_areas.review_body, Rect::new(0, 1, 64, 13));
        assert_eq!(narrow_areas.current_context.y, 14);
        assert_eq!(narrow_areas.contextual_keys.y, 15);

        let minimum = rows(&render(&renderer, &mut model, 48, 8));
        assert!(minimum.iter().any(|row| row.contains("@@ -1 +1 @@")));
        assert!(minimum.iter().any(|row| row.contains("navigation.rs")));
        assert!(minimum[6].contains("Review"), "{}", minimum[6]);
        assert!(minimum[7].starts_with("Keys:"));
        let minimum_areas = ShellAreas::resolve(Rect::new(0, 0, 48, 8), true);
        assert_eq!(minimum_areas.navigation_rail, None);
        assert_eq!(minimum_areas.review_body, Rect::new(0, 1, 48, 5));
        assert_eq!(minimum_areas.current_context.y, 6);
        assert_eq!(minimum_areas.contextual_keys.y, 7);
    }

    #[test]
    fn ascii_and_wide_paths_are_truncated_without_crossing_region_borders() {
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(
                truncate_start("src/画面/とても長いレビュー項目.rs", 16).as_str()
            ),
            16
        );
        assert!(truncate_start("a/very/long/path/to/file.rs", 14).ends_with("file.rs"));
        assert!(unicode_width::UnicodeWidthStr::width(truncate_start("long/✈️", 2).as_str()) <= 2);
        assert_eq!(truncate_start("long/画\u{301}", 2), "…");

        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        let buffer = render(&renderer, &mut model, 120, 24);
        for y in 2..21 {
            assert_eq!(buffer[(29, y)].symbol(), "│");
            assert_ne!(buffer[(30, y)].symbol(), "║");
        }
    }

    #[test]
    fn hunk_content_keeps_both_borders_at_a_fixed_width() {
        let line = Line::from(vec![
            Span::styled("画面", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("x".repeat(40)),
        ]);
        let boxed = super::hunk_box_content(line, 20, Style::default());

        assert_eq!(boxed.width(), 20);
        assert_eq!(boxed.spans.first().unwrap().content, "│");
        assert_eq!(boxed.spans.last().unwrap().content, "│");
    }

    #[test]
    fn no_color_retains_change_state_selection_and_focus_in_text_and_shape() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        let buffer = render(&renderer, &mut model, 120, 24);
        let rendered = rows(&buffer).join("\n");

        assert!(rendered.contains(""));
        assert!(rendered.contains("╭─ @@ -1 +1 @@"));
        assert!(rendered.contains("Diff"));
        assert!(rendered.contains("-old_navigation"));
        assert!(rendered.contains("+new_navigation"));
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
    }

    #[test]
    fn selected_semantic_search_match_has_a_non_color_marker() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(READABLE_DIFF);
        for event in [
            review::Event::BeginSearch,
            review::Event::InsertSearchCharacter('i'),
            review::Event::InsertSearchCharacter('n'),
            review::Event::InsertSearchCharacter('s'),
            review::Event::InsertSearchCharacter('e'),
            review::Event::InsertSearchCharacter('r'),
            review::Event::InsertSearchCharacter('t'),
            review::Event::InsertSearchCharacter('e'),
            review::Event::InsertSearchCharacter('d'),
        ] {
            crate::app::update(&mut model, event);
        }

        let rendered = rows(&render(&renderer, &mut model, 80, 24)).join("\n");
        assert!(rendered.contains(""));
        assert!(rendered.contains("Context: Search input"));
        assert!(rendered.contains("Esc cancel"));
    }

    #[test]
    fn case_insensitive_search_reverses_the_actual_source_graphemes() {
        let raw =
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+INSERTED\n";
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(raw);
        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Split));
        crate::app::update(&mut model, review::Event::BeginSearch);
        for character in "inserted".chars() {
            crate::app::update(&mut model, review::Event::InsertSearchCharacter(character));
        }

        let buffer = render(&renderer, &mut model, 80, 24);
        let source_row = rows(&buffer)
            .into_iter()
            .find(|row| row.contains("+INSERTED"))
            .expect("selected source row is rendered");
        assert!(source_row.contains("  "), "{source_row:?}");
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| { cell.symbol() == "I" && cell.modifier.contains(Modifier::REVERSED) })
        );
    }

    #[test]
    fn renderer_keeps_selected_split_and_stack_change_evidence_composed() {
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n let stable = 1;\n-let timeout = 30;\n+let timeout = 60;\n@@ -10,2 +10,2 @@\n fn stable_two() {}\n-let retries = 2;\n+let retries = 3;\n";
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };
        let mut model = model_with_diff(raw);
        crate::app::update(&mut model, review::Event::BeginSearch);
        for character in "60".chars() {
            crate::app::update(&mut model, review::Event::InsertSearchCharacter(character));
        }

        for layout in [LayoutMode::Split, LayoutMode::Stack] {
            crate::app::update(&mut model, review::Event::SetLayout(layout));
            let buffer = render(&renderer, &mut model, 80, 32);
            let source_row = rows(&buffer)
                .into_iter()
                .find(|row| row.contains("60"))
                .expect("selected replacement source row is rendered");
            assert!(source_row.contains("  "), "{layout:?}: {source_row:?}");
            let changed = buffer
                .content()
                .iter()
                .find(|cell| cell.symbol() == "6" && cell.modifier.contains(Modifier::REVERSED))
                .expect("searched changed grapheme is reverse-marked");
            assert!(changed.modifier.contains(Modifier::UNDERLINED));

            let rendered_rows = rows(&buffer);
            for source in ["stable = 1", "timeout = 30", "timeout = 60", "stable_two"] {
                let y = rendered_rows
                    .iter()
                    .position(|row| row.contains(source))
                    .unwrap_or_else(|| panic!("{layout:?}: missing source row {source:?}"));
                assert!(
                    (0..buffer.area.width).any(|x| {
                        let cell = &buffer[(x, y as u16)];
                        cell.fg != Color::Reset && !cell.symbol().trim().is_empty()
                    }),
                    "{layout:?}: {source:?} lost lexical foreground"
                );
            }

            let selected_box = rendered_rows
                .iter()
                .position(|row| row.contains("@@ -1,2 +1,2 @@"))
                .expect("selected hunk box is visible");
            let unselected_box = rendered_rows
                .iter()
                .position(|row| row.contains("@@ -10,2 +10,2 @@"))
                .expect("unselected hunk box is visible");
            assert!(
                (0..buffer.area.width).any(|x| buffer[(x, selected_box as u16)].fg == Color::Cyan)
            );
            assert!(
                (0..buffer.area.width)
                    .all(|x| buffer[(x, unselected_box as u16)].fg != Color::Cyan)
            );
        }
    }

    #[test]
    fn hidden_hunk_header_search_marks_the_resolved_content_row() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(READABLE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleHunkHeaders);
        crate::app::update(&mut model, review::Event::BeginSearch);
        for character in "second()".chars() {
            crate::app::update(&mut model, review::Event::InsertSearchCharacter(character));
        }

        let rendered = rows(&render(&renderer, &mut model, 80, 24)).join("\n");
        assert!(!rendered.contains("@@ -100,2 +200,3 @@ fn second()"));
        assert!(rendered.contains(""));
        assert!(rendered.contains("next"));
    }

    #[test]
    fn split_search_marker_identifies_the_selected_change_side() {
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-needle old\n+needle new\n";
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(raw);
        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Split));
        crate::app::update(&mut model, review::Event::BeginSearch);
        for character in "needle".chars() {
            crate::app::update(&mut model, review::Event::InsertSearchCharacter(character));
        }

        let removed = rows(&render(&renderer, &mut model, 120, 16)).join("\n");
        assert!(removed.contains(""));
        assert!(removed.contains("-needle old"));
        crate::app::update(&mut model, review::Event::FinishSearch);
        crate::app::update(&mut model, review::Event::MoveSearch(1));
        let added = rows(&render(&renderer, &mut model, 120, 16)).join("\n");
        assert!(added.contains(""));
        assert!(added.contains("+needle new"));
    }

    #[test]
    fn hunk_box_survives_hidden_hunk_headers_without_color() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleHunkHeaders);
        let buffer = render(&renderer, &mut model, 64, 16);
        let rendered = rows(&buffer).join("\n");

        assert!(!rendered.contains("▶"));
        assert!(rows(&buffer).iter().any(|row| row.contains("╭─")));
        assert!(rows(&buffer).iter().any(|row| row.contains("╰─")));
    }

    #[test]
    fn hidden_headers_keep_every_selected_source_row_visible_without_color() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(HIDDEN_SELECTION_DIFF);
        crate::app::update(&mut model, review::Event::ToggleSidebar);
        crate::app::update(&mut model, review::Event::ToggleHunkHeaders);

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Split));
        let split = render(&renderer, &mut model, 120, 24);
        assert_selected_source_block(&split, &["before", "-old_", "after"]);

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        crate::app::update(&mut model, review::Event::ToggleWrap);
        let stack = render(&renderer, &mut model, 64, 24);
        assert_selected_source_block(&stack, &["before", "-old_", "↪", "+new", "after"]);
    }

    fn assert_selected_source_block(buffer: &Buffer, expected_rows: &[&str]) {
        let rendered = rows(buffer);
        assert!(rendered.iter().all(|row| !row.contains("@@")));
        for expected in expected_rows {
            let (y, row) = rendered
                .iter()
                .enumerate()
                .find(|(_, row)| row.contains(expected))
                .expect("selected source row is visible");
            assert!(
                row.contains('│'),
                "source row is outside its hunk box: {row:?}"
            );
            assert!(
                (1..buffer.area.width.saturating_sub(1))
                    .all(|x| { !buffer[(x, y as u16)].modifier.contains(Modifier::REVERSED) }),
                "hunk membership must not reverse-paint source row {y}"
            );
        }
    }

    #[test]
    fn wide_and_narrow_diff_rows_preserve_number_gutters_and_region_bounds() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(READABLE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleSidebar);

        let wide_buffer = render(&renderer, &mut model, 120, 32);
        let wide = rows(&wide_buffer);
        let replacement = wide
            .iter()
            .find(|row| row.contains("old_ascii_line"))
            .expect("wide fixture renders its replacement row");
        assert!(replacement.contains("old_ascii_line"));
        assert!(replacement.contains("new wide_"), "{replacement:?}");
        assert!(!replacement.contains('≈'));
        let split_at = replacement.find(" │ ").expect("split separator is visible");
        let split_column = unicode_width::UnicodeWidthStr::width(&replacement[..split_at]);
        let context = wide
            .iter()
            .find(|row| row.contains("context"))
            .expect("wide fixture renders context numbers");
        let context_split = context.find(" │ ").expect("context separator is visible");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(&context[..context_split]),
            split_column
        );
        assert!(wide.iter().any(|row| row.contains("100  next")));
        assert!(wide.iter().any(|row| row.contains("│ 200  next")));
        assert!(wide.iter().any(|row| row.contains("· index 111..222")));
        assert!(wide.iter().any(|row| row.contains("╭─ @@ -8,3 +18,3 @@")));
        assert!(wide.iter().any(|row| row.contains("╰──")));

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        let narrow_buffer = render(&renderer, &mut model, 64, 32);
        let narrow = rows(&narrow_buffer);
        assert!(
            narrow
                .iter()
                .any(|row| row.contains("-old_ascii_line") && !row.contains('≈'))
        );
        assert!(
            narrow
                .iter()
                .any(|row| row.contains("+new wide_") && !row.contains('≈'))
        );
        assert!(narrow.iter().any(|row| row.contains('…')));
        assert!(narrow.iter().all(|row| !row.contains('\t')));
        assert!(narrow.iter().any(|row| row.contains("  100 200 │  next")));
        assert!(
            narrow_buffer
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );

        let ascii_row = narrow
            .iter()
            .position(|row| row.contains("-old_ascii_line"))
            .expect("selected ASCII row is visible") as u16;
        assert_eq!(narrow_buffer[(0, ascii_row)].symbol(), "│");
        assert!(
            !narrow_buffer[(62, ascii_row)]
                .modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn wrapped_stack_rows_keep_gutters_and_selected_hunk_shape() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(READABLE_DIFF);
        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        crate::app::update(&mut model, review::Event::ToggleWrap);
        let buffer = render(&renderer, &mut model, 64, 40);
        let rendered = rows(&buffer);
        let continuation = rendered
            .iter()
            .find(|row| row.contains('↪'))
            .expect("wrapped source has a gutter-preserving continuation");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(continuation.as_str()),
            64
        );

        let boxed_source_rows = rendered
            .iter()
            .filter(|row| row.starts_with("│ "))
            .collect::<Vec<_>>();
        assert!(boxed_source_rows.len() >= 6);
    }

    #[test]
    fn syntax_colors_preserve_the_theme_truecolor_palette() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };
        let mut model = model_with_diff(
            "diff --git a/main.rs b/main.rs\n--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-fn old() { let value: usize = 1; }\n+fn new() { let value: usize = 2; }\n",
        );
        let buffer = render(&renderer, &mut model, 88, 20);

        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| matches!(cell.fg, Color::Rgb(_, _, _)))
        );
    }

    #[test]
    fn added_and_removed_rows_have_distinct_quiet_backgrounds() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };
        let mut model = model_with_diff(
            "diff --git a/main.rs b/main.rs\n--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-old_value\n+new_value\n",
        );
        let buffer = render(&renderer, &mut model, 80, 20);
        let rendered = rows(&buffer);
        let removed_y = rendered
            .iter()
            .position(|row| row.contains("-old_value"))
            .expect("removed row is visible") as u16;
        let added_y = rendered
            .iter()
            .position(|row| row.contains("+new_value"))
            .expect("added row is visible") as u16;
        let removed = (0..buffer.area.width)
            .find_map(|x| {
                (buffer[(x, removed_y)].symbol() == "o").then_some(buffer[(x, removed_y)].bg)
            })
            .expect("removed source cell is visible");
        let added = (0..buffer.area.width)
            .find_map(|x| (buffer[(x, added_y)].symbol() == "n").then_some(buffer[(x, added_y)].bg))
            .expect("added source cell is visible");

        assert_eq!(removed, Color::Rgb(54, 27, 32));
        assert_eq!(added, Color::Rgb(20, 46, 32));
        assert_ne!(removed, added);
    }

    #[test]
    fn tree_sitter_rust_query_highlights_macros_and_structural_punctuation() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };
        let mut model = model_with_diff(
            "diff --git a/main.rs b/main.rs\n--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-old();\n+assert_eq!(actual, expected);\n",
        );
        let buffer = render(&renderer, &mut model, 80, 20);
        let rendered = rows(&buffer);
        let y = rendered
            .iter()
            .position(|row| row.contains("assert_eq!(actual"))
            .expect("macro row is visible") as u16;
        let row = &rendered[usize::from(y)];
        let macro_byte = row.find("assert_eq!").expect("macro starts in row");
        let paren_byte = row.find('(').expect("opening parenthesis is visible");
        let macro_x = unicode_width::UnicodeWidthStr::width(&row[..macro_byte]) as u16;
        let paren_x = unicode_width::UnicodeWidthStr::width(&row[..paren_byte]) as u16;

        assert_eq!(buffer[(macro_x, y)].fg, Color::Rgb(220, 220, 170));
        assert!(buffer[(macro_x, y)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(paren_x, y)].fg, Color::Rgb(143, 161, 190));
    }

    #[test]
    fn layout_wrap_and_search_decorate_only_visible_hunks() {
        let mut raw =
            String::from("diff --git a/main.rs b/main.rs\n--- a/main.rs\n+++ b/main.rs\n");
        for index in 1..=40 {
            raw.push_str(&format!(
                "@@ -{index} +{index} @@\n-old_{index}\n+new_{index}\n"
            ));
        }
        let renderer = Renderer::default();
        let mut model = model_with_diff(&raw);
        let _ = render(&renderer, &mut model, 80, 20);
        let initial = renderer.hunk_text_cache.borrow().len();
        assert!(initial < 10, "rendered {initial} off-screen hunk variants");

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Split));
        let _ = render(&renderer, &mut model, 80, 20);
        crate::app::update(&mut model, review::Event::ToggleWrap);
        let _ = render(&renderer, &mut model, 80, 20);
        crate::app::update(&mut model, review::Event::BeginSearch);
        for character in "new_1".chars() {
            crate::app::update(&mut model, review::Event::InsertSearchCharacter(character));
            let _ = render(&renderer, &mut model, 80, 20);
        }

        let variants = renderer.hunk_text_cache.borrow().len();
        assert!(
            variants < 30,
            "layout/search/wrap decorated the whole diff: {variants} variants"
        );
    }

    #[test]
    fn no_color_environment_value_disables_the_semantic_palette() {
        assert!(SemanticTheme::from_no_color(None).colors_enabled());
        assert!(SemanticTheme::from_no_color(Some(std::ffi::OsStr::new(""))).colors_enabled());
        assert!(!SemanticTheme::from_no_color(Some(std::ffi::OsStr::new("1"))).colors_enabled());
    }

    #[test]
    fn status_changes_do_not_reflow_the_review_body() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        let before = rows(&render(&renderer, &mut model, 88, 20));
        model.global.status = Some("reloading diff…".into());
        let after = rows(&render(&renderer, &mut model, 88, 20));

        assert_eq!(&before[1..18], &after[1..18]);
        assert_ne!(before[18], after[18]);
        assert!(after[18].contains("Context: Diff"));
        assert!(after[18].contains("Status: reloading diff…"));
        assert_eq!(before[19], after[19]);
    }

    #[test]
    fn scrolling_reuses_the_predecorated_review_text() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(READABLE_DIFF);
        let _ = render(&renderer, &mut model, 80, 20);
        let initial_lines = renderer
            .hunk_text_cache
            .borrow()
            .front()
            .expect("initial render populates the hunk text cache")
            .lines
            .as_ptr();

        crate::app::update(&mut model, review::Event::ScrollRows(1));
        let _ = render(&renderer, &mut model, 80, 20);
        let scrolled_lines = renderer
            .hunk_text_cache
            .borrow()
            .front()
            .expect("scroll keeps the hunk text cache")
            .lines
            .as_ptr();

        assert_eq!(initial_lines, scrolled_lines);
    }

    #[test]
    fn hunk_and_file_navigation_reuse_the_predecorated_review_text() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        let initial = render(&renderer, &mut model, 80, 20);
        let initial_focus_count = (0..initial.area.height)
            .flat_map(|y| (0..initial.area.width).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let cell = &initial[(*x, *y)];
                cell.symbol() == "╭" && cell.fg == Color::Cyan
            })
            .count();
        let _initial_focus_row = (0..initial.area.height)
            .find(|y| {
                (0..initial.area.width).any(|x| {
                    let cell = &initial[(x, *y)];
                    cell.symbol() == "╭" && cell.fg == Color::Cyan
                })
            })
            .expect("selected hunk box is highlighted");
        assert_eq!(initial_focus_count, 1);
        let initial_location = model.review.selected_location();
        let initial_lines = renderer
            .hunk_text_cache
            .borrow()
            .front()
            .expect("initial render populates the hunk text cache")
            .lines
            .as_ptr();

        crate::app::update(&mut model, review::Event::MoveHunk(1));
        assert_ne!(model.review.selected_location(), initial_location);
        let navigated = render(&renderer, &mut model, 80, 20);
        let navigated_focus_count = (0..navigated.area.height)
            .flat_map(|y| (0..navigated.area.width).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let cell = &navigated[(*x, *y)];
                cell.symbol() == "╭" && cell.fg == Color::Cyan
            })
            .count();
        let _navigated_focus_row = (0..navigated.area.height)
            .find(|y| {
                (0..navigated.area.width).any(|x| {
                    let cell = &navigated[(x, *y)];
                    cell.symbol() == "╭" && cell.fg == Color::Cyan
                })
            })
            .expect("navigated hunk box is highlighted");
        assert_eq!(navigated_focus_count, 1);
        let navigated_lines = renderer
            .hunk_text_cache
            .borrow()
            .front()
            .expect("navigation keeps the hunk text cache")
            .lines
            .as_ptr();

        assert_eq!(initial_lines, navigated_lines);

        crate::app::update(&mut model, review::Event::MoveFile(1));
        let _ = render(&renderer, &mut model, 80, 20);
        let file_navigated_lines = renderer
            .hunk_text_cache
            .borrow()
            .front()
            .expect("file navigation keeps the hunk text cache")
            .lines
            .as_ptr();
        assert_eq!(initial_lines, file_navigated_lines);
    }

    #[test]
    fn modal_overlay_is_the_only_region_that_claims_focus() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::from_no_color(None),
            ..Renderer::default()
        };

        let mut composer = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(
            &mut composer,
            review::Event::BeginThread { always_new: true },
        );
        let semantic = crate::app::view(&composer);
        let Some(crate::semantic::Overlay::Composer(ref overlay)) = semantic.overlay else {
            panic!("composer overlay")
        };
        let overlay_area = super::centered_rect(70, overlay.height, Rect::new(0, 0, 120, 24));
        let buffer = render(&renderer, &mut composer, 120, 24);
        let rendered = rows(&buffer).join("\n");
        assert!(rendered.contains("Comment •"));
        assert!(rendered.contains("Ctrl-J post · Enter newline · Esc cancel"));
        assert!(!rendered.contains("DIFF FOCUS"));
        assert!(!rendered.contains("THREAD TARGET"));
        assert!(buffer[(0, 0)].modifier.contains(Modifier::DIM));
        assert_eq!(buffer[(0, 0)].fg, Color::DarkGray);
        assert_eq!(buffer[(overlay_area.x, overlay_area.y)].symbol(), "╭");
        assert_ne!(buffer[(overlay_area.x, overlay_area.y)].fg, Color::DarkGray);
        assert!(
            !buffer[(overlay_area.x, overlay_area.y)]
                .modifier
                .contains(Modifier::DIM)
        );

        let mut help = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut help, crate::app::global::Event::OpenHelp);
        let rendered = rows(&render(&renderer, &mut help, 120, 24)).join("\n");
        assert!(rendered.contains("Keyboard help — Esc/? to close"));
        assert!(!rendered.contains("DIFF FOCUS"));
        assert!(!rendered.contains("THREAD TARGET"));
    }

    #[test]
    fn composer_places_the_terminal_cursor_after_wide_graphemes() {
        let renderer = Renderer::default();
        let mut composer = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(
            &mut composer,
            review::Event::BeginThread { always_new: true },
        );
        crate::app::update(
            &mut composer,
            crate::mode::composer::Event::InsertCharacter('a'),
        );
        crate::app::update(
            &mut composer,
            crate::mode::composer::Event::InsertCharacter('画'),
        );
        crate::app::update(
            &mut composer,
            crate::app::global::Event::ViewportResized {
                rows: 19,
                columns: 120,
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        let semantic = crate::app::view(&composer);
        terminal
            .draw(|frame| renderer.render(frame, &semantic))
            .unwrap();

        terminal.backend_mut().assert_cursor_position((22, 11));
    }

    #[test]
    fn narrow_footer_keeps_target_and_mode_actions_visible() {
        let renderer = Renderer::default();
        let mut review = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(
            &mut review,
            crate::app::global::Event::ViewportResized {
                rows: 16,
                columns: 48,
            },
        );
        let review_rows = rows(&render(&renderer, &mut review, 48, 20));
        assert!(review_rows[18].contains("hunk 1/1"), "{}", review_rows[18]);
        assert!(review_rows[19].contains("q exit"), "{}", review_rows[19]);
        assert!(review_rows[19].contains("c comment"), "{}", review_rows[19]);
        assert!(review_rows[19].contains("? help"), "{}", review_rows[19]);
        crate::app::update(&mut review, review::Event::ToggleSidebar);
        crate::app::update(&mut review, review::Event::ToggleSidebar);
        let review_rows = rows(&render(&renderer, &mut review, 48, 20));
        assert!(
            review_rows[18].contains("rail hidden"),
            "{}",
            review_rows[18]
        );
        crate::app::update(
            &mut review,
            crate::app::global::Event::ViewportResized {
                rows: 16,
                columns: 64,
            },
        );
        crate::app::update(&mut review, review::Event::MoveFile(1));
        crate::app::update(&mut review, review::Event::ToggleSidebar);
        crate::app::update(&mut review, review::Event::ToggleSidebar);
        let wide_target_rows = rows(&render(&renderer, &mut review, 64, 20));
        assert!(
            wide_target_rows[18].contains("rail hidden"),
            "{}",
            wide_target_rows[18]
        );
        assert!(
            wide_target_rows[18].contains("hunk 1/1"),
            "{}",
            wide_target_rows[18]
        );

        let mut threads = ThreadState::default();
        let human = Participant {
            id: "human".into(),
            kind: ParticipantKind::Human,
        };
        threads.post(
            Anchor::new(
                "deadbeef",
                HunkLocation::new("src/components/review/navigation.rs", "@@ -1 +1 @@"),
            ),
            human,
            "thread".into(),
            1,
        );
        let mut thread = Model::new(
            DiffRequest {
                target: DiffTarget::WorkingTree,
                context_lines: 3,
            },
            LoadedDiff {
                text: RESPONSIVE_DIFF.into(),
                document: DiffDocument::parse(RESPONSIVE_DIFF),
            },
            threads,
        );
        crate::app::update(
            &mut thread,
            crate::app::global::Event::ViewportResized {
                rows: 16,
                columns: 64,
            },
        );
        crate::app::update(&mut thread, review::Event::MoveThread(1));
        let semantic = crate::app::view(&thread);
        assert!(semantic.footer.current_context.text.contains("thread #0"));
        assert!(semantic.footer.contextual_keys.text.contains("Tab diff"));
        assert!(semantic.footer.contextual_keys.text.contains("x resolve"));
        let thread_rows = rows(&render(&renderer, &mut thread, 64, 20)).join("\n");
        assert!(thread_rows.contains("Conversation #0"));
        assert!(thread_rows.contains("Conversation #0"));
    }

    #[test]
    fn empty_rollup_uses_its_own_title_and_available_keys() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::ShowRollup);
        let rendered = rows(&render(&renderer, &mut model, 120, 24)).join("\n");

        assert!(rendered.contains("Thread rollup"));
        assert!(rendered.contains("Keys: v/Esc return · no targets"));
        assert!(!rendered.contains("j/k select • Enter jump"));
    }

    #[test]
    fn diff_end_jump_renders_the_end_of_the_diff() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleSidebar);
        crate::app::update(
            &mut model,
            crate::app::global::Event::ViewportResized {
                rows: 10,
                columns: 80,
            },
        );
        crate::app::update(&mut model, review::Event::JumpToDiffEdge { end: true });

        let at_end = rows(&render(&renderer, &mut model, 80, 14)).join("\n");
        assert!(at_end.contains("new_wide"), "{at_end}");

        crate::app::update(&mut model, review::Event::ScrollRows(-1));
        let before_end = rows(&render(&renderer, &mut model, 80, 14)).join("\n");
        assert_ne!(at_end, before_end);
        assert!(before_end.contains("1 row before end"), "{before_end}");
    }

    #[test]
    fn sticky_context_and_scrollbar_track_the_same_review_viewport() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(READABLE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleSidebar);

        let top = render(&renderer, &mut model, 80, 14);
        let top_rows = rows(&top);
        assert!(top_rows[1].contains("󰈔 1/1 src/readable.rs"));
        let top_thumb = (1..12)
            .find(|y| top[(79, *y)].symbol() == symbols::SCROLL_THUMB)
            .expect("long review renders a scrollbar thumb");
        assert!((1..12).all(|y| matches!(top[(79, y)].symbol(), " " | symbols::SCROLL_THUMB)));

        crate::app::update(&mut model, review::Event::ScrollViewport(1));
        let middle = render(&renderer, &mut model, 80, 14);
        let middle_thumb = (1..12)
            .find(|y| middle[(79, *y)].symbol() == symbols::SCROLL_THUMB)
            .expect("middle review renders a scrollbar thumb");
        assert!(middle_thumb >= top_thumb);

        crate::app::update(&mut model, review::Event::JumpToDiffEdge { end: true });
        let end = render(&renderer, &mut model, 80, 14);
        let end_thumb = (1..12)
            .rev()
            .find(|y| end[(79, *y)].symbol() == symbols::SCROLL_THUMB)
            .expect("review end renders a scrollbar thumb");
        assert!(end_thumb > top_thumb);
        assert_eq!(end[(79, 11)].symbol(), symbols::SCROLL_THUMB);
        assert!(rows(&end)[1].contains("󰈔 1/1 src/readable.rs"));
    }

    #[test]
    fn logical_review_window_slices_beyond_u16_terminal_coordinates() {
        let body = ratatui::text::Text::from(
            (0..70_000)
                .map(|row| ratatui::text::Line::raw(row.to_string()))
                .collect::<Vec<_>>(),
        );

        let visible = logical_review_window(body, 69_990, 10);

        assert_eq!(visible.height(), 10);
        assert_eq!(visible.lines[0].to_string(), "69990");
        assert_eq!(visible.lines[9].to_string(), "69999");
    }
}
