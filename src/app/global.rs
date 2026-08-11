use crate::{
    input::{BindingResolution, Key, PhysicalInput},
    semantic::{ContextualKeys, CurrentContext, Footer, Header, SurfaceContext},
    thread::{Resolution, ThreadState},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunningState {
    #[default]
    Running,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingEffect {
    ReloadDiff,
    ChangeThreads,
    ResolveThread,
}

#[derive(Debug)]
pub struct Model {
    pub threads: ThreadState,
    pub status: Option<String>,
    pub pending: Option<PendingEffect>,
    pub running: RunningState,
}

impl Model {
    pub fn new(threads: ThreadState) -> Self {
        Self {
            threads,
            status: None,
            pending: None,
            running: RunningState::Running,
        }
    }

    pub fn set_pending(&mut self, pending: PendingEffect) {
        self.pending = Some(pending);
        self.status = Some(pending_status(pending).into());
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Quit,
    OpenHelp,
    ViewportResized(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    OpenHelp,
    ResizeViewport(u16),
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    match input.key {
        Key::Char('q') | Key::Esc => BindingResolution::Handle(Event::Quit),
        Key::Char('?') => BindingResolution::Handle(Event::OpenHelp),
        _ => BindingResolution::Unbound,
    }
}

pub fn update(model: &mut Model, event: Event) -> Update {
    let mut result = Update::default();
    match event {
        Event::Quit => model.running = RunningState::Done,
        Event::OpenHelp => result.intents.push(Intent::OpenHelp),
        Event::ViewportResized(rows) => result.intents.push(Intent::ResizeViewport(rows)),
    }
    result
}

pub struct ViewInput {
    pub file_count: usize,
    pub context: SurfaceContext,
}

pub struct View {
    pub header: Header,
    pub footer: Footer,
}

pub fn view(model: &Model, input: ViewInput) -> View {
    let threads = model.threads.threads();
    let needs_attention = threads
        .iter()
        .filter(|thread| thread.needs_attention)
        .count();
    let open = threads
        .iter()
        .filter(|thread| matches!(thread.resolution, Resolution::Open))
        .count();
    let resolved = threads.len().saturating_sub(open);
    View {
        header: Header {
            file_count: input.file_count,
            needs_attention,
            open,
            resolved,
        },
        footer: Footer {
            current_context: CurrentContext {
                text: model.status.as_ref().map_or_else(
                    || context_label(input.context).into(),
                    |status| format!("{} • Status: {status}", context_label(input.context)),
                ),
            },
            contextual_keys: ContextualKeys {
                text: context_keys(input.context).into(),
            },
        },
    }
}

fn context_label(context: SurfaceContext) -> &'static str {
    match context {
        SurfaceContext::Files => "Context: Files navigation",
        SurfaceContext::Review => "Context: Review stream",
        SurfaceContext::Threads => "Context: Inline threads",
        SurfaceContext::Rollup => "Context: Thread rollup",
        SurfaceContext::Composer => "Context: Thread composer",
        SurfaceContext::Help => "Context: Keyboard help",
    }
}

fn context_keys(context: SurfaceContext) -> &'static str {
    match context {
        SurfaceContext::Files => ",/. file • [/] hunk • Tab next region • s rail • ? help",
        SurfaceContext::Review => {
            "j/k scroll • f/b page • [/] hunk • ,/. file • Tab next region • ? help"
        }
        SurfaceContext::Threads => {
            "t select • c reply • x resolve • a attention • Tab next region • ? help"
        }
        SurfaceContext::Rollup => "j/k select • Enter jump • v/Esc return • ? help",
        SurfaceContext::Composer => "Enter post • Esc cancel",
        SurfaceContext::Help => "Esc close help",
    }
}

fn pending_status(kind: PendingEffect) -> &'static str {
    match kind {
        PendingEffect::ReloadDiff => "reloading diff…",
        PendingEffect::ChangeThreads => "updating thread…",
        PendingEffect::ResolveThread => "resolving thread…",
    }
}
