//! Intent-level commands for one review session.
//!
//! Terminal key codes are intentionally absent. The crossterm adapter maps a
//! key event to one of these commands, while the review workflow owns their
//! effect on session and thread state.

use crate::ui::LayoutMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewCommand {
    Quit,
    ShowHelp,
    CycleFocus,
    PreviousFocus,
    ScrollRows(i16),
    ScrollViewport(i16),
    ScrollHalfViewport(i16),
    JumpToStreamEdge { end: bool },
    FocusReview,
    MoveHunk(i32),
    MoveFile(i32),
    AdjustContext(i32),
    BeginThread { always_new: bool },
    SelectThread,
    CloseThread,
    ReopenThread,
    ToggleAttention,
    ToggleOutdated,
    MoveAttention(i32),
    ShowRollup,
    SetLayout(LayoutMode),
    ToggleSidebar,
    ReloadDiff,
    ToggleHunkHeaders,
    ToggleWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    Continue,
    Quit,
}
