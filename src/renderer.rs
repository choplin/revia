use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};

use crate::{
    diff::DiffLineKind,
    presentation,
    semantic::{
        Body, DiffSearchTarget, FileAttention, Overlay, ReviewBody, RollupBody,
        StickyReviewContext, ThreadState, Tone, View,
    },
    ui::{FocusArea, LayoutMode, ShellSize, truncate_end, truncate_start},
};
use unicode_width::UnicodeWidthStr;

const MINIMUM_WIDTH: u16 = 48;
const MINIMUM_HEIGHT: u16 = 8;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SemanticTheme {
    colors_enabled: bool,
}

fn sticky_context_text(context: &StickyReviewContext, width: u16) -> String {
    let file = format!(
        "▣ {}/{} {}",
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

    fn from_no_color(no_color: Option<&std::ffi::OsStr>) -> Self {
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
    syntax_set: SyntaxSet,
    theme: Theme,
    semantic_theme: SemanticTheme,
}

impl Default for Renderer {
    fn default() -> Self {
        let themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .get("base16-ocean.dark")
            .or_else(|| themes.themes.values().next())
            .expect("syntect includes a default theme")
            .clone();
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme,
            semantic_theme: SemanticTheme::from_environment(),
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
                        FileAttention::NeedsAttention => "!",
                        FileAttention::Open => "•",
                        FileAttention::Resolved => "✓",
                        FileAttention::None => " ",
                    };
                    let selection = if rail.selected == Some(index) {
                        "›"
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

        let body_inner_width = areas.review_body.width.saturating_sub(2);
        let (body, scroll) = match &view.body {
            Body::Review(review) => {
                let body = self.review_text(review, review.viewport.presentation_width, view);
                debug_assert_eq!(body.height(), review.viewport.total_rows);
                (
                    logical_review_window(body, review.scroll, review.viewport.visible_rows),
                    0,
                )
            }
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
        let body_is_focused = view.overlay.is_none();
        let body_title = match &view.body {
            Body::Review(_) => "Review stream",
            Body::Rollup(_) => "Thread rollup",
        };
        let body_focus_label = match (&view.body, view.layout.focus, body_is_focused) {
            (_, _, false) => None,
            (Body::Rollup(_), _, true) => Some("ROLLUP FOCUS"),
            (Body::Review(_), FocusArea::Threads, true) => Some("THREAD TARGET"),
            (Body::Review(_), FocusArea::Review, true) => Some("STREAM FOCUS"),
        };
        frame.render_widget(
            region_block(
                body_title,
                body_is_focused,
                body_focus_label,
                self.semantic_theme,
            ),
            areas.review_body,
        );
        let inner = Rect::new(
            areas.review_body.x.saturating_add(1),
            areas.review_body.y.saturating_add(1),
            areas.review_body.width.saturating_sub(2),
            areas.review_body.height.saturating_sub(2),
        );
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
                    .end_symbol(None),
                areas.review_body,
                &mut state,
            );
        }

        frame.render_widget(
            Paragraph::new(format!("◆ {}", view.footer.current_context.text))
                .style(self.semantic_theme.style(Tone::FocusSelection)),
            areas.current_context,
        );
        frame.render_widget(
            Paragraph::new(format!("Keys: {}", view.footer.contextual_keys.text))
                .style(self.semantic_theme.style(Tone::MutedResolved)),
            areas.contextual_keys,
        );
        if let Some(overlay) = &view.overlay {
            self.render_overlay(frame, overlay);
        }
    }

    fn review_text(&self, review: &ReviewBody, available_width: u16, view: &View) -> Text<'static> {
        if let Some(message) = &review.empty_state {
            return Text::raw(message.clone());
        }
        let mut lines = Vec::new();
        for file in &review.files {
            lines.push(Line::raw(""));
            let path_width = usize::from(available_width).saturating_sub(6);
            let path_match = matches!(
                review.search_target.as_ref(),
                Some(DiffSearchTarget::FilePath { path }) if path == &file.path
            );
            lines.push(Line::styled(
                format!(
                    "{}─ {} ──",
                    if path_match { "⌕" } else { "─" },
                    truncate_start(&file.path, path_width)
                ),
                if path_match {
                    self.semantic_theme.selection()
                } else {
                    self.semantic_theme.style(Tone::Attention)
                },
            ));
            lines.extend(file.metadata.iter().map(|line| {
                Line::styled(
                    truncate_end(&format!("· {line}"), usize::from(available_width)),
                    self.semantic_theme.style(Tone::MutedResolved),
                )
            }));
            let syntax = file
                .extension
                .as_deref()
                .and_then(|extension| self.syntax_set.find_syntax_by_extension(extension))
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
            let mut highlighter = HighlightLines::new(syntax, &self.theme);
            let number_width = presentation::line_number_width(
                file.hunks
                    .iter()
                    .map(|hunk| (hunk.lines.as_slice(), hunk.coordinates)),
            );
            for hunk in &file.hunks {
                if let Some(header) = &hunk.header {
                    let header_match = matches!(
                        review.search_target.as_ref(),
                        Some(DiffSearchTarget::HunkHeader { location }) if location == &hunk.anchor
                    );
                    let style = if header_match || hunk.selected {
                        self.semantic_theme.selection()
                    } else {
                        self.semantic_theme.style(Tone::FocusSelection)
                    };
                    lines.push(Line::styled(
                        truncate_end(
                            &format!(
                                "{} {header}",
                                if header_match {
                                    "⌕"
                                } else if hunk.selected {
                                    "▶"
                                } else {
                                    " "
                                }
                            ),
                            usize::from(available_width),
                        ),
                        style,
                    ));
                }
                let mut hunk_lines =
                    if view.layout.diff_layout.resolved(available_width) == LayoutMode::Split {
                        presentation::split_hunk_lines(
                            &hunk.lines,
                            hunk.coordinates,
                            available_width,
                            number_width,
                            hunk.selected,
                            self.semantic_theme,
                        )
                    } else {
                        presentation::stack_hunk_lines(
                            &hunk.lines,
                            hunk.coordinates,
                            available_width,
                            number_width,
                            view.layout.wrap_lines,
                            hunk.selected,
                            &mut highlighter,
                            &self.syntax_set,
                            self.semantic_theme,
                        )
                    };
                let search_row = match review.search_target.as_ref() {
                    Some(DiffSearchTarget::HunkHeader { location })
                        if location == &hunk.anchor && hunk.header.is_none() =>
                    {
                        Some((0, "⌕ "))
                    }
                    Some(DiffSearchTarget::DiffLine {
                        location,
                        line_index,
                    }) if location == &hunk.anchor => {
                        let marker = hunk
                            .lines
                            .get(*line_index)
                            .map(|line| search_marker(line.kind))
                            .unwrap_or("⌕ ");
                        presentation::search_line_row(
                            &hunk.lines,
                            hunk.coordinates,
                            *line_index,
                            available_width,
                            number_width,
                            view.layout,
                        )
                        .map(|row| (row, marker))
                    }
                    _ => None,
                };
                if let Some((row, search_marker)) = search_row
                    && let Some(line) = hunk_lines.get_mut(row)
                    && let Some(marker) = line.spans.first_mut()
                {
                    marker.content = search_marker.into();
                    marker.style = self.semantic_theme.style(Tone::Attention);
                }
                lines.extend(hunk_lines);
                for thread in &hunk.threads {
                    let state = match thread.state {
                        ThreadState::NeedsAttention => "NEEDS ATTENTION",
                        ThreadState::Open => "OPEN",
                        ThreadState::Resolved => "RESOLVED",
                    };
                    let style = match (thread.active, thread.state) {
                        (true, _) => self.semantic_theme.selection(),
                        (false, ThreadState::NeedsAttention) => {
                            self.semantic_theme.style(Tone::Attention)
                        }
                        (false, ThreadState::Resolved) => {
                            self.semantic_theme.style(Tone::MutedResolved)
                        }
                        (false, ThreadState::Open) => Style::default(),
                    };
                    lines.push(Line::styled(
                        format!(
                            "{} ┌ #{:03} {state}{}",
                            if thread.active { "▶" } else { " " },
                            thread.id,
                            if thread.outdated { " · outdated" } else { "" }
                        ),
                        style,
                    ));
                    lines.push(Line::styled(format!("  │ {}", thread.latest), style));
                    lines.push(Line::styled(
                        if thread.active {
                            "  └ c reply · x resolve · R reopen · a attention"
                        } else {
                            "  └ select with t/T to use thread actions"
                        },
                        style,
                    ));
                }
            }
        }
        Text::from(lines)
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
            Overlay::Composer { input, replying } => {
                let area = centered_rect(70, 7, frame.area());
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(input.as_str()).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Double)
                            .style(self.semantic_theme.style(Tone::FocusSelection))
                            .title(format!(
                                " {} — Enter post · Esc cancel ",
                                if *replying { "Reply" } else { "New thread" }
                            )),
                    ),
                    area,
                );
            }
            Overlay::Help { text } => {
                let area = centered_rect(86, 20, frame.area());
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(text.as_str())
                        .wrap(Wrap { trim: false })
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Double)
                                .border_style(self.semantic_theme.style(Tone::FocusSelection))
                                .title("Keyboard help — Esc/? to close"),
                        ),
                    area,
                );
            }
        }
    }
}

