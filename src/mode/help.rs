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
    SetStatus(&'static str),
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    if input.phase != crate::input::KeyPhase::Press {
        return BindingResolution::Consume;
    }
    match input.key {
        Key::Esc | Key::Char('?') => BindingResolution::Override(Event::Close),
        _ => BindingResolution::Consume,
    }
}

pub fn update(_: &mut Model, event: Event) -> Update {
    let mut result = Update::default();
    match event {
        Event::Close => {
            result
                .intents
                .push(Intent::SetStatus("closed keyboard help"));
            result.intents.push(Intent::Close);
        }
    }
    result
}

pub fn view(_: &Model) -> Overlay {
    Overlay::Help {
        text: HUNK_KEYBOARD_HELP,
    }
}

const HUNK_KEYBOARD_HELP: &str = "Hunk-compatible review stream\n\n  j/k, ↑/↓ rows      f/Space, b pages\n  d/u half page      g/G, Home/End edges\n  [/] hunk           ,/. file\n  1/2/0 layout       s file rail\n  =/- context        r reload\n  m headers          w wrapping\n\nrevia review extensions\n\n  t/T thread         {/} attention\n  c/C compose        x/R resolve/reopen\n  a/o flags          v rollup\n  Tab stream/thread  ? close help\n  q quit review      Esc close/quit";
