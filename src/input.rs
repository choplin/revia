#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Esc,
    Enter,
    Backspace,
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