fn search_marker(kind: DiffLineKind) -> &'static str {
    match kind {
        DiffLineKind::Added => "⌕+",
        DiffLineKind::Removed => "⌕-",
        DiffLineKind::Context | DiffLineKind::Meta => "⌕ ",
    }
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
                    if item.selected { "› " } else { "  " },
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
            "revia  •  {} files  •  {} need you  •  {} open  •  {} resolved",
            view.header.file_count,
            view.header.needs_attention,
            view.header.open,
            view.header.resolved
        ),
        ShellSize::Medium => format!(
            "revia  •  {} files  •  {} need you  •  {} open",
            view.header.file_count, view.header.needs_attention, view.header.open
        ),
        ShellSize::Narrow => format!(
            "revia • {} files • {} need you",
            view.header.file_count, view.header.needs_attention
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
        .title(match focus_label {
            Some(label) => format!(" {title} ◆ {label} "),
            None => format!(" {title} "),
        });
    if focused {
        block
            .border_type(BorderType::Double)
            .border_style(theme.style(Tone::FocusSelection))
    } else {
        block.border_style(theme.style(Tone::MutedResolved))
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
        style::{Color, Modifier},
    };

    use crate::{
        anchor::{Anchor, HunkLocation},
        app::Model,
        diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff},
        mode::review,
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
        let renderer = Renderer::default();

        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        let semantic = crate::app::view(&model);
        terminal
            .draw(|frame| renderer.render(frame, &semantic))
            .unwrap();
        let split = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(split.contains("1 need you"));
        assert!(split.contains("NEEDS ATTENTION"));
        assert!(split.contains("RESOLVED"));
        assert!(split.contains(" │ "));

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        let semantic = crate::app::view(&model);
        let mut terminal = Terminal::new(TestBackend::new(80, 32)).unwrap();
        terminal
            .draw(|frame| renderer.render(frame, &semantic))
            .unwrap();
        let stack = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(stack.contains("-old_a"));
        assert!(stack.contains("+new_a"));
    }

    #[test]
    fn renders_a_stable_explanation_below_the_minimum_layout_size() {
        let renderer = Renderer::default();
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
    fn wide_medium_narrow_and_minimum_shells_have_intentional_roles() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);

        let wide = rows(&render(&renderer, &mut model, 120, 24));
        assert!(wide[0].contains("resolved"));
        assert!(wide.iter().any(|row| row.contains("Files")));
        assert!(
            wide.iter()
                .any(|row| row.contains("Review stream ◆ STREAM FOCUS"))
        );
        assert!(wide[22].contains("Context: Review stream"));
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
        assert_eq!(narrow[0].trim_end(), "revia • 2 files • 0 need you");
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
        assert!(minimum.iter().any(|row| row.contains("Review stream")));
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
            assert_eq!(buffer[(30, y)].symbol(), "║");
        }
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

        assert!(rendered.contains("›"));
        assert!(rendered.contains("▶"));
        assert!(rendered.contains("◆ STREAM FOCUS"));
        assert!(rendered.contains("1 -old_navigation"));
        assert!(rendered.contains("1 +new_navigation"));
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
        assert!(rendered.contains("⌕+"));
        assert!(rendered.contains("Context: Search input"));
        assert!(rendered.contains("Esc cancel"));
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
        assert!(rendered.contains("⌕ "));
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
        assert!(removed.contains("⌕-"));
        crate::app::update(&mut model, review::Event::FinishSearch);
        crate::app::update(&mut model, review::Event::MoveSearch(1));
        let added = rows(&render(&renderer, &mut model, 120, 16)).join("\n");
        assert!(added.contains("⌕+"));
        assert!(!added.contains("⌕-"));
    }

    #[test]
    fn stack_selection_survives_hidden_hunk_headers_without_color() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleHunkHeaders);
        let buffer = render(&renderer, &mut model, 64, 16);
        let rendered = rows(&buffer).join("\n");

        assert!(!rendered.contains("▶"));
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| { cell.symbol() == "-" && cell.modifier.contains(Modifier::REVERSED) })
        );
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
        assert_selected_source_block(&split, &["┃  8  before", "┃  9 -old_", "┃ 10  after"]);

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        crate::app::update(&mut model, review::Event::ToggleWrap);
        let stack = render(&renderer, &mut model, 64, 24);
        assert_selected_source_block(
            &stack,
            &[
                "┃  8 18 │  before",
                "┃  9    │ -old_",
                "┃       │ ↪",
                "┃    19 │ +new",
                "┃ 10 20 │  after",
            ],
        );
    }

    fn assert_selected_source_block(buffer: &Buffer, expected_rows: &[&str]) {
        let rendered = rows(buffer);
        assert!(rendered.iter().all(|row| !row.contains("@@")));
        let start = rendered
            .iter()
            .position(|row| row.contains(expected_rows[0]))
            .expect("first expected selected source row is visible");
        assert_eq!(
            rendered.iter().filter(|row| row.contains('┃')).count(),
            expected_rows.len()
        );
        for (offset, expected) in expected_rows.iter().enumerate() {
            let y = start + offset;
            assert!(
                rendered[y].contains(expected),
                "row {y} did not contain {expected:?}: {:?}",
                rendered[y]
            );
            for x in 1..buffer.area.width.saturating_sub(1) {
                assert!(
                    buffer[(x, y as u16)].modifier.contains(Modifier::REVERSED),
                    "selection style missing at ({x}, {y})"
                );
            }
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
        assert!(replacement.contains("┃   9 -old_ascii_line"));
        assert!(replacement.contains("│  19 +new wide_"), "{replacement:?}");
        let split_at = replacement.find(" │ ").expect("split separator is visible");
        let split_column = unicode_width::UnicodeWidthStr::width(&replacement[..split_at]);
        let context = wide
            .iter()
            .find(|row| row.contains("┃   8  context"))
            .expect("wide fixture renders context numbers");
        let context_split = context.find(" │ ").expect("context separator is visible");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(&context[..context_split]),
            split_column
        );
        assert!(wide.iter().any(|row| row.contains("100  next")));
        assert!(wide.iter().any(|row| row.contains("│ 200  next")));
        assert!(wide.iter().any(|row| row.contains("· index 111..222")));
        assert!(wide.iter().any(|row| row.contains("▶ @@ -8,3 +18,3 @@")));

        crate::app::update(&mut model, review::Event::SetLayout(LayoutMode::Stack));
        let narrow_buffer = render(&renderer, &mut model, 64, 32);
        let narrow = rows(&narrow_buffer);
        assert!(
            narrow
                .iter()
                .any(|row| row.contains("┃   9     │ -old_ascii_line"))
        );
        assert!(
            narrow
                .iter()
                .any(|row| row.contains("┃      19 │ +new wide_"))
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

        for (y, _row) in narrow
            .iter()
            .enumerate()
            .filter(|(_, row)| row.contains('┃'))
        {
            assert_eq!(narrow_buffer[(63, y as u16)].symbol(), "║");
            assert!(
                narrow_buffer[(1, y as u16)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
            assert!(
                narrow_buffer[(62, y as u16)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
        }
        let ascii_selection_row = narrow
            .iter()
            .position(|row| row.contains("-old_ascii_line"))
            .expect("selected ASCII row is visible") as u16;
        for x in 1..63 {
            assert!(
                narrow_buffer[(x, ascii_selection_row)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
        }
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
            .find(|row| row.contains("┃         │ ↪"))
            .expect("wrapped source has a gutter-preserving continuation");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(continuation.as_str()),
            64
        );

        let selected_source_rows = rendered
            .iter()
            .filter(|row| row.contains("║┃ "))
            .collect::<Vec<_>>();
        assert!(selected_source_rows.len() >= 6);
    }

    #[test]
    fn syntax_colors_use_the_terminal_palette_instead_of_fixed_rgb() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        let buffer = render(&renderer, &mut model, 88, 20);

        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| !matches!(cell.fg, Color::Rgb(_, _, _)))
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
        assert!(after[18].contains("Context: Review stream"));
        assert!(after[18].contains("Status: reloading diff…"));
        assert_eq!(before[19], after[19]);
    }

    #[test]
    fn modal_overlay_is_the_only_region_that_claims_focus() {
        let renderer = Renderer::default();

        let mut composer = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(
            &mut composer,
            review::Event::BeginThread { always_new: true },
        );
        let rendered = rows(&render(&renderer, &mut composer, 120, 24)).join("\n");
        assert!(rendered.contains("New thread — Enter post · Esc cancel"));
        assert!(!rendered.contains("STREAM FOCUS"));
        assert!(!rendered.contains("THREAD TARGET"));

        let mut help = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut help, crate::app::global::Event::OpenHelp);
        let rendered = rows(&render(&renderer, &mut help, 120, 24)).join("\n");
        assert!(rendered.contains("Keyboard help — Esc/? to close"));
        assert!(!rendered.contains("STREAM FOCUS"));
        assert!(!rendered.contains("THREAD TARGET"));
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
        assert!(
            review_rows[19].contains("j/k stream"),
            "{}",
            review_rows[19]
        );
        assert!(review_rows[19].contains(",/. file"), "{}", review_rows[19]);
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
        let thread_rows = rows(&render(&renderer, &mut thread, 64, 20));
        assert!(thread_rows[18].contains("thread #0"), "{}", thread_rows[18]);
        assert!(
            thread_rows[19].contains("Tab stream"),
            "{}",
            thread_rows[19]
        );
        assert!(thread_rows[19].contains("x/R"), "{}", thread_rows[19]);
    }

    #[test]
    fn empty_rollup_uses_its_own_title_and_available_keys() {
        let renderer = Renderer::default();
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::ShowRollup);
        let rendered = rows(&render(&renderer, &mut model, 120, 24)).join("\n");

        assert!(rendered.contains("Thread rollup ◆ ROLLUP FOCUS"));
        assert!(rendered.contains("No thread targets • v/Esc return"));
        assert!(!rendered.contains("j/k select • Enter jump"));
    }

    #[test]
    fn stream_end_jump_renders_the_end_of_the_diff() {
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
        crate::app::update(&mut model, review::Event::JumpToStreamEdge { end: true });

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
        assert!(top_rows[2].contains("▣ 1/1 src/readable.rs"));
        let top_thumb = (2..12)
            .find(|y| top[(79, *y)].symbol() == "█")
            .expect("long review renders a scrollbar thumb");

        crate::app::update(&mut model, review::Event::ScrollViewport(1));
        let middle = render(&renderer, &mut model, 80, 14);
        let middle_thumb = (2..12)
            .find(|y| middle[(79, *y)].symbol() == "█")
            .expect("middle review renders a scrollbar thumb");
        assert!(middle_thumb >= top_thumb);

        crate::app::update(&mut model, review::Event::JumpToStreamEdge { end: true });
        let end = render(&renderer, &mut model, 80, 14);
        let end_thumb = (2..12)
            .rev()
            .find(|y| end[(79, *y)].symbol() == "█")
            .expect("review end renders a scrollbar thumb");
        assert!(end_thumb > top_thumb);
        assert_eq!(end[(79, 11)].symbol(), "█");
        assert!(rows(&end)[2].contains("▣ 1/1 src/readable.rs"));
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
