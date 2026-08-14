//! Pure review-surface navigation state.
//!
//! This module deliberately knows nothing about crossterm or ratatui.  Keeping
//! viewport, focus, and responsive layout decisions here prevents the renderer
//! and input loop from drifting into separate notions of the visible surface.
//! Review-target selection is domain state in `review::ReviewSession`.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    Review,
    Threads,
}

impl FocusArea {
    pub fn next(self) -> Self {
        match self {
            Self::Review => Self::Threads,
            Self::Threads => Self::Review,
        }
    }

    pub fn previous(self) -> Self {
        self.next()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Auto,
    Split,
    Stack,
}

impl LayoutMode {
    pub fn resolved(self, available_width: u16) -> Self {
        match self {
            Self::Auto if available_width >= 88 => Self::Split,
            Self::Auto => Self::Stack,
            value => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSize {
    Wide,
    Medium,
    Narrow,
}

impl ShellSize {
    pub fn for_width(width: u16) -> Self {
        match width {
            120.. => Self::Wide,
            72.. => Self::Medium,
            _ => Self::Narrow,
        }
    }

    pub fn rail_width(self, available_width: u16) -> Option<u16> {
        match self {
            Self::Wide => Some((available_width / 4).clamp(28, 36)),
            Self::Medium => Some((available_width / 3).clamp(22, 28)),
            Self::Narrow => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewState {
    pub focus: FocusArea,
    pub scroll: usize,
    pub scroll_from_end: Option<usize>,
    pub layout: LayoutMode,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            focus: FocusArea::Review,
            scroll: 0,
            scroll_from_end: None,
            layout: LayoutMode::Auto,
        }
    }
}

impl ViewState {
    pub fn scroll_by(&mut self, delta: i16) {
        if let Some(offset) = &mut self.scroll_from_end {
            if delta < 0 {
                *offset = offset.saturating_add(usize::from(delta.unsigned_abs()));
            } else {
                *offset = offset.saturating_sub(delta as usize);
            }
            return;
        }
        self.scroll = if delta < 0 {
            self.scroll
                .saturating_sub(usize::from(delta.unsigned_abs()))
        } else {
            self.scroll.saturating_add(delta as usize)
        };
    }

    pub fn set_scroll(&mut self, row: usize) {
        self.scroll = row;
        self.scroll_from_end = None;
    }

    pub fn jump_to_end(&mut self) {
        self.scroll_from_end = Some(0);
    }

    pub fn reveal(&mut self, row: usize, viewport_height: usize, margin: usize) {
        self.scroll_from_end = None;
        let margin = margin.min(viewport_height.saturating_sub(1) / 2);
        let top = self.scroll.saturating_add(margin);
        let bottom = self
            .scroll
            .saturating_add(viewport_height.saturating_sub(1).saturating_sub(margin));
        if row < top {
            self.scroll = row.saturating_sub(margin);
        } else if row > bottom {
            self.scroll =
                row.saturating_sub(viewport_height.saturating_sub(1).saturating_sub(margin));
        }
    }

    pub fn resolved_scroll(self, total_rows: usize, viewport_height: usize) -> usize {
        let max_scroll = total_rows.saturating_sub(viewport_height);
        self.scroll_from_end.map_or_else(
            || self.scroll.min(max_scroll),
            |offset| max_scroll.saturating_sub(offset),
        )
    }

    pub fn clamp(&mut self, total_rows: usize, viewport_height: usize) {
        if self.scroll_from_end.is_none() {
            self.scroll = self.resolved_scroll(total_rows, viewport_height);
        }
    }
}

pub(crate) fn review_body_width(columns: u16, sidebar_visible: bool) -> u16 {
    let body_width = if sidebar_visible {
        ShellSize::for_width(columns)
            .rail_width(columns)
            .map_or(columns, |rail| columns.saturating_sub(rail))
    } else {
        columns
    };
    body_width.saturating_sub(1)
}

pub(crate) fn truncate_start(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".into();
    }

    let mut tail = String::new();
    let mut width: usize = 1;
    for grapheme in value.graphemes(true).rev() {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > max_width {
            break;
        }
        tail.insert_str(0, grapheme);
        width = width.saturating_add(grapheme_width);
    }
    format!("…{tail}")
}

pub(crate) fn truncate_end(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".into();
    }

    let mut result = String::new();
    let mut width: usize = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > max_width - 1 {
            break;
        }
        result.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    result.push('…');
    result
}

pub(crate) fn fit_width(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let truncated = UnicodeWidthStr::width(value) > width;
    let content_width = if truncated {
        width.saturating_sub(1)
    } else {
        width
    };
    let mut result = String::new();
    let mut used: usize = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used.saturating_add(grapheme_width) > content_width {
            break;
        }
        result.push_str(grapheme);
        used = used.saturating_add(grapheme_width);
    }
    if truncated {
        result.push('…');
        used = used.saturating_add(1);
    }
    result.push_str(&" ".repeat(width.saturating_sub(used)));
    result
}

#[cfg(test)]
mod tests {
    use super::{FocusArea, LayoutMode, ShellSize, ViewState, fit_width, truncate_start};

