use crate::{
    app::effect::{OperationId, PendingEffectKind},
    input::{BindingResolution, Key, KeyboardProtocol, PhysicalInput},
    mode::ActiveMode,
    semantic::{ContextualKeys, CurrentContext, Footer, Header, SurfaceContext},
    thread::ThreadState,
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
    pub keyboard_protocol: KeyboardProtocol,
}

impl Model {
    pub fn new(threads: ThreadState) -> Self {
        Self {
            threads,
            status: None,
            pending: None,
            next_operation_id: 0,
            running: RunningState::Running,
            keyboard_protocol: KeyboardProtocol::Legacy,
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
    pub comparison: String,
    pub file_count: usize,
    pub magnitude: crate::diff::Magnitude,
    pub active_filter: &'static str,
    pub context: SurfaceContext,
    pub target: Option<String>,
    pub selected_thread_available: bool,
    pub selected_thread_resolved: bool,
    pub width: u16,
}

pub struct View {
    pub header: Header,
    pub footer: Footer,
}

pub fn view(model: &Model, input: ViewInput) -> View {
    View {
        header: Header {
            comparison: input.comparison,
            file_count: input.file_count,
            magnitude: input.magnitude,
            active_filter: input.active_filter.into(),
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
                text: context_keys(
                    input.context,
                    input.selected_thread_available,
                    input.selected_thread_resolved,
                    model.keyboard_protocol,
                    input.width,
                ),
            },
        },
    }
}

fn context_label(context: SurfaceContext) -> &'static str {
    match context {
        SurfaceContext::Review => "Context: Diff",
        SurfaceContext::Threads => "Context: Inline threads",
        SurfaceContext::Files => "Context: File rail",
        SurfaceContext::SearchInput => "Context: Search input",
        SurfaceContext::SearchResults => "Context: Search results",
        SurfaceContext::Rollup => "Context: Thread rollup",
        SurfaceContext::Composer => "Context: Comment",
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
        SurfaceContext::Files => "Files",
        SurfaceContext::SearchInput => "Search",
        SurfaceContext::SearchResults => "Matches",
        SurfaceContext::Rollup => "Rollup",
        SurfaceContext::Composer => "Comment",
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
    selected_thread_resolved: bool,
    keyboard_protocol: KeyboardProtocol,
    width: u16,
) -> String {
    let (required, optional): (&[&str], &[&str]) = match context {
        SurfaceContext::Review if selected_thread_available => (
            &["q exit", "c comment", "? help"],
            &[
                "j/k rows",
                "[/] hunk",
                "1/2 layout",
                "w wrap",
                "/ search",
                "t/T thread",
                ",/. file",
                "F filter",
                "A all",
            ],
        ),
        SurfaceContext::Review => (
            &["q exit", "c comment", "? help"],
            &[
                "j/k rows",
                "[/] hunk",
                "1/2 layout",
                "w wrap",
                "/ search",
                ",/. file",
                "Tab rail",
                "F filter",
                "A all",
            ],
        ),
        SurfaceContext::Threads if selected_thread_resolved => (
            &["q exit", "Tab", "R reopen", "e fold", "? help"],
            &["t/T thread", "c reply", "C new", "a/o flags"],
        ),
        SurfaceContext::Threads if selected_thread_available => (
            &["q exit", "Tab diff", "x resolve", "? help"],
            &["t/T thread", "c reply", "C new", "a/o flags", "e fold"],
        ),
        SurfaceContext::Threads => (&["q exit", "Tab diff", "c comment", "? help"], &[]),
        SurfaceContext::Files => (
            &["q exit", "Enter open/fold", "Tab/←/→ diff", "? help"],
            &["j/k item", "` flat/tree", "-/= fold", "s hide rail"],
        ),
        SurfaceContext::SearchInput => (
            &["Esc cancel", "Enter keep"],
            &["type query", "Backspace delete"],
        ),
        SurfaceContext::SearchResults => {
            (&["Esc cancel", "n/N matches", "? help"], &["/ new search"])
        }
        SurfaceContext::Rollup if selected_thread_available => {
            (&["v/Esc return", "Enter jump"], &["j/k select"])
        }
        SurfaceContext::Rollup => (&["v/Esc return"], &["no targets"]),
        SurfaceContext::Composer if keyboard_protocol.supports_ctrl_enter() => (
            &["Esc cancel", "Ctrl-Enter/Ctrl-J post"],
            &["Enter newline"],
        ),
        SurfaceContext::Composer => (&["Esc cancel", "Ctrl-J post"], &["Enter newline"]),
        SurfaceContext::Help => (&["Esc/? close"], &["j/k rows", "f/b pages", "g/G edges"]),
    };
    fit_key_groups(required, optional, width)
}

fn fit_key_groups(required: &[&str], optional: &[&str], width: u16) -> String {
    const PREFIX: &str = "Keys: ";
    const SEPARATOR: &str = " · ";
    let budget = usize::from(width).saturating_sub(UnicodeWidthStr::width(PREFIX));
    let mut result = String::new();
    for group in required.iter().chain(optional) {
        let separator = if result.is_empty() { "" } else { SEPARATOR };
        let added_width = UnicodeWidthStr::width(separator) + UnicodeWidthStr::width(*group);
        if UnicodeWidthStr::width(result.as_str()).saturating_add(added_width) <= budget {
            result.push_str(separator);
            result.push_str(group);
        }
    }
    result
}

fn pending_status(kind: PendingEffectKind) -> &'static str {
    match kind {
        PendingEffectKind::ReloadDiff => "reloading diff…",
        PendingEffectKind::ChangeThreads => "updating thread…",
        PendingEffectKind::ResolveThread => "resolving thread…",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_thread_keys_preserve_critical_actions_at_exact_widths() {
        let at_48 = context_keys(
            SurfaceContext::Threads,
            true,
            true,
            KeyboardProtocol::Legacy,
            48,
        );
        let at_72 = context_keys(
            SurfaceContext::Threads,
            true,
            true,
            KeyboardProtocol::Legacy,
            72,
        );
        let at_119 = context_keys(
            SurfaceContext::Threads,
            true,
            true,
            KeyboardProtocol::Legacy,
            119,
        );

        assert_eq!(at_48, "q exit · Tab · R reopen · e fold · ? help");
        assert_eq!(
            at_72,
            "q exit · Tab · R reopen · e fold · ? help · t/T thread · c reply"
        );
        assert_eq!(
            at_119,
            "q exit · Tab · R reopen · e fold · ? help · t/T thread · c reply · C new · a/o flags"
        );
        for (width, keys) in [(48, at_48), (72, at_72), (119, at_119)] {
            assert!(keys.contains("Tab"));
            assert!(keys.contains("R reopen"));
            assert!(keys.contains("e fold"));
            assert!(keys.contains("? help"));
            assert!(UnicodeWidthStr::width(format!("Keys: {keys}").as_str()) <= width);
        }
    }

    #[test]
    fn every_footer_context_fits_the_display_cell_budget() {
        let contexts = [
            SurfaceContext::Review,
            SurfaceContext::Threads,
            SurfaceContext::Files,
            SurfaceContext::SearchInput,
            SurfaceContext::SearchResults,
            SurfaceContext::Rollup,
            SurfaceContext::Composer,
            SurfaceContext::Help,
        ];
        for width in [48, 72, 119] {
            for context in contexts {
                for selected in [false, true] {
                    let keys =
                        context_keys(context, selected, selected, KeyboardProtocol::Legacy, width);
                    assert!(
                        UnicodeWidthStr::width(format!("Keys: {keys}").as_str())
                            <= usize::from(width),
                        "{context:?} at {width}: {keys}"
                    );
                }
            }
        }
    }

    #[test]
    fn composer_keys_only_advertise_ctrl_enter_when_enhanced_input_is_available() {
        let legacy = context_keys(
            SurfaceContext::Composer,
            false,
            false,
            KeyboardProtocol::Legacy,
            80,
        );
        let kitty = context_keys(
            SurfaceContext::Composer,
            false,
            false,
            KeyboardProtocol::Kitty,
            80,
        );

        assert_eq!(legacy, "Esc cancel · Ctrl-J post · Enter newline");
        assert_eq!(kitty, "Esc cancel · Ctrl-Enter/Ctrl-J post · Enter newline");
    }
}
