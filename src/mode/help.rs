use crate::{
    input::{BindingResolution, Key, PhysicalInput},
    semantic::Overlay,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Context {
    #[default]
    Review,
    Threads,
    SearchResults,
}

#[derive(Debug, Default)]
pub struct Model {
    context: Context,
}

impl Model {
    pub fn begin(&mut self, context: Context) {
        self.context = context;
    }
}

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

pub fn view(model: &Model) -> Overlay {
    let context = match model.context {
        Context::Review => "review stream",
        Context::Threads => "inline thread",
        Context::SearchResults => "search results",
    };
    let search = marker(model.context == Context::SearchResults);
    let thread = marker(model.context == Context::Threads);
    let escape = if model.context == Context::SearchResults {
        "Esc cancel search"
    } else {
        "Esc quit review"
    };
    Overlay::Help {
        text: format!(
            "◆ commands valid from {context}\n\nNavigation\n◆ j/k, ↑/↓ rows   f/Space, b pages   d/u half page\n◆ g/G edges        [/] hunk          ,/. file\n◆ / full-diff search {search} n/N next/previous match (wrap)\n\nView\n◆ F cycle filter   A All changes     1/2/0 layout\n◆ s file rail      m headers · w wrap\n◆ =/- context      r reload (filter retained)\n\nReview actions\n◆ t/T thread       Tab stream/thread c/C compose/new\n{thread} x/R resolve/reopen   {thread} a/o flags   {thread} e resolved fold\n◆ {{/}} attention (Git order, wrap)   v rollup\n\nGlobal / exit\n◆ ? help           q quit review     {escape}\n  In help: Esc/? closes and returns to {context}"
        ),
    }
}

fn marker(valid: bool) -> &'static str {
    if valid { "◆" } else { "·" }
}
