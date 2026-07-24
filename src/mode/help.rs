use crate::{
    input::{BindingResolution, Key, PhysicalInput},
    semantic::Overlay,
};

#[derive(Debug, Default)]
pub struct Model;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Close,
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    match input.key {
        Key::Esc | Key::Char('?') => BindingResolution::Override(Event::Close),
        _ => BindingResolution::Consume,
    }
}

pub fn update(_: &mut Model, event: Event) -> Update {
    let mut result = Update::default();
    match event {
        Event::Close => result.intents.push(Intent::Close),
    }
    result
}

pub fn view(_: &Model) -> Overlay {
    Overlay::Help {
        text: HUNK_KEYBOARD_HELP,
    }
}

const HUNK_KEYBOARD_HELP: &str = "Hunk-compatible navigation\n\n  j/k  ↑/↓      scroll one row\n  f/Space, b    page down/up\n  d/u            half page\n  g/G Home/End   start/end\n  [/]            previous/next hunk\n  ,/.            previous/next file\n  1/2/0          split/stack/auto\n  s              toggle file rail\n  r              reload diff\n  m / w          hunk headers / wrapping\n\nrevia review extensions: c compose, t select thread, x resolve, R reopen, a attention, {/} attention jump.";
