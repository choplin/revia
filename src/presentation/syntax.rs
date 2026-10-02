use std::{cell::RefCell, ops::Range};

use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, ThemeSet},
    parsing::SyntaxSet,
};
use tree_sitter_highlight::{
    HighlightConfiguration, HighlightEvent, Highlighter as TreeSitterHighlighter,
};

use crate::domain::diff::{PatchLine, PatchLineKind};

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "comment.documentation",
    "constant",
    "constant.builtin",
    "constructor",
    "escape",
    "function",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenStyle {
    pub(crate) foreground: (u8, u8, u8),
    pub(crate) bold: bool,
    pub(crate) italic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StyledRange {
    pub(crate) bytes: Range<usize>,
    pub(crate) style: TokenStyle,
}

pub(crate) type SyntaxLine = Vec<StyledRange>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HunkSyntax {
    old: Vec<Option<SyntaxLine>>,
    new: Vec<Option<SyntaxLine>>,
}

impl HunkSyntax {
    pub(crate) fn old(&self, source_index: usize) -> Option<&SyntaxLine> {
        self.old.get(source_index).and_then(Option::as_ref)
    }

    pub(crate) fn new_side(&self, source_index: usize) -> Option<&SyntaxLine> {
        self.new.get(source_index).and_then(Option::as_ref)
    }
}

pub(crate) struct SyntaxHighlighter {
    syntax_set: SyntaxSet,
    syntect_theme: syntect::highlighting::Theme,
    tree_sitter_languages: Vec<TreeSitterLanguage>,
    tree_sitter: RefCell<TreeSitterHighlighter>,
}

struct TreeSitterLanguage {
    extensions: &'static [&'static str],
    configuration: HighlightConfiguration,
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let themes = ThemeSet::load_defaults();
        let syntect_theme = themes
            .themes
            .get("base16-ocean.dark")
            .or_else(|| themes.themes.values().next())
            .expect("syntect includes a default theme")
            .clone();
        Self {
            syntax_set,
            syntect_theme,
            tree_sitter_languages: tree_sitter_languages(),
            tree_sitter: RefCell::new(TreeSitterHighlighter::new()),
        }
    }
}

impl SyntaxHighlighter {
    pub(crate) fn highlight_hunk(
        &self,
        extension: Option<&str>,
        lines: &[PatchLine],
    ) -> HunkSyntax {
        if let Some(configuration) = self.tree_sitter_configuration(extension) {
            HunkSyntax {
                old: self.highlight_tree_sitter_side(configuration, lines, SourceSide::Old),
                new: self.highlight_tree_sitter_side(configuration, lines, SourceSide::New),
            }
        } else {
            HunkSyntax {
                old: self.highlight_syntect_side(extension, lines, SourceSide::Old),
                new: self.highlight_syntect_side(extension, lines, SourceSide::New),
            }
        }
    }

    fn highlight_tree_sitter_side(
        &self,
        configuration: &HighlightConfiguration,
        lines: &[PatchLine],
        side: SourceSide,
    ) -> Vec<Option<SyntaxLine>> {
        let mut source = String::new();
        let mut offsets = vec![None; lines.len()];
        for (index, line) in lines.iter().enumerate() {
            if !side.includes(line.kind) {
                continue;
            }
            let start = source.len();
            source.push_str(&expand_tabs(&line.text));
            let end = source.len();
            source.push('\n');
            offsets[index] = Some(start..end);
        }
        let mut byte_styles = vec![None; source.len()];
        let mut highlighter = self.tree_sitter.borrow_mut();
        let Ok(events) = highlighter.highlight(configuration, source.as_bytes(), None, |_| None)
        else {
            return vec![None; lines.len()];
        };
        let mut stack = Vec::new();
        for event in events.flatten() {
            match event {
                HighlightEvent::HighlightStart(highlight) => stack.push(highlight.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    let Some(capture) = stack.last().copied() else {
                        continue;
                    };
                    let style = capture_style(HIGHLIGHT_NAMES[capture]);
                    for byte in start..end.min(byte_styles.len()) {
                        byte_styles[byte] = Some(style);
                    }
                }
            }
        }
        offsets
            .into_iter()
            .map(|offset| offset.map(|range| collect_ranges(&byte_styles[range])))
            .collect()
    }

    fn tree_sitter_configuration(
        &self,
        extension: Option<&str>,
    ) -> Option<&HighlightConfiguration> {
        let extension = extension?.trim_start_matches('.');
        self.tree_sitter_languages
            .iter()
            .find(|language| {
                language
                    .extensions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
            })
            .map(|language| &language.configuration)
    }

