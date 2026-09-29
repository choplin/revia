//! Urushi view construction for Revia's semantic UI.
//!
//! Layout, panels, viewports, scrollbars, overlays, cursor placement, and text
//! decoration are native Urushi values.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use urushi::{
    Align, BlockStyle, BlockTitle, Border, Canvas, CanvasContext, CanvasItem, CellContribution,
    Composition, Length, Overflow, Position, PositionedCell, Projection, ProjectionBoundary,
    Scrollbar, ScrollbarGlyphs, ScrollbarOrientation, ScrollbarPresentation, StyledText, TextStyle,
    VerticalAlign, View, Viewport,
};
use urushi_tui_app::SurfaceSize;

use crate::{
    diff::FileStatus,
    renderer::{self, Renderer, SemanticTheme},
    semantic::{
        Body, FileRailRow, FileRailRowKind, Overlay, ReviewBody, RollupBody, ThreadState, Tone,
        View as SemanticView,
    },
    styled_text::{Document, Line, Span, patch_style},
    symbols,
    ui::{FocusArea, ShellSize, truncate_end},
};

const MINIMUM_WIDTH: usize = 48;
const MINIMUM_HEIGHT: usize = 8;

#[derive(Default)]
pub(crate) struct UrushiRenderer {
    presentation: Renderer,
}

impl UrushiRenderer {
    pub(crate) fn view(&self, semantic: &SemanticView, surface: SurfaceSize) -> View {
        let columns = surface.columns();
        let rows = surface.rows();
        if columns < MINIMUM_WIDTH || rows < MINIMUM_HEIGHT {
            return too_small_view(columns, rows);
        }

        let base = self.shell_view(semantic, columns, rows);
        match semantic.overlay.clone() {
            Some(overlay) => View::canvas(Canvas::new().item(OverlayItem {
                base,
                overlay,
                theme: self.presentation.semantic_theme(),
            })),
            None => base,
        }
    }

    fn shell_view(&self, semantic: &SemanticView, columns: usize, rows: usize) -> View {
        let width = cell_u16(columns);
        let theme = self.presentation.semantic_theme();
        let size = ShellSize::for_width(width);
        let header = fixed_row(
            View::text(renderer::header_text(semantic, width, size), theme.header()),
            1,
        );
        let body = match (semantic.file_rail.as_ref(), size.rail_width(width)) {
            (Some(rail), Some(rail_width)) => View::row(
                VerticalAlign::Top,
                [
                    self.file_rail_view(semantic, rail, rail_width, rows.saturating_sub(3), theme),
                    fill_box(self.body_view(semantic, theme, width.saturating_sub(rail_width))),
                ],
            ),
            _ => fill_box(self.body_view(semantic, theme, width)),
        };
        let context = fixed_row(
            View::text(
                semantic.footer.current_context.text.clone(),
                theme.style(Tone::FocusSelection),
            ),
            1,
        );
        let keys = fixed_row(
            View::text(
                format!("Keys: {}", semantic.footer.contextual_keys.text),
                theme.style(Tone::MutedResolved),
            ),
            1,
        );

        View::block(
            BlockStyle::new()
                .width(Length::fill(1))
                .height(Length::Cells(cell_u16(rows))),
            View::column(Align::Left, [header, fill_box(body), context, keys]),
        )
    }

