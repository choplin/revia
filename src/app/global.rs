use crate::{
    app::effect::{OperationId, PendingEffectKind},
    input::{BindingResolution, Key, PhysicalInput},
    mode::ActiveMode,
    semantic::{ContextualKeys, CurrentContext, Footer, Header, SurfaceContext},
    thread::{Resolution, ThreadState},
    ui::{truncate_end, truncate_start},
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunningState {
    #[default]
    Running,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingEffect {
    pub operation_id: OperationId,
    pub owner: ActiveMode,
    pub kind: PendingEffectKind,
}

#[derive(Debug)]
pub struct Model {
    pub threads: ThreadState,
    pub status: Option<String>,
    pub pending: Option<PendingEffect>,
    next_operation_id: OperationId,
    pub running: RunningState,
}

impl Model {
    pub fn new(threads: ThreadState) -> Self {
        Self {
            threads,
            status: None,
            pending: None,
            next_operation_id: 0,
            running: RunningState::Running,
        }
    }

    pub fn set_pending(&mut self, owner: ActiveMode, kind: PendingEffectKind) -> OperationId {
        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.wrapping_add(1);
        self.pending = Some(PendingEffect {
            operation_id,
            owner,
            kind,
        });
        self.status = Some(pending_status(kind).into());
        operation_id
    }

    pub fn finish_pending(
        &mut self,
        operation_id: OperationId,
        owner: ActiveMode,
        kind: PendingEffectKind,
    ) -> bool {
        let matches = self.pending.is_some_and(|pending| {
            pending.operation_id == operation_id && pending.owner == owner && pending.kind == kind
        });
        if matches {
            self.pending = None;
        }
        matches
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Quit,
    OpenHelp,
    ViewportResized { rows: u16, columns: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    OpenHelp,
    ResizeViewport { rows: u16, columns: u16 },
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
        Key::Char('q') | Key::Esc => BindingResolution::Handle(Event::Quit),
        Key::Char('?') => BindingResolution::Handle(Event::OpenHelp),
        _ => BindingResolution::Unbound,
    }
}

pub fn update(model: &mut Model, event: Event) -> Update {
    let mut result = Update::default();
    match event {
        Event::Quit => model.running = RunningState::Done,
        Event::OpenHelp => {
            model.status = Some("opened keyboard help".into());
            result.intents.push(Intent::OpenHelp);
        }
        Event::ViewportResized { rows, columns } => result
            .intents
            .push(Intent::ResizeViewport { rows, columns }),
    }
    result
}

pub struct ViewInput {
    pub file_count: usize,
    pub context: SurfaceContext,
    pub target: Option<String>,
    pub selected_thread_available: bool,
    pub width: u16,
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
                text: context_text(
                    input.context,
                    input.target.as_deref(),
                    model.status.as_deref(),
                    model.pending.map(|pending| pending_status(pending.kind)),
                    input.width,
                ),
            },
            contextual_keys: ContextualKeys {
                text: context_keys(input.context, input.selected_thread_available, input.width)
                    .into(),
            },
        },
    }
}

fn context_label(context: SurfaceContext) -> &'static str {
    match context {
        SurfaceContext::Review => "Context: Review stream",
        SurfaceContext::Threads => "Context: Inline threads",
        SurfaceContext::SearchInput => "Context: Search input",
        SurfaceContext::SearchResults => "Context: Search results",
        SurfaceContext::Rollup => "Context: Thread rollup",
        SurfaceContext::Composer => "Context: Thread composer",
        SurfaceContext::Help => "Context: Keyboard help",
    }
}

fn context_text(
    context: SurfaceContext,
    target: Option<&str>,
    status: Option<&str>,
    pending: Option<&str>,
    width: u16,
) -> String {
    if width < 72 {
        return compact_context(context, target, status, pending, width);
    }
    let mut text = context_label(context).to_owned();
    if let Some(status) = status {
        text.push_str(" • Status: ");
        text.push_str(status);
    }
    if let Some(pending) = pending.filter(|pending| Some(*pending) != status) {
        text.push_str(" • Pending: ");
        text.push_str(pending);
    }
    if let Some(target) = target {
        text.push_str(" • Target: ");
        text.push_str(target);
    }
    text
}

