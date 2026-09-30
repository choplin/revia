use unicode_width::UnicodeWidthStr;
use urushi::{StyledText, TextSpan, TextStyle};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) content: String,
    pub(crate) style: TextStyle,
}

impl Span {
    pub(crate) fn raw(content: impl Into<String>) -> Self {
        Self::styled(content, TextStyle::new())
    }

    pub(crate) fn styled(content: impl Into<String>, style: TextStyle) -> Self {
        Self {
            content: content.into(),
            style,
        }
    }

    pub(crate) fn width(&self) -> usize {
        UnicodeWidthStr::width(self.content.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) spans: Vec<Span>,
}

impl Line {
    pub(crate) fn raw(content: impl Into<String>) -> Self {
        Self::styled(content, TextStyle::new())
    }

    pub(crate) fn styled(content: impl Into<String>, style: TextStyle) -> Self {
        Self {
            spans: vec![Span::styled(content, style)],
        }
    }

    pub(crate) fn width(&self) -> usize {
        self.spans.iter().map(Span::width).sum()
    }
}

impl From<Vec<Span>> for Line {
    fn from(spans: Vec<Span>) -> Self {
        Self { spans }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Document {
    pub(crate) lines: Vec<Line>,
}

impl Document {
    pub(crate) fn into_styled_text(self) -> StyledText {
        let line_count = self.lines.len();
        let spans = self
            .lines
            .into_iter()
            .enumerate()
            .flat_map(|(index, line)| {
                line.spans
                    .into_iter()
                    .map(|span| TextSpan::new(span.content, span.style))
                    .chain((index + 1 < line_count).then(|| TextSpan::from("\n")))
            });
        StyledText::try_from_spans(spans)
            .expect("presentation spans end at complete grapheme boundaries")
    }
}

impl From<Vec<Line>> for Document {
    fn from(lines: Vec<Line>) -> Self {
        Self { lines }
    }
}

pub(crate) fn patch_style(mut base: TextStyle, overlay: &TextStyle) -> TextStyle {
    if let Some(color) = overlay.get_foreground() {
        base = base.foreground(color);
    }
    if let Some(color) = overlay.get_background() {
        base = base.background(color);
    }
    if let Some(underline) = overlay.get_underline() {
        base = base.underline(underline);
    }
    if let Some(hyperlink) = overlay.get_hyperlink() {
        base = base.hyperlink(hyperlink.clone());
    }
    base.add_attributes(overlay.get_attributes())
}

#[cfg(test)]
mod tests {
    use urushi::{Color, TextAttribute, TextStyle};

    use super::*;

    #[test]
    fn document_preserves_line_boundaries_and_urushi_styles() {
        let document = Document::from(vec![
            Line::styled("first", TextStyle::new().foreground(Color::CYAN)),
            Line::styled("second", TextStyle::new().bold()),
        ]);

        let text = document.into_styled_text();

        assert_eq!(text.as_str(), "first\nsecond");
        let spans = text.spans().collect::<Vec<_>>();
        assert_eq!(spans[0].1.get_foreground(), Some(Color::CYAN));
        assert!(spans[2].1.get_attributes().contains(TextAttribute::Bold));
    }
}
