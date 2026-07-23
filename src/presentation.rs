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

use crate::diff::{DiffLine, DiffLineKind};

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
) -> Vec<Line<'static>> {
    let column_width = usize::from(available_width.saturating_sub(5) / 2).max(12);
    split_rows(lines)
        .into_iter()
        .map(|row| {
            let old = split_cell(row.old.as_ref(), '-', column_width, selected);
            let new = split_cell(row.new.as_ref(), '+', column_width, selected);
            Line::from(vec![
                old,
                Span::styled(" │ ", selected_style(selected)),
                new,
            ])
        })
        .collect()
}

pub(crate) fn highlight_line(
    line: &DiffLine,
    highlighter: &mut HighlightLines<'_>,
    syntax_set: &SyntaxSet,
) -> Line<'static> {
    let marker = match line.kind {
        DiffLineKind::Added => "+",
        DiffLineKind::Removed => "-",
        DiffLineKind::Context => " ",
        DiffLineKind::Meta => "\\",
    };
    let background = match line.kind {
        DiffLineKind::Added => Some(Color::Rgb(24, 54, 35)),
        DiffLineKind::Removed => Some(Color::Rgb(65, 29, 34)),
        DiffLineKind::Context | DiffLineKind::Meta => None,
    };
    let base = background.map_or_else(Style::default, |background| Style::default().bg(background));
    let mut spans = vec![Span::styled(marker, base.fg(marker_color(line.kind)))];

    match highlighter.highlight_line(&line.text, syntax_set) {
        Ok(ranges) => {
            spans.extend(ranges.into_iter().map(|(style, text)| {
                Span::styled(text.to_owned(), merge_syntect_style(base, style))
            }))
        }
        Err(_) => spans.push(Span::styled(line.text.clone(), base)),
    }
    Line::from(spans)
}

fn split_cell(
    line: Option<&DiffLine>,
    marker: char,
    width: usize,
    selected: bool,
) -> Span<'static> {
    let (prefix, text, color) = match line {
        Some(line) => {
            let prefix = match line.kind {
                DiffLineKind::Added => '+',
                DiffLineKind::Removed => '-',
                DiffLineKind::Context => ' ',
                DiffLineKind::Meta => '\\',
            };
            (prefix, line.text.as_str(), marker_color(line.kind))
        }
        None => (' ', "", Color::DarkGray),
    };
    let clipped = if text.chars().count() > width.saturating_sub(2) {
        format!(
            "{}…",
            text.chars()
                .take(width.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        text.to_owned()
    };
    Span::styled(
        format!(
            "{prefix}{marker} {:width$}",
            clipped,
            width = width.saturating_sub(2)
        ),
        selected_style(selected).fg(color),
    )
}

fn selected_style(selected: bool) -> Style {
    if selected {
        Style::default().bg(Color::Rgb(42, 52, 74))
    } else {
        Style::default()
    }
}

fn marker_color(kind: DiffLineKind) -> Color {
    match kind {
        DiffLineKind::Added => Color::Green,
        DiffLineKind::Removed => Color::Red,
        DiffLineKind::Context => Color::DarkGray,
        DiffLineKind::Meta => Color::Yellow,
    }
}

fn merge_syntect_style(base: Style, source: SyntectStyle) -> Style {
    let foreground = source.foreground;
    base.fg(Color::Rgb(foreground.r, foreground.g, foreground.b))
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
