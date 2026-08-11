use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};

use crate::{
    presentation,
    semantic::{Body, FileAttention, Overlay, ReviewBody, RollupBody, ThreadState, Tone, View},
    ui::{FocusArea, LayoutMode, ShellSize, truncate_end, truncate_start},
};
use unicode_width::UnicodeWidthStr;

const MINIMUM_WIDTH: u16 = 48;
const MINIMUM_HEIGHT: u16 = 8;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SemanticTheme {
    colors_enabled: bool,
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
    fn no_color() -> Self {
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
                rail.focused,
                rail.focused.then_some("FOCUSED"),
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
            Body::Review(review) => (
                self.review_text(review, body_inner_width, view),
                review.scroll,
            ),
            Body::Rollup(rollup) => (
                rollup_text(rollup, body_inner_width, self.semantic_theme),
                rollup.scroll,
            ),
        };
        let body_is_focused =
            view.layout.focus != FocusArea::Files || areas.navigation_rail.is_none();
        let body_focus_label = match view.layout.focus {
            FocusArea::Files if areas.navigation_rail.is_none() => {
                Some("FILES FOCUS · rail hidden")
            }
            FocusArea::Threads => Some("THREAD FOCUS"),
            FocusArea::Review => Some("FOCUSED"),
            FocusArea::Files => None,
        };
        let paragraph = Paragraph::new(body)
            .block(region_block(
                "Review stream",
                body_is_focused,
                body_focus_label,
                self.semantic_theme,
            ))
            .scroll((scroll, 0));
        if view.layout.wrap_lines
            && view.layout.diff_layout.resolved(body_inner_width) == LayoutMode::Stack
        {
            frame.render_widget(paragraph.wrap(Wrap { trim: false }), areas.review_body);
        } else {
            frame.render_widget(paragraph, areas.review_body);
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
            lines.push(Line::styled(
                format!("── {} ──", truncate_start(&file.path, path_width)),
                self.semantic_theme.style(Tone::Attention),
            ));
            lines.extend(file.metadata.iter().map(|line| {
                Line::styled(
                    truncate_end(line, usize::from(available_width)),
                    self.semantic_theme.style(Tone::MutedResolved),
                )
            }));
            let syntax = file
                .extension
                .as_deref()
                .and_then(|extension| self.syntax_set.find_syntax_by_extension(extension))
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
            let mut highlighter = HighlightLines::new(syntax, &self.theme);
            for hunk in &file.hunks {
                if let Some(header) = &hunk.header {
                    let style = if hunk.selected {
                        self.semantic_theme.selection()
                    } else {
                        self.semantic_theme.style(Tone::FocusSelection)
                    };
                    lines.push(Line::styled(
                        truncate_end(
                            &format!("{} {header}", if hunk.selected { "▶" } else { " " }),
                            usize::from(available_width),
                        ),
                        style,
                    ));
                }
                if view.layout.diff_layout.resolved(available_width) == LayoutMode::Split {
                    lines.extend(presentation::split_hunk_lines(
                        &hunk.lines,
                        available_width,
                        hunk.selected,
                        self.semantic_theme,
                    ));
                } else {
                    lines.extend(hunk.lines.iter().map(|line| {
                        presentation::highlight_line(
                            line,
                            &mut highlighter,
                            &self.syntax_set,
                            hunk.selected,
                            self.semantic_theme,
                        )
                    }));
                }
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
                        "  └ c reply · x resolve · r reopen · a attention",
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
                let area = centered_rect(86, 18, frame.area());
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(*text).wrap(Wrap { trim: false }).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Double)
                            .border_style(self.semantic_theme.style(Tone::FocusSelection))
                            .title("Keyboard help — Esc to close"),
                    ),
                    area,
                );
            }
        }
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
        "j/k select • Enter jump • v/Esc return",
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

    use super::{Renderer, SemanticTheme, ShellAreas};
    use crate::ui::truncate_start;

    const RESPONSIVE_DIFF: &str = "diff --git a/src/components/review/navigation.rs b/src/components/review/navigation.rs\n--- a/src/components/review/navigation.rs\n+++ b/src/components/review/navigation.rs\n@@ -1 +1 @@\n-old_navigation\n+new_navigation\ndiff --git a/src/画面/とても長いレビュー項目.rs b/src/画面/とても長いレビュー項目.rs\n--- a/src/画面/とても長いレビュー項目.rs\n+++ b/src/画面/とても長いレビュー項目.rs\n@@ -1 +1 @@\n-old_wide\n+new_wide\n";

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

    fn render(renderer: &Renderer, model: &Model, width: u16, height: u16) -> Buffer {
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
        let model = model_with_diff(RESPONSIVE_DIFF);

        let wide = rows(&render(&renderer, &model, 120, 24));
        assert!(wide[0].contains("resolved"));
        assert!(wide.iter().any(|row| row.contains("Files")));
        assert!(
            wide.iter()
                .any(|row| row.contains("Review stream ◆ FOCUSED"))
        );
        assert!(wide[22].contains("Context: Review stream"));
        assert!(wide[23].starts_with("Keys:"));
        let wide_areas = ShellAreas::resolve(Rect::new(0, 0, 120, 24), true);
        assert_eq!(wide_areas.header, Rect::new(0, 0, 120, 1));
        assert_eq!(wide_areas.navigation_rail, Some(Rect::new(0, 1, 30, 21)));
        assert_eq!(wide_areas.review_body, Rect::new(30, 1, 90, 21));
        assert_eq!(wide_areas.current_context, Rect::new(0, 22, 120, 1));
        assert_eq!(wide_areas.contextual_keys, Rect::new(0, 23, 120, 1));

        let medium = rows(&render(&renderer, &model, 88, 20));
        assert!(medium[0].contains("open"));
        assert!(!medium[0].contains("resolved"));
        assert!(medium.iter().any(|row| row.contains("Files")));
        assert!(medium.iter().any(|row| row.contains("-old_navigation")));
        let medium_areas = ShellAreas::resolve(Rect::new(0, 0, 88, 20), true);
        assert_eq!(medium_areas.navigation_rail, Some(Rect::new(0, 1, 28, 17)));
        assert_eq!(medium_areas.review_body, Rect::new(28, 1, 60, 17));
        assert_eq!(medium_areas.current_context.y, 18);
        assert_eq!(medium_areas.contextual_keys.y, 19);

        let narrow = rows(&render(&renderer, &model, 64, 16));
        assert_eq!(narrow[0].trim_end(), "revia • 2 files • 0 need you");
        assert!(!narrow.iter().any(|row| row.contains(" Files ")));
        assert!(
            narrow
                .iter()
                .any(|row| row.contains("review/navigation.rs"))
        );
        assert!(narrow[14].contains("Context: Review stream"));
        let narrow_areas = ShellAreas::resolve(Rect::new(0, 0, 64, 16), true);
        assert_eq!(narrow_areas.navigation_rail, None);
        assert_eq!(narrow_areas.review_body, Rect::new(0, 1, 64, 13));
        assert_eq!(narrow_areas.current_context.y, 14);
        assert_eq!(narrow_areas.contextual_keys.y, 15);

        let minimum = rows(&render(&renderer, &model, 48, 8));
        assert!(minimum.iter().any(|row| row.contains("Review stream")));
        assert!(minimum.iter().any(|row| row.contains("navigation.rs")));
        assert!(minimum[6].contains("Context:"));
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
        let model = model_with_diff(RESPONSIVE_DIFF);
        let buffer = render(&renderer, &model, 120, 24);
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
        let model = model_with_diff(RESPONSIVE_DIFF);
        let buffer = render(&renderer, &model, 120, 24);
        let rendered = rows(&buffer).join("\n");

        assert!(rendered.contains("›"));
        assert!(rendered.contains("▶"));
        assert!(rendered.contains("◆ FOCUSED"));
        assert!(rendered.contains("-- old_navigation"));
        assert!(rendered.contains("++ new_navigation"));
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
    }

    #[test]
    fn stack_selection_survives_hidden_hunk_headers_without_color() {
        let renderer = Renderer {
            semantic_theme: SemanticTheme::no_color(),
            ..Renderer::default()
        };
        let mut model = model_with_diff(RESPONSIVE_DIFF);
        crate::app::update(&mut model, review::Event::ToggleHunkHeaders);
        let buffer = render(&renderer, &model, 64, 16);
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
    fn syntax_colors_use_the_terminal_palette_instead_of_fixed_rgb() {
        let renderer = Renderer::default();
        let model = model_with_diff(RESPONSIVE_DIFF);
        let buffer = render(&renderer, &model, 88, 20);

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
        let before = rows(&render(&renderer, &model, 88, 20));
        model.global.status = Some("reloading diff…".into());
        let after = rows(&render(&renderer, &model, 88, 20));

        assert_eq!(&before[1..18], &after[1..18]);
        assert_ne!(before[18], after[18]);
        assert!(after[18].contains("Context: Review stream"));
        assert!(after[18].contains("Status: reloading diff…"));
        assert_eq!(before[19], after[19]);
    }
}