fn compact_context(
    context: SurfaceContext,
    target: Option<&str>,
    status: Option<&str>,
    pending: Option<&str>,
    width: u16,
) -> String {
    let status = status.map(|status| {
        if status == "file rail enabled; hidden below 72 columns" {
            "rail hidden<72; on"
        } else {
            status
        }
    });
    let label = match context {
        SurfaceContext::Review => "Review",
        SurfaceContext::Threads => "Thread",
        SurfaceContext::SearchInput => "Search",
        SurfaceContext::SearchResults => "Matches",
        SurfaceContext::Rollup => "Rollup",
        SurfaceContext::Composer => "Compose",
        SurfaceContext::Help => "Help",
    };
    let available = usize::from(width.saturating_sub(2));
    let mut text = label.to_owned();
    if let Some(target) = target {
        let has_feedback = status.is_some() || pending.is_some();
        let budget = if has_feedback {
            (available / 3).max(8)
        } else {
            available
                .saturating_sub(UnicodeWidthStr::width(label))
                .saturating_sub(3)
        };
        text.push_str(" • ");
        text.push_str(&truncate_start(target, budget));
    }
    let feedback = match (status, pending.filter(|pending| Some(*pending) != status)) {
        (Some(status), Some(pending)) => Some(format!("{status}; {pending}")),
        (Some(status), None) => Some(status.to_owned()),
        (None, Some(pending)) => Some(pending.to_owned()),
        (None, None) => None,
    };
    if let Some(feedback) = feedback {
        let used = UnicodeWidthStr::width(text.as_str()).saturating_add(3);
        if used < available {
            text.push_str(" • ");
            text.push_str(&truncate_end(&feedback, available - used));
        }
    }
    text
}

fn context_keys(
    context: SurfaceContext,
    selected_thread_available: bool,
    width: u16,
) -> &'static str {
    if width < 72 {
        return match context {
            SurfaceContext::Review if selected_thread_available => {
                "j/k stream · [/] hunk · ,/. file · / search"
            }
            SurfaceContext::Review => "j/k stream · [/] hunk · ,/. file · / search",
            SurfaceContext::Threads if selected_thread_available => {
                "Tab stream · t/T · c/C · x/R · a/o"
            }
            SurfaceContext::Threads => "Tab stream · c new · ? help",
            SurfaceContext::SearchInput => "type query · Enter keep · Esc cancel",
            SurfaceContext::SearchResults => "n/N matches · / new · Esc cancel",
            SurfaceContext::Rollup if selected_thread_available => {
                "j/k select · Enter jump · v/Esc return"
            }
            SurfaceContext::Rollup => "no targets · v/Esc return",
            SurfaceContext::Composer => "Ctrl-S post · Enter newline · Esc cancel",
            SurfaceContext::Help => "Esc/? close",
        };
    }
    match context {
        SurfaceContext::Review if selected_thread_available => {
            "j/k stream • [/] hunk • ,/. file • / search • t/T thread • Tab thread"
        }
        SurfaceContext::Review => {
            "j/k stream • [/] hunk • ,/. file • / search • c new thread • ? help"
        }
        SurfaceContext::Threads if selected_thread_available => {
            "t/T thread • c reply • C new • x resolve • R reopen • a attention • Tab stream • ? help"
        }
        SurfaceContext::Threads => "c new thread • Tab stream • ? help",
        SurfaceContext::SearchInput => "type query • Backspace delete • Enter keep • Esc cancel",
        SurfaceContext::SearchResults => {
            "n/N next/previous (wrap) • / new search • ? help • Esc cancel"
        }
        SurfaceContext::Rollup if selected_thread_available => {
            "j/k select • Enter jump • v/Esc return"
        }
        SurfaceContext::Rollup => "no thread targets • v/Esc return",
        SurfaceContext::Composer => "Ctrl-S post • Enter newline • Esc cancel",
        SurfaceContext::Help => "Esc/? close help",
    }
}

fn pending_status(kind: PendingEffectKind) -> &'static str {
    match kind {
        PendingEffectKind::ReloadDiff => "reloading diff…",
        PendingEffectKind::ChangeThreads => "updating thread…",
        PendingEffectKind::ResolveThread => "resolving thread…",
    }
}
