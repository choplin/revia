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
    semantic::{Body, FileAttention, Overlay, ReviewBody, RollupBody, ThreadState, View},
    ui::LayoutMode,
};

pub struct Renderer {
    syntax_set: SyntaxSet,
    theme: Theme,
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
        }
    }
}

impl Renderer {
    pub fn render(&self, frame: &mut Frame, view: &View) {
        let [header, content, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        let (sidebar, body_area) = if view.file_rail.is_some() {
            let [sidebar, body] =
                Layout::horizontal([Constraint::Length(30), Constraint::Min(1)]).areas(content);
            (Some(sidebar), body)
        } else {
            (None, content)
        };

        frame.render_widget(
            Paragraph::new(format!(
                "revia  •  {} files  •  {} need you  •  {} open  •  {} resolved",
                view.header.file_count,
                view.header.needs_attention,
                view.header.open,
                view.header.resolved
            ))
            .style(Style::default().add_modifier(Modifier::BOLD)),
            header,
        );

        if let (Some(area), Some(rail)) = (sidebar, &view.file_rail) {
            let items = rail
                .items
                .iter()
                .map(|file| {
                    let marker = match file.attention {
                        FileAttention::NeedsAttention => "!",
                        FileAttention::Open => "•",
                        FileAttention::Resolved => "✓",
                        FileAttention::None => " ",
                    };
                    ListItem::new(format!(
                        "{marker} {}  {}h/{}t",
                        file.path, file.hunk_count, file.thread_count
                    ))
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default();
            state.select(rail.selected);
            frame.render_stateful_widget(
                List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::RIGHT)
                            .title(if rail.focused {
                                "Files • focus"
                            } else {
                                "Files"
                            }),
                    )
                    .highlight_style(
                        Style::default()
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    ),
                area,
                &mut state,
            );
        }

        let (body, scroll) = match &view.body {
            Body::Review(review) => (
                self.review_text(review, body_area.width, view),
                review.scroll,
            ),
            Body::Rollup(rollup) => (rollup_text(rollup), rollup.scroll),
        };
        let paragraph = Paragraph::new(body)
            .block(Block::default().borders(Borders::NONE).title(
                if view.layout.focus == crate::ui::FocusArea::Review {
                    "Review stream • focus"
                } else {
                    "Review stream"
                },
            ))
            .scroll((scroll, 0));
        if view.layout.wrap_lines
            && view.layout.diff_layout.resolved(body_area.width) == LayoutMode::Stack
        {
            frame.render_widget(paragraph.wrap(Wrap { trim: false }), body_area);
        } else {
            frame.render_widget(paragraph, body_area);
        }

        frame.render_widget(
            Paragraph::new(view.footer.text.as_str()).style(Style::default().fg(Color::Gray)),
            footer,
        );
        if let Some(overlay) = &view.overlay {
            self.render_overlay(frame, overlay);
        }
    }

    fn review_text(&self, review: &ReviewBody, available_width: u16, view: &View) -> Text<'static> {
        if review.files.is_empty() {
            return Text::raw("No changed files.");
        }
        let mut lines = Vec::new();
        for file in &review.files {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                format!("── {} ──", file.path),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            lines.extend(
                file.metadata
                    .iter()
                    .map(|line| Line::styled(line.clone(), Style::default().fg(Color::DarkGray))),
            );
            let syntax = file
                .extension
                .as_deref()
                .and_then(|extension| self.syntax_set.find_syntax_by_extension(extension))
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
            let mut highlighter = HighlightLines::new(syntax, &self.theme);
            for hunk in &file.hunks {
                if let Some(header) = &hunk.header {
                    let style = if hunk.selected {
                        Style::default()
                            .bg(Color::DarkGray)
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    };
                    lines.push(Line::styled(
                        format!("{} {header}", if hunk.selected { "▶" } else { " " }),
                        style,
                    ));
                }
                if view.layout.diff_layout.resolved(available_width) == LayoutMode::Split {
                    lines.extend(presentation::split_hunk_lines(
                        &hunk.lines,
                        available_width,
                        hunk.selected,
                    ));
                } else {
                    lines.extend(hunk.lines.iter().map(|line| {
                        presentation::highlight_line(line, &mut highlighter, &self.syntax_set)
                    }));
                }
                for thread in &hunk.threads {
                    let state = match thread.state {
                        ThreadState::NeedsAttention => "NEEDS ATTENTION",
                        ThreadState::Open => "OPEN",
                        ThreadState::Resolved => "RESOLVED",
                    };
                    let style = if thread.active {
                        Style::default().bg(Color::DarkGray).fg(Color::Yellow)
                    } else if thread.state == ThreadState::NeedsAttention {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    lines.push(Line::styled(
                        format!(
                            "  ┌ #{:03} {state}{}",
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
                            .style(Style::default().bg(Color::Rgb(20, 24, 34)).fg(Color::White))
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
                            .title("Keyboard help — Esc to close"),
                    ),
                    area,
                );
            }
        }
    }
}

fn rollup_text(rollup: &RollupBody) -> Text<'static> {
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
            format!(
                "{}#{id} [{state}] {} {}{provenance}",
                if item.selected { "> " } else { "  " },
                item.path,
                item.hunk_header,
                id = item.id
            ),
            if item.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            },
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k select • Enter jump • v/Esc return",
        Style::default().fg(Color::Gray),
    ));
    Text::from(lines)
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
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{
        anchor::{Anchor, HunkLocation},
        app::Model,
        diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff},
        mode::review,
        thread::{Participant, ParticipantKind, ThreadState},
        ui::LayoutMode,
    };

    use super::Renderer;

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
}