    fn file_rail_view(
        &self,
        semantic: &SemanticView,
        rail: &crate::semantic::FileRail,
        width: u16,
        height: usize,
        theme: SemanticTheme,
    ) -> View {
        let inner_width = usize::from(width.saturating_sub(2));
        let focused = semantic.layout.focus == FocusArea::Files;
        let content = View::styled_text(
            Document::from(
                rail.rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        file_list_line(row, inner_width, rail.selected == Some(index), theme)
                    })
                    .collect::<Vec<_>>(),
            )
            .into_styled_text(),
        );
        let visible_rows = height.saturating_sub(2).max(1);
        let scroll = rail
            .selected
            .map(|selected| selected.saturating_sub(visible_rows.saturating_sub(1)))
            .unwrap_or(0);
        let content = View::viewport(
            Viewport::vertical(Projection::new(
                cell_i64(scroll),
                ProjectionBoundary::Preserve,
            )),
            content,
        );
        let focus_label = if focused {
            Some("FILE FOCUS")
        } else {
            rail.selected.map(|_| "CURRENT FILE")
        };
        titled_panel(
            PanelSpec {
                width,
                height: Length::fill(1),
                title: "Files",
                focus_label,
                focused,
                border: Border::ROUNDED,
            },
            theme,
            content,
        )
    }

    fn body_view(&self, semantic: &SemanticView, theme: SemanticTheme, width: u16) -> View {
        match &semantic.body {
            Body::Review(review) => self.review_body_view(semantic, review, theme),
            Body::Rollup(rollup) => self.rollup_body_view(semantic, rollup, theme, width),
        }
    }

    fn review_body_view(
        &self,
        semantic: &SemanticView,
        review: &ReviewBody,
        theme: SemanticTheme,
    ) -> View {
        let body = text_view(self.presentation.review_window(
            review,
            review.viewport.presentation_width,
            semantic,
        ));
        let content = if let Some(context) = &review.viewport.sticky_context {
            let [file, hunk] =
                renderer::sticky_context_lines(context, review.viewport.presentation_width);
            let file_tone = if semantic.layout.focus == FocusArea::Review {
                Tone::FocusSelection
            } else {
                Tone::MutedResolved
            };
            View::column(
                Align::Left,
                [
                    fixed_row(View::text(file, theme.style(file_tone)), 1),
                    fixed_row(View::text(hunk, theme.style(Tone::MutedResolved)), 1),
                    fill_box(body),
                ],
            )
        } else {
            fill_box(body)
        };

        let scrollbar = if review.viewport.total_rows > review.viewport.visible_rows {
            let presentation = ScrollbarPresentation::new(
                theme.style(Tone::FocusSelection),
                theme.style(Tone::MutedResolved),
            )
            .glyphs(
                ScrollbarOrientation::Vertical,
                ScrollbarGlyphs::new(symbols::SCROLL_THUMB)
                    .track(None)
                    .begin(None)
                    .end(None),
            );
            presentation.compose(
                &Scrollbar::new(
                    ScrollbarOrientation::Vertical,
                    review.viewport.total_rows,
                    review.viewport.visible_rows,
                )
                .position(review.scroll),
            )
        } else {
            View::block(
                BlockStyle::new()
                    .width(Length::Cells(1))
                    .height(Length::fill(1)),
                View::empty(),
            )
        };
        View::row(VerticalAlign::Top, [fill_box(content), scrollbar])
    }

    fn rollup_body_view(
        &self,
        semantic: &SemanticView,
        rollup: &RollupBody,
        theme: SemanticTheme,
        width: u16,
    ) -> View {
        let presentation_width = width.saturating_sub(2);
        let text = text_view(renderer::rollup_text(rollup, presentation_width, theme));
        let viewport = View::viewport(
            Viewport::vertical(Projection::new(
                i64::from(rollup.scroll),
                ProjectionBoundary::Preserve,
            )),
            text,
        );
        titled_panel(
            PanelSpec {
                width: 0,
                height: Length::fill(1),
                title: "Thread rollup",
                focus_label: None,
                focused: semantic.overlay.is_none(),
                border: Border::ROUNDED,
            },
            theme,
            viewport,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
struct OverlayItem {
    base: View,
    overlay: Overlay,
    theme: SemanticTheme,
}

impl CanvasItem for OverlayItem {
    fn draw(&self, context: &mut CanvasContext) {
        let size = context.size();
        context.view(
            Position::new(0, 0),
            self.base.clone(),
            Some(size.width()),
            Some(size.height()),
        );
        if matches!(self.overlay, Overlay::Composer(_) | Overlay::Help(_)) {
            let style = self.theme.modal_backdrop();
            let cells = (0..size.height()).flat_map(|row| {
                let style = style.clone();
                (0..size.width()).map(move |column| {
                    PositionedCell::new(
                        Position::new(cell_i64(column), cell_i64(row)),
                        CellContribution::new().style(style.clone()),
                    )
                })
            });
            context.cells_with(cells, Composition::Overlay);
        }

        let (width_percent, requested_height) = match &self.overlay {
            Overlay::Composer(composer) => (70, usize::from(composer.height)),
            Overlay::Help(_) => (86, 32),
            Overlay::Thread(_) => (78, 18),
        };
        let width = size
            .width()
            .saturating_mul(width_percent)
            .saturating_div(100)
            .max(24)
            .min(size.width());
        let height = requested_height.min(size.height().saturating_sub(2)).max(3);
        let x = size.width().saturating_sub(width) / 2;
        let y = size.height().saturating_sub(height) / 2;
        context.view(
            Position::new(cell_i64(x), cell_i64(y)),
            popup_view(&self.overlay, width, height, self.theme),
            Some(width),
            Some(height),
        );
    }
}

fn popup_view(overlay: &Overlay, width: usize, height: usize, theme: SemanticTheme) -> View {
    match overlay {
        Overlay::Composer(composer) => {
            let editor = composer_editor(composer);
            let feedback = fixed_row(
                View::text(
                    composer
                        .message
                        .as_deref()
                        .unwrap_or(&composer.instructions),
                    theme.style(Tone::MutedResolved),
                ),
                1,
            );
            titled_panel(
                PanelSpec {
                    width: cell_u16(width),
                    height: Length::Cells(cell_u16(height)),
                    title: &composer.context,
                    focus_label: None,
                    focused: true,
                    border: Border::ROUNDED,
                },
                theme,
                View::column(Align::Left, [fill_box(editor), feedback]),
            )
        }
        Overlay::Help(help) => {
            let content = View::viewport(
                Viewport::vertical(Projection::new(
                    cell_i64(help.scroll),
                    ProjectionBoundary::Preserve,
                )),
                text_view(help_document(&help.lines, theme)),
            );
            let hint = fixed_row(
                View::text(
                    truncate_end(&help.position_hint, width.saturating_sub(2)),
                    theme.style(Tone::MutedResolved),
                ),
                1,
            );
            titled_panel(
                PanelSpec {
                    width: cell_u16(width),
                    height: Length::Cells(cell_u16(height)),
                    title: "Keyboard help",
                    focus_label: None,
                    focused: true,
                    border: Border::DOUBLE,
                },
                theme,
                View::column(Align::Left, [fill_box(content), hint]),
            )
        }
        Overlay::Thread(thread) => {
            let state = match thread.state {
                ThreadState::NeedsAttention => "NEEDS ATTENTION",
                ThreadState::Open => "OPEN",
                ThreadState::Resolved => "RESOLVED",
            };
            let outdated = if thread.outdated { " · OUTDATED" } else { "" };
            let title = format!(
                "Conversation #{} · {state}{outdated} · {}",
                thread.id, thread.position
            );
            let context = fixed_row(
                View::text(
                    truncate_end(&thread.context, width.saturating_sub(2)),
                    theme.style(Tone::MutedResolved),
                ),
                1,
            );
            let messages = thread
                .messages
                .iter()
                .enumerate()
                .flat_map(|(index, message)| {
                    let separator = (index > 0).then(|| View::text("", TextStyle::new()));
                    separator.into_iter().chain([
                        View::text(message.author.clone(), TextStyle::new().bold()),
                        View::text(message.body.clone(), TextStyle::new()),
                    ])
                });
            let actions = if thread.state == ThreadState::Resolved {
                if width.saturating_sub(2) >= 48 {
                    "t/T switch · c reply · R reopen · Esc close"
                } else {
                    "t/T · c reply · R reopen · Esc"
                }
            } else if width.saturating_sub(2) >= 48 {
                "t/T switch · c reply · x resolve · Esc close"
            } else {
                "t/T · c reply · x resolve · Esc"
            };
            titled_panel(
                PanelSpec {
                    width: cell_u16(width),
                    height: Length::Cells(cell_u16(height)),
                    title: &title,
                    focus_label: None,
                    focused: true,
                    border: Border::ROUNDED,
                },
                theme,
                View::column(
                    Align::Left,
                    [
                        context,
                        fill_box(View::column(Align::Left, messages)),
                        fixed_row(View::text(actions, theme.style(Tone::MutedResolved)), 1),
                    ],
                ),
            )
        }
    }
}

fn help_document(lines: &[String], theme: SemanticTheme) -> Document {
    Document::from(
        lines
            .iter()
            .map(|line| {
                if is_help_heading(line) {
                    return Line::styled(line.clone(), theme.style(Tone::FocusSelection));
                }
                if let Some(context) = line.strip_prefix("Commands from: ") {
                    return Line::from(vec![
                        Span::styled("Commands from: ", theme.style(Tone::MutedResolved)),
                        Span::styled(context, theme.style(Tone::Attention)),
                    ]);
                }

                let (available, command) = if let Some(command) = line.strip_prefix("◆ ") {
                    (true, command)
                } else if let Some(command) = line.strip_prefix("· ") {
                    (false, command)
                } else {
                    return Line::raw(line.clone());
                };
                let marker_style = if available {
                    theme.style(Tone::FocusSelection)
                } else {
                    theme.style(Tone::MutedResolved)
                };
                let Some((keys, description)) = command.split_once(" — ") else {
                    return Line::from(vec![
                        Span::styled(if available { "◆ " } else { "· " }, marker_style),
                        Span::styled(command, theme.style(Tone::MutedResolved)),
                    ]);
                };
                let description_style = if available {
                    TextStyle::new()
                } else {
                    theme.style(Tone::MutedResolved)
                };
                Line::from(vec![
                    Span::styled(if available { "◆ " } else { "· " }, marker_style.clone()),
                    Span::styled(keys, marker_style),
                    Span::styled("  ", description_style.clone()),
                    Span::styled(description, description_style),
                ])
            })
            .collect::<Vec<_>>(),
    )
}

fn is_help_heading(line: &str) -> bool {
    matches!(
        line,
        "Navigation" | "View" | "Review actions" | "Global / exit" | "In help"
    )
}

fn composer_editor(composer: &crate::semantic::ComposerOverlay) -> View {
    let lines = composer.lines.iter().enumerate().map(|(row, line)| {
        let content = if row == composer.cursor_row {
            let split = byte_at_column(line, composer.cursor_column);
            View::row(
                VerticalAlign::Top,
                [
                    View::text(&line[..split], TextStyle::new()),
                    View::anchor("composer-cursor"),
                    View::text(&line[split..], TextStyle::new()),
                ],
            )
        } else {
            View::text(line.clone(), TextStyle::new())
        };
        fixed_row(content, 1)
    });
    View::viewport(
        Viewport::vertical(Projection::new(
            cell_i64(composer.scroll),
            ProjectionBoundary::Preserve,
        )),
        View::column(Align::Left, lines),
    )
}

struct PanelSpec<'a> {
    width: u16,
    height: Length,
    title: &'a str,
    focus_label: Option<&'a str>,
    focused: bool,
    border: Border,
}

#[cfg(test)]
fn file_list_label(row: &FileRailRow, width: usize) -> String {
    const INDENT: usize = 2;
    let indentation = " ".repeat(row.depth.saturating_mul(INDENT));
    let available = width.saturating_sub(UnicodeWidthStr::width(indentation.as_str()));
    let (prefix, label) = match row.kind {
        FileRailRowKind::Directory { collapsed } => (
            format!("{} {} ", if collapsed { "▶" } else { "▼" }, symbols::FOLDER),
            row.label.clone(),
        ),
        FileRailRowKind::File { ref change, .. } => (
            format!(
                " {} {} ",
                file_change_icon(change),
                symbols::file_type(&row.path).glyph
            ),
            row.label.clone(),
        ),
    };
    let label_width = available.saturating_sub(UnicodeWidthStr::width(prefix.as_str()));
    format!("{indentation}{prefix}{}", truncate_end(&label, label_width))
}

fn file_list_line(row: &FileRailRow, width: usize, selected: bool, theme: SemanticTheme) -> Line {
    const INDENT: usize = 2;
    let indentation = " ".repeat(row.depth.saturating_mul(INDENT));
    let available = width.saturating_sub(UnicodeWidthStr::width(indentation.as_str()));
    let selection = selected.then(|| theme.file_selection());
    let styled = |base: TextStyle| match &selection {
        Some(overlay) => patch_style(base, overlay),
        None => base,
    };
    let plain = styled(TextStyle::new());

    let mut spans = vec![Span::styled(indentation, plain.clone())];
    match row.kind {
        FileRailRowKind::Directory { collapsed } => {
            let prefix = format!("{} {} ", if collapsed { "▶" } else { "▼" }, symbols::FOLDER);
            let label_width = available.saturating_sub(UnicodeWidthStr::width(prefix.as_str()));
            spans.extend([
                Span::styled(if collapsed { "▶ " } else { "▼ " }, plain.clone()),
                Span::styled(
                    symbols::FOLDER,
                    styled(theme.file_icon(urushi::Color::Rgb(0x87, 0x87, 0x87))),
                ),
                Span::styled(" ", plain.clone()),
                Span::styled(truncate_end(&row.label, label_width), plain),
            ]);
        }
        FileRailRowKind::File { ref change, .. } => {
            let icon = symbols::file_type(&row.path);
            let prefix = format!(" {} {} ", file_change_icon(change), icon.glyph);
            let label_width = available.saturating_sub(UnicodeWidthStr::width(prefix.as_str()));
            spans.extend([
                Span::styled(" ", plain.clone()),
                Span::styled(file_change_icon(change), styled(theme.file_status())),
                Span::styled(" ", plain.clone()),
                Span::styled(icon.glyph, styled(theme.file_icon(icon.color))),
                Span::styled(" ", plain.clone()),
                Span::styled(truncate_end(&row.label, label_width), plain),
            ]);
        }
    }
    Line::from(spans)
}

fn file_change_icon(change: &crate::diff::FileChange) -> &'static str {
    if let Some(moved) = &change.moved {
        return if moved.copied {
            symbols::CHANGE_COPIED
        } else {
            symbols::CHANGE_RENAMED
        };
    }
    match change.status {
        FileStatus::Modified => symbols::CHANGE_BOTH,
        FileStatus::Added => symbols::CHANGE_ADDED,
        FileStatus::Deleted => symbols::CHANGE_REMOVED,
    }
}

