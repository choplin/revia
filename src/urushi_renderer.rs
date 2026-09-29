//! Urushi view construction for Revia's semantic UI.
//!
//! Layout, panels, viewports, scrollbars, overlays, cursor placement, and text
//! decoration are native Urushi values.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use urushi::{
    Align, BlockStyle, BlockTitle, Border, Canvas, CanvasContext, CanvasItem, CellContribution,
    Composition, Length, List, ListItem, ListItemPresentation, ListPosition, ListPresentation,
    Overflow, Position, PositionedCell, Projection, ProjectionBoundary, Scrollbar, ScrollbarGlyphs,
    ScrollbarOrientation, ScrollbarPresentation, StyledText, TextStyle, VerticalAlign, View,
    Viewport,
};
use urushi_tui_app::SurfaceSize;

use crate::{
    presentation,
    renderer::{self, Renderer, SemanticTheme},
    semantic::{
        Body, FileAttention, Overlay, ReviewBody, RollupBody, ThreadState, Tone,
        View as SemanticView,
    },
    styled_text::{Document, Line, Span},
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
                    self.file_rail_view(semantic, rail, rail_width, theme),
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
        theme: SemanticTheme,
    ) -> View {
        let inner_width = usize::from(width.saturating_sub(2));
        let focused = semantic.layout.focus == FocusArea::Files;
        let list = file_list(rail);
        let items = ListItemPresentation::new(|row: &FileListRow, position| {
            file_list_label(row, position, inner_width)
        })
        .item_style(|row, _, _| {
            Some(match &row.kind {
                FileListKind::Directory => theme.directory(),
                FileListKind::File {
                    attention,
                    selected,
                    ..
                } => theme.file_item(*attention, *selected, focused),
            })
        });
        let content = ListPresentation::new(TextStyle::new(), TextStyle::new())
            .enumerator(file_list_enumerator)
            .nesting_indent(FILE_LIST_INDENT)
            .compose_with(&list, &items);
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

const FILE_LIST_INDENT: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileListRow {
    label: String,
    kind: FileListKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileListKind {
    Directory,
    File {
        magnitude: crate::diff::Magnitude,
        attention: FileAttention,
        selected: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileListBranch {
    row: FileListRow,
    children: Vec<FileListBranch>,
}

fn file_list(rail: &crate::semantic::FileRail) -> List<FileListRow> {
    let mut roots = Vec::new();
    for (index, file) in rail.items.iter().enumerate() {
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
        insert_file_branch(&mut roots, &components, file, rail.selected == Some(index));
    }
    List::new().items(roots.into_iter().map(file_list_item))
}

fn insert_file_branch(
    branches: &mut Vec<FileListBranch>,
    components: &[&str],
    file: &crate::semantic::FileItem,
    selected: bool,
) {
    let Some((component, remaining)) = components.split_first() else {
        return;
    };
    if remaining.is_empty() {
        branches.push(FileListBranch {
            row: FileListRow {
                label: (*component).to_owned(),
                kind: FileListKind::File {
                    magnitude: file.magnitude,
                    attention: file.attention,
                    selected,
                },
            },
            children: Vec::new(),
        });
        return;
    }

    let directory = branches
        .iter()
        .position(|branch| {
            branch.row.label == *component && matches!(branch.row.kind, FileListKind::Directory)
        })
        .unwrap_or_else(|| {
            branches.push(FileListBranch {
                row: FileListRow {
                    label: (*component).to_owned(),
                    kind: FileListKind::Directory,
                },
                children: Vec::new(),
            });
            branches.len() - 1
        });
    insert_file_branch(&mut branches[directory].children, remaining, file, selected);
}

fn file_list_item(branch: FileListBranch) -> ListItem<FileListRow> {
    ListItem::new(branch.row).items(branch.children.into_iter().map(file_list_item))
}

fn file_list_label(row: &FileListRow, position: ListPosition, width: usize) -> String {
    let available = width.saturating_sub(position.depth().saturating_mul(FILE_LIST_INDENT));
    match &row.kind {
        FileListKind::Directory => truncate_end(&format!("▾ {}/", row.label), available),
        FileListKind::File {
            magnitude,
            attention,
            selected,
        } => {
            let marker = match attention {
                FileAttention::NeedsAttention => symbols::NEEDS_ATTENTION,
                FileAttention::Open => symbols::OPEN,
                FileAttention::Resolved => symbols::RESOLVED,
                FileAttention::None => " ",
            };
            let selection = if *selected { symbols::NEXT } else { " " };
            let change = presentation::change_direction(*magnitude);
            let prefix = format!("{selection} {marker} ");
            let label_width = available
                .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
                .saturating_sub(UnicodeWidthStr::width(change))
                .saturating_sub(2);
            presentation::right_aligned_row(
                &prefix,
                &truncate_end(&row.label, label_width),
                change,
                available,
            )
        }
    }
}

fn file_list_enumerator(_: ListPosition) -> String {
    String::new()
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
        diff::Magnitude,
        semantic::{
            ContextualKeys, CurrentContext, FileItem, FileRail, Footer, Header, LayoutPolicy,
            ReviewViewport,
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
                selected: Some(0),
                items: vec![FileItem {
                    path: "src/main.rs".into(),
                    magnitude: Magnitude {
                        additions: 2,
                        deletions: 1,
                    },
                    attention: FileAttention::Open,
                }],
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
        assert!(text.contains("▾ src/"));
        assert!(text.contains("main.rs"));
        assert!(text.contains("Nothing to review"));
        assert!(text.contains("Keys: q quit"));
    }

    #[test]
    fn file_list_indents_shared_directories_without_changing_file_order() {
        let rail = FileRail {
            selected: Some(1),
            items: vec![
                FileItem {
                    path: "src/app/mod.rs".into(),
                    magnitude: Magnitude::default(),
                    attention: FileAttention::None,
                },
                FileItem {
                    path: "src/app/view.rs".into(),
                    magnitude: Magnitude::default(),
                    attention: FileAttention::Open,
                },
                FileItem {
                    path: "README.md".into(),
                    magnitude: Magnitude::default(),
                    attention: FileAttention::None,
                },
            ],
        };

        let list = file_list(&rail);
        assert_eq!(list.item_nodes().len(), 2);
        assert_eq!(list.item_nodes()[0].value().label, "src");
        let app = &list.item_nodes()[0].item_nodes()[0];
        assert_eq!(app.value().label, "app");
        assert_eq!(app.item_nodes().len(), 2);
        assert_eq!(app.item_nodes()[0].value().label, "mod.rs");
        assert_eq!(app.item_nodes()[1].value().label, "view.rs");
        assert!(matches!(
            app.item_nodes()[1].value().kind,
            FileListKind::File { selected: true, .. }
        ));
        assert_eq!(list.item_nodes()[1].value().label, "README.md");
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
