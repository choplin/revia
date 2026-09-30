#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Esc,
    Enter,
    Submit,
    Backspace,
    Delete,
    DeleteWordBackward,
    Left,
    Right,
    WordLeft,
    WordRight,
    Tab,
    BackTab,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPhase {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyboardProtocol {
    #[default]
    Legacy,
    Kitty,
}

impl KeyboardProtocol {
    pub const fn supports_ctrl_enter(self) -> bool {
        matches!(self, Self::Kitty)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalInput {
    pub key: Key,
    pub shift: bool,
    pub phase: KeyPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingResolution<E> {
    Handle(E),
    Delegate,
    Consume,
    Override(E),
    Unbound,
}