fn titled_panel(spec: PanelSpec<'_>, theme: SemanticTheme, content: View) -> View {
    let tone = if spec.focused {
        Tone::FocusSelection
    } else {
        Tone::MutedResolved
    };
    let border_style = theme.border(tone);
    let border = if spec.focused && spec.border == Border::ROUNDED {
        Border::THICK
    } else {
        spec.border
    };
    let mut style = BlockStyle::new()
        .border(border)
        .height(spec.height)
        .overflow(Overflow::clip());
    if spec.width > 0 {
        style = style.width(Length::Cells(spec.width));
    } else {
        style = style.width(Length::fill(1));
    }
    if let Some(color) = border_style.get_foreground() {
        style = style.border_foreground(color);
    }
    let title = match spec.focus_label {
        Some(label) => format!("{}  {label}", spec.title),
        None => spec.title.to_owned(),
    };
    View::titled_block(
        style,
        BlockTitle::new(StyledText::new(title, border_style)),
        content,
    )
}

fn too_small_view(columns: usize, rows: usize) -> View {
    View::block(
        BlockStyle::new()
            .width(Length::fill(1))
            .height(Length::fill(1)),
        View::text(
            format!(
                "Terminal is too small for revia ({columns}×{rows}).\nResize to at least 48×8, then continue.\nPress q to quit."
            ),
            TextStyle::new(),
        ),
    )
}