    #[test]
    fn viewport_state_does_not_own_review_selection() {
        let mut view = ViewState {
            scroll: 22,
            ..ViewState::default()
        };
        view.scroll_by(-5);
        assert_eq!(view.scroll, 17);
    }

    #[test]
    fn focus_and_layout_have_explicit_cycles() {
        assert_eq!(FocusArea::Review.previous(), FocusArea::Threads);
        assert_eq!(FocusArea::Threads.next(), FocusArea::Review);
        assert_eq!(LayoutMode::Auto.resolved(120), LayoutMode::Split);
        assert_eq!(LayoutMode::Auto.resolved(80), LayoutMode::Stack);
        assert_eq!(LayoutMode::Auto.resolved(87), LayoutMode::Stack);
        assert_eq!(LayoutMode::Auto.resolved(88), LayoutMode::Split);
    }

    #[test]
    fn reveal_keeps_the_target_inside_the_requested_margin() {
        let mut view = ViewState {
            scroll: 10,
            ..ViewState::default()
        };
        view.reveal(14, 8, 2);
        assert_eq!(view.scroll, 10);
        view.reveal(16, 8, 2);
        assert_eq!(view.scroll, 11);
        view.reveal(2, 8, 2);
        assert_eq!(view.scroll, 0);
        view.reveal(8, 3, 2);
        assert_eq!(view.scroll, 7);
    }

    #[test]
    fn end_relative_scrolling_moves_without_an_unbounded_offset() {
        let mut view = ViewState::default();
        view.jump_to_end();
        assert_eq!(view.scroll_from_end, Some(0));
        view.scroll_by(-1);
        assert_eq!(view.scroll_from_end, Some(1));
        view.scroll_by(1);
        assert_eq!(view.scroll_from_end, Some(0));
        view.set_scroll(3);
        assert_eq!(view.scroll_from_end, None);
        assert_eq!(view.scroll, 3);
    }

    #[test]
    fn resolved_scroll_clamps_absolute_and_preserves_end_relative_offsets() {
        let mut view = ViewState {
            scroll: 99,
            ..ViewState::default()
        };
        assert_eq!(view.resolved_scroll(40, 10), 30);
        view.jump_to_end();
        assert_eq!(view.resolved_scroll(40, 10), 30);
        view.scroll_by(-3);
        assert_eq!(view.resolved_scroll(40, 10), 27);
    }

    #[test]
    fn logical_scroll_is_not_limited_by_the_terminal_coordinate_width() {
        let mut view = ViewState::default();
        view.jump_to_end();
        assert_eq!(view.resolved_scroll(70_000, 10), 69_990);
        view.scroll_by(-3);
        assert_eq!(view.resolved_scroll(70_000, 10), 69_987);
    }

    #[test]
    fn shell_size_keeps_a_readable_body_as_width_decreases() {
        assert_eq!(ShellSize::for_width(120).rail_width(120), Some(30));
        assert_eq!(ShellSize::for_width(119), ShellSize::Medium);
        assert_eq!(ShellSize::for_width(120), ShellSize::Wide);
        assert_eq!(ShellSize::for_width(88).rail_width(88), Some(28));
        assert_eq!(ShellSize::for_width(64).rail_width(64), None);
    }

    #[test]
    fn terminal_width_helpers_preserve_complete_graphemes() {
        assert!(unicode_width::UnicodeWidthStr::width(truncate_start("long/✈️", 2).as_str()) <= 2);
        assert_eq!(truncate_start("long/画\u{301}", 2), "…");
        for value in ["✈️", "👨‍👩‍👧‍👦", "画\u{301}"] {
            let fitted = fit_width(value, 10);
            assert_eq!(unicode_width::UnicodeWidthStr::width(fitted.as_str()), 10);
        }
    }
}
