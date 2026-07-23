//! Pure review-surface navigation state.
//!
//! This module deliberately knows nothing about crossterm or ratatui.  Keeping
//! viewport, focus, and responsive layout decisions here prevents the renderer
//! and input loop from drifting into separate notions of the visible surface.
//! Review-target selection is domain state in `review::ReviewSession`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    Files,
    Review,
    Threads,
}

impl FocusArea {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::Review,
            Self::Review => Self::Threads,
            Self::Threads => Self::Files,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Files => Self::Threads,
            Self::Review => Self::Files,
            Self::Threads => Self::Review,
        }
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
            Self::Auto if available_width >= 92 => Self::Split,
            Self::Auto => Self::Stack,
            value => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewState {
    pub focus: FocusArea,
    pub scroll: u16,
    pub layout: LayoutMode,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            focus: FocusArea::Review,
            scroll: 0,
            layout: LayoutMode::Auto,
        }
    }
}

impl ViewState {
    pub fn scroll_by(&mut self, delta: i16) {
        self.scroll = self.scroll.saturating_add_signed(delta);
    }

    pub fn reveal(&mut self, row: u16, viewport_height: u16) {
        let bottom = self
            .scroll
            .saturating_add(viewport_height.saturating_sub(1));
        if row < self.scroll {
            self.scroll = row;
        } else if row > bottom {
            self.scroll = row.saturating_sub(viewport_height.saturating_sub(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FocusArea, LayoutMode, ViewState};

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
        assert_eq!(FocusArea::Files.previous(), FocusArea::Threads);
        assert_eq!(FocusArea::Threads.next(), FocusArea::Files);
        assert_eq!(LayoutMode::Auto.resolved(120), LayoutMode::Split);
        assert_eq!(LayoutMode::Auto.resolved(80), LayoutMode::Stack);
    }

    #[test]
    fn reveal_only_moves_the_viewport_when_needed() {
        let mut view = ViewState {
            scroll: 10,
            ..ViewState::default()
        };
        view.reveal(14, 8);
        assert_eq!(view.scroll, 10);
        view.reveal(20, 8);
        assert_eq!(view.scroll, 13);
        view.reveal(2, 8);
        assert_eq!(view.scroll, 2);
    }
}