fn text_view(text: Document) -> View {
    View::styled_text(text.into_styled_text())
}

fn fixed_row(view: View, height: u16) -> View {
    View::block(
        BlockStyle::new()
            .width(Length::fill(1))
            .height(Length::Cells(height))
            .overflow(Overflow::clip()),
        view,
    )
}

fn fill_box(view: View) -> View {
    View::block(
        BlockStyle::new()
            .width(Length::fill(1))
            .height(Length::fill(1))
            .overflow(Overflow::clip()),
        view,
    )
}

fn byte_at_column(value: &str, column: usize) -> usize {
    let mut used = 0;
    for (offset, grapheme) in value.grapheme_indices(true) {
        if used >= column {
            return offset;
        }
        used = used.saturating_add(UnicodeWidthStr::width(grapheme));
    }
    value.len()
}

fn cell_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn cell_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use urushi::{Available, RenderSettings, TextAttribute, render, resolve};

    use super::*;
    use crate::{
        diff::{FileChange, Magnitude},
        semantic::{
            ContextualKeys, CurrentContext, FileAttention, FileRail, FileRailRow, FileRailRowKind,
            Footer, Header, LayoutPolicy, ReviewViewport,
        },
        ui::LayoutMode,
    };

    #[test]
    fn too_small_view_uses_the_complete_surface() {
        let view = too_small_view(40, 7);
        let resolved = resolve(&view, Available::size(40, 7)).unwrap();

        assert_eq!(resolved.size().width(), 40);
        assert_eq!(resolved.size().height(), 7);
        assert!(render(&resolved, &RenderSettings::default()).contains("40×7"));
    }

    #[test]
    fn composer_cursor_split_uses_terminal_columns() {
        assert_eq!(byte_at_column("a界b", 1), 1);
        assert_eq!(byte_at_column("a界b", 3), "a界".len());
    }

    #[test]
    fn help_document_distinguishes_headings_keys_and_unavailable_commands() {
        let document = help_document(
            &[
                "Navigation".into(),
                "◆ j / k — Move by row".into(),
                "· n / N — Move between matches".into(),
            ],
            SemanticTheme::no_color(),
        );

        assert!(
            document.lines[0].spans[0]
                .style
                .get_attributes()
                .contains(TextAttribute::Bold)
        );
        assert_eq!(document.lines[1].spans[1].content, "j / k");
        assert!(
            document.lines[1].spans[1]
                .style
                .get_attributes()
                .contains(TextAttribute::Bold)
        );
        assert!(
            document.lines[2].spans[3]
                .style
                .get_attributes()
                .contains(TextAttribute::Dim)
        );
    }

    #[test]
    fn complete_shell_resolves_through_urushi() {
        let semantic = SemanticView {
            header: Header {
                comparison: "working tree".into(),
                file_count: 1,
                magnitude: Magnitude {
                    additions: 2,
                    deletions: 1,
                },
                active_filter: "all".into(),
            },
            file_rail: Some(FileRail {
                selected: Some(1),
                tree: true,
                rows: vec![
                    FileRailRow {
                        label: "src".into(),
                        path: "src".into(),
                        depth: 0,
                        kind: FileRailRowKind::Directory { collapsed: false },
                    },
                    FileRailRow {
                        label: "main.rs".into(),
                        path: "src/main.rs".into(),
                        depth: 1,
                        kind: FileRailRowKind::File {
                            change: FileChange::default(),
                            attention: FileAttention::Open,
                        },
                    },
                ],
            }),
            body: Body::Review(Box::new(ReviewBody {
                scroll: 0,
                viewport: ReviewViewport {
                    presentation_width: 89,
                    total_rows: 1,
                    visible_rows: 20,
                    sticky_context: None,
                    sections: Arc::new(Vec::new()),
                },
                empty_state: Some("Nothing to review".into()),
                search_target: None,
                search_query: None,
                files: Vec::new(),
            })),
            footer: Footer {
                current_context: CurrentContext {
                    text: "Review".into(),
                },
                contextual_keys: ContextualKeys {
                    text: "q quit".into(),
                },
            },
            overlay: None,
            layout: LayoutPolicy {
                focus: FocusArea::Review,
                diff_layout: LayoutMode::Auto,
                wrap_lines: false,
            },
        };

        let view = UrushiRenderer::default().view(&semantic, SurfaceSize::new(120, 24));
        let resolved = resolve(&view, Available::size(120, 24)).unwrap();
        let text = render(&resolved, &RenderSettings::default());

        assert_eq!(resolved.size().width(), 120);
        assert_eq!(resolved.size().height(), 24);
        assert!(text.contains("working tree"));
        assert!(text.contains("▼  src"));
        assert!(text.contains("main.rs"));
        assert!(text.contains("Nothing to review"));
        assert!(text.contains("Keys: q quit"));
    }

    #[test]
    fn file_list_labels_distinguish_open_and_collapsed_directories() {
        let open = FileRailRow {
            label: "src".into(),
            path: "src".into(),
            depth: 1,
            kind: FileRailRowKind::Directory { collapsed: false },
        };
        let collapsed = FileRailRow {
            kind: FileRailRowKind::Directory { collapsed: true },
            ..open.clone()
        };

        assert_eq!(file_list_label(&open, 20), "  ▼  src");
        assert_eq!(file_list_label(&collapsed, 20), "  ▶  src");
    }

    #[test]
    fn file_and_directory_rows_match_lazygit_columns() {
        let directory = FileRailRow {
            label: "src".into(),
            path: "src".into(),
            depth: 1,
            kind: FileRailRowKind::Directory { collapsed: false },
        };
        let file = FileRailRow {
            label: "main.rs".into(),
            path: "src/main.rs".into(),
            depth: 1,
            kind: FileRailRowKind::File {
                change: FileChange::default(),
                attention: FileAttention::None,
            },
        };
        let directory = file_list_label(&directory, 30);
        let file = file_list_label(&file, 30);

        let directory_name = directory.find("src").expect("directory name");
        let file_name = file.find("main.rs").expect("file name");
        assert_eq!(
            UnicodeWidthStr::width(&directory[..directory_name]) + 1,
            UnicodeWidthStr::width(&file[..file_name])
        );
        assert!(file.trim_start().starts_with(symbols::CHANGE_BOTH));
        assert!(file.contains(" main.rs"));
    }

    #[test]
    fn file_rows_color_status_icon_and_name_independently() {
        let row = FileRailRow {
            label: "main.rs".into(),
            path: "src/main.rs".into(),
            depth: 1,
            kind: FileRailRowKind::File {
                change: FileChange::default(),
                attention: FileAttention::None,
            },
        };
        let theme = SemanticTheme::from_no_color(None);
        let line = file_list_line(&row, 30, false, theme);

        assert_eq!(line.spans[2].content, symbols::CHANGE_BOTH);
        assert_eq!(
            line.spans[2].style.get_foreground(),
            Some(urushi::Color::RED)
        );
        assert_eq!(line.spans[4].content, "");
        assert_eq!(
            line.spans[4].style.get_foreground(),
            Some(urushi::Color::Rgb(0xff, 0x70, 0x43))
        );
        assert_eq!(line.spans[6].content, "main.rs");
        assert_eq!(line.spans[6].style.get_foreground(), None);
    }

    #[test]
    fn selected_file_keeps_element_colors_and_adds_the_same_emphasis() {
        let row = FileRailRow {
            label: "README.md".into(),
            path: "README.md".into(),
            depth: 0,
            kind: FileRailRowKind::File {
                change: FileChange::default(),
                attention: FileAttention::None,
            },
        };
        let line = file_list_line(&row, 30, true, SemanticTheme::from_no_color(None));

        assert_eq!(
            line.spans[2].style.get_foreground(),
            Some(urushi::Color::RED)
        );
        assert_eq!(
            line.spans[4].style.get_foreground(),
            Some(urushi::Color::Rgb(0x42, 0xa5, 0xf5))
        );
        assert!(
            line.spans
                .iter()
                .all(|span| { span.style.get_attributes().contains(TextAttribute::Bold) })
        );
    }

    #[test]
    fn composer_cursor_is_an_urushi_anchor_at_the_display_column() {
        let editor = composer_editor(&crate::semantic::ComposerOverlay {
            context: "Comment".into(),
            lines: vec!["a界b".into()],
            cursor_row: 0,
            cursor_column: 3,
            scroll: 0,
            height: 5,
            message: None,
            instructions: "submit".into(),
        });

        let resolved = resolve(&editor, Available::size(20, 4)).unwrap();
        let cursor = resolved.anchor("composer-cursor").unwrap();

        assert_eq!(cursor.x(), 3);
        assert_eq!(cursor.y(), 0);
    }
}
