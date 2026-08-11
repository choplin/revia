//! Pure diff-to-terminal presentation transforms.
//!
//! The renderer owns widget layout. This module owns only the conversion from
//! Git patch lines to styled terminal rows, so split pairing and syntax styles
//! are testable without an interactive review session.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use syntect::{easy::HighlightLines, highlighting::Style as SyntectStyle, parsing::SyntaxSet};

use crate::{
    diff::{DiffLine, DiffLineKind},
    renderer::SemanticTheme,
    semantic::Tone,
    ui::fit_width,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SplitRow {
    pub(crate) old: Option<DiffLine>,
    pub(crate) new: Option<DiffLine>,
}

/// Pair adjacent removed and added runs for side-by-side presentation without
/// claiming that Git supplied an exact line-level correspondence.
pub(crate) fn split_rows(lines: &[DiffLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        match lines[index].kind {
            DiffLineKind::Removed | DiffLineKind::Added => {
                let mut removed = Vec::new();
                while index < lines.len() && lines[index].kind == DiffLineKind::Removed {
                    removed.push(lines[index].clone());
                    index += 1;
                }
                let mut added = Vec::new();
                while index < lines.len() && lines[index].kind == DiffLineKind::Added {
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
            _ => {
                let line = lines[index].clone();
                rows.push(SplitRow {
                    old: Some(line.clone()),
                    new: Some(line),
                });
                index += 1;
            }
        }
    }
    rows
}

pub(crate) fn split_hunk_lines(
    lines: &[DiffLine],
    available_width: u16,
    selected: bool,
    theme: SemanticTheme,
) -> Vec<Line<'static>> {
    let column_width = usize::from(available_width.saturating_sub(5) / 2).max(12);
    split_rows(lines)
        .into_iter()
        .map(|row| {
            let old = split_cell(row.old.as_ref(), '-', column_width, selected, theme);
            let new = split_cell(row.new.as_ref(), '+', column_width, selected, theme);
            Line::from(vec![
                old,
                Span::styled(" │ ", selected_style(selected, theme)),
                new,
            ])
        })
        .collect()
}

pub(crate) fn highlight_line(
    line: &DiffLine,
    highlighter: &mut HighlightLines<'_>,
    syntax_set: &SyntaxSet,
    selected: bool,
    theme: SemanticTheme,
) -> Line<'static> {
    let marker = match line.kind {
        DiffLineKind::Added => "+",
        DiffLineKind::Removed => "-",
        DiffLineKind::Context => " ",
        DiffLineKind::Meta => "\\",
    };
    let base = if selected {
        theme.selection().patch(theme.style(line_tone(line.kind)))
    } else {
        theme.style(line_tone(line.kind))
    };
    let mut spans = vec![Span::styled(marker, base)];

    match (
        theme.colors_enabled(),
        highlighter.highlight_line(&line.text, syntax_set),
    ) {
        (true, Ok(ranges)) => {
            spans.extend(ranges.into_iter().map(|(style, text)| {
                Span::styled(text.to_owned(), merge_syntect_style(base, style))
            }))
        }
        _ => spans.push(Span::styled(line.text.clone(), base)),
    }
    Line::from(spans)
}

fn split_cell(
    line: Option<&DiffLine>,
    marker: char,
    width: usize,
    selected: bool,
    theme: SemanticTheme,
) -> Span<'static> {
    let (prefix, text, tone) = match line {
        Some(line) => {
            let prefix = match line.kind {
                DiffLineKind::Added => '+',
                DiffLineKind::Removed => '-',
                DiffLineKind::Context => ' ',
                DiffLineKind::Meta => '\\',
            };
            (prefix, line.text.as_str(), line_tone(line.kind))
        }
        None => (' ', "", Tone::MutedResolved),
    };
    let fitted = fit_width(text, width.saturating_sub(2));
    Span::styled(
        format!("{prefix}{marker} {fitted}"),
        selected_style(selected, theme).patch(theme.style(tone)),
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
    use super::split_rows;
    use crate::diff::{DiffLine, DiffLineKind};

    #[test]
    fn pairs_replacement_runs_and_retains_one_sided_rows() {
        let rows = split_rows(&[
            DiffLine {
                kind: DiffLineKind::Removed,
                text: "old one".into(),
            },
            DiffLine {
                kind: DiffLineKind::Removed,
                text: "old two".into(),
            },
            DiffLine {
                kind: DiffLineKind::Added,
                text: "new one".into(),
            },
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].old.as_ref().unwrap().text, "old one");
        assert_eq!(rows[0].new.as_ref().unwrap().text, "new one");
        assert_eq!(rows[1].old.as_ref().unwrap().text, "old two");
        assert!(rows[1].new.is_none());
    }
}