    fn highlight_syntect_side(
        &self,
        extension: Option<&str>,
        lines: &[PatchLine],
        side: SourceSide,
    ) -> Vec<Option<SyntaxLine>> {
        let syntax = extension
            .and_then(|extension| self.syntax_set.find_syntax_by_extension(extension))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(syntax, &self.syntect_theme);
        lines
            .iter()
            .map(|line| {
                side.includes(line.kind).then(|| {
                    let expanded = expand_tabs(&line.text);
                    let Ok(highlighted) = highlighter.highlight_line(&expanded, &self.syntax_set)
                    else {
                        return Vec::new();
                    };
                    let mut cursor = 0usize;
                    highlighted
                        .into_iter()
                        .map(|(style, text)| {
                            let start = cursor;
                            cursor = cursor.saturating_add(text.len());
                            StyledRange {
                                bytes: start..cursor,
                                style: TokenStyle {
                                    foreground: (
                                        style.foreground.r,
                                        style.foreground.g,
                                        style.foreground.b,
                                    ),
                                    bold: style.font_style.contains(FontStyle::BOLD),
                                    italic: style.font_style.contains(FontStyle::ITALIC),
                                },
                            }
                        })
                        .collect()
                })
            })
            .collect()
    }
}

fn tree_sitter_languages() -> Vec<TreeSitterLanguage> {
    vec![
        tree_sitter_language(
            &["rs"],
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        tree_sitter_language(
            &["js", "mjs", "cjs"],
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        tree_sitter_language(
            &["jsx"],
            tree_sitter_javascript::LANGUAGE.into(),
            "jsx",
            &format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        tree_sitter_language(
            &["ts", "mts", "cts"],
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        tree_sitter_language(
            &["tsx"],
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "tsx",
            &format!(
                "{}\n{}",
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        tree_sitter_language(
            &["py", "pyi"],
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        tree_sitter_language(
            &["go"],
            tree_sitter_go::LANGUAGE.into(),
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        tree_sitter_language(
            &["json"],
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    ]
}

fn tree_sitter_language(
    extensions: &'static [&'static str],
    language: tree_sitter::Language,
    name: &'static str,
    highlights_query: &str,
    injections_query: &str,
    locals_query: &str,
) -> TreeSitterLanguage {
    let mut configuration = HighlightConfiguration::new(
        language,
        name,
        highlights_query,
        injections_query,
        locals_query,
    )
    .unwrap_or_else(|error| panic!("bundled {name} highlight query must compile: {error}"));
    configuration.configure(HIGHLIGHT_NAMES);
    TreeSitterLanguage {
        extensions,
        configuration,
    }
}

#[derive(Debug, Clone, Copy)]
enum SourceSide {
    Old,
    New,
}

impl SourceSide {
    fn includes(self, kind: PatchLineKind) -> bool {
        match self {
            Self::Old => matches!(kind, PatchLineKind::Removed | PatchLineKind::Context),
            Self::New => matches!(kind, PatchLineKind::Added | PatchLineKind::Context),
        }
    }
}

fn collect_ranges(styles: &[Option<TokenStyle>]) -> SyntaxLine {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    while start < styles.len() {
        let Some(style) = styles[start] else {
            start += 1;
            continue;
        };
        let mut end = start + 1;
        while end < styles.len() && styles[end] == Some(style) {
            end += 1;
        }
        ranges.push(StyledRange {
            bytes: start..end,
            style,
        });
        start = end;
    }
    debug_assert!(ranges.iter().all(|range| range.bytes.end <= styles.len()));
    ranges
}

fn capture_style(name: &str) -> TokenStyle {
    let (foreground, bold, italic) = if name.starts_with("comment") {
        ((101, 115, 126), false, true)
    } else if name.starts_with("string") {
        ((163, 190, 140), false, false)
    } else if name.starts_with("keyword") {
        ((198, 149, 198), true, false)
    } else if name.starts_with("function.macro") {
        ((220, 220, 170), true, false)
    } else if name.starts_with("function") {
        ((97, 175, 239), false, false)
    } else if name.starts_with("punctuation.bracket") {
        ((143, 161, 190), false, false)
    } else if name.starts_with("type") || name == "constructor" {
        ((78, 201, 176), false, false)
    } else if name.starts_with("constant") || name == "number" {
        ((209, 154, 102), false, false)
    } else if name == "operator" || name.starts_with("punctuation") {
        ((86, 182, 194), false, false)
    } else if name == "attribute" || name == "label" {
        ((229, 192, 123), false, false)
    } else {
        ((192, 197, 206), false, false)
    };
    TokenStyle {
        foreground,
        bold,
        italic,
    }
}

fn expand_tabs(value: &str) -> String {
    let mut expanded = String::new();
    let mut column = 0usize;
    for character in value.chars() {
        if character == '\t' {
            let spaces = 4 - column % 4;
            expanded.push_str(&" ".repeat(spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::SyntaxHighlighter;

    #[test]
    fn tree_sitter_is_primary_for_registered_extensions() {
        let highlighter = SyntaxHighlighter::default();

        for extension in [
            "rs", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "py", "pyi", "go", "json",
        ] {
            assert!(
                highlighter
                    .tree_sitter_configuration(Some(extension))
                    .is_some(),
                "{extension} should use Tree-sitter"
            );
        }
    }

    #[test]
    fn unregistered_extensions_fall_back_to_syntect() {
        let highlighter = SyntaxHighlighter::default();

        assert!(
            highlighter
                .tree_sitter_configuration(Some("unknown"))
                .is_none()
        );
        assert!(highlighter.tree_sitter_configuration(None).is_none());
    }
}
