use crate::{
    anchor::HunkLocation,
    input::{BindingResolution, Key, PhysicalInput},
    semantic::{Body, RollupBody, RollupItem, ThreadState as SemanticThreadState},
    thread::{Resolution, ThreadId, ThreadState},
};

#[derive(Debug, Default)]
pub struct Model {
    selected: usize,
}

impl Model {
    #[cfg(test)]
    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_thread_id(&self, threads: &ThreadState) -> Option<ThreadId> {
        threads.ordered_ids().get(self.selected).copied()
    }

    pub fn prepare(&mut self, threads: &ThreadState) {
        self.selected = self
            .selected
            .min(threads.ordered_ids().len().saturating_sub(1));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Close,
    Move(i32),
    OpenSelected,
    EffectCompleted(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ResolveThread { id: ThreadId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    ThreadResolved {
        id: ThreadId,
        result: Result<HunkLocation, String>,
    },
}

pub struct UpdateInput<'a> {
    pub threads: &'a ThreadState,
    pub operation_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Close,
    OpenThread {
        id: ThreadId,
        location: HunkLocation,
    },
    SetStatus(String),
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    if input.phase == crate::input::KeyPhase::Release
        || input.phase == crate::input::KeyPhase::Repeat
            && matches!(input.key, Key::Char('v') | Key::Esc | Key::Enter)
    {
        return BindingResolution::Consume;
    }
    match input.key {
        Key::Char('v') | Key::Esc => BindingResolution::Override(Event::Close),
        Key::Char('j') | Key::Down => BindingResolution::Override(Event::Move(1)),
        Key::Char('k') | Key::Up => BindingResolution::Override(Event::Move(-1)),
        Key::Enter => BindingResolution::Handle(Event::OpenSelected),
        _ => BindingResolution::Consume,
    }
}

pub fn update(model: &mut Model, event: Event, input: UpdateInput<'_>) -> Update {
    let mut result = Update::default();
    match event {
        Event::Close => {
            result
                .intents
                .push(Intent::SetStatus("closed thread rollup".into()));
            result.intents.push(Intent::Close);
        }
        Event::Move(delta) => {
            let ids = input.threads.ordered_ids();
            let count = ids.len();
            if count > 0 {
                model.selected = wrapped_index(model.selected, count, delta);
                let id = ids[model.selected];
                result
                    .intents
                    .push(Intent::SetStatus(format!("rollup target: thread #{id}")));
            } else {
                result.intents.push(Intent::SetStatus(
                    "cannot move: rollup has no threads".into(),
                ));
            }
        }
        Event::OpenSelected => {
            if input.operation_pending {
                result.intents.push(Intent::SetStatus(
                    "cannot jump while another operation is pending".into(),
                ));
                return result;
            }
            let ids = input.threads.ordered_ids();
            if let Some(id) = ids.get(model.selected).copied() {
                result.effects.push(Effect::ResolveThread { id });
            } else {
                result.intents.push(Intent::SetStatus(
                    "cannot jump: rollup has no threads".into(),
                ));
            }
        }
        Event::EffectCompleted(Outcome::ThreadResolved {
            id,
            result: outcome,
        }) => match outcome {
            Ok(location) => result.intents.push(Intent::OpenThread { id, location }),
            Err(error) => result.intents.push(Intent::SetStatus(format!(
                "thread #{id} anchor cannot resolve: {error}"
            ))),
        },
    }
    result
}

pub struct ViewInput<'a> {
    pub threads: &'a ThreadState,
}

pub fn view(model: &Model, input: ViewInput<'_>) -> Body {
    let threads = input.threads.threads();
    let need = threads
        .iter()
        .filter(|thread| thread.needs_attention)
        .count();
    let open = threads
        .iter()
        .filter(|thread| !thread.needs_attention && matches!(thread.resolution, Resolution::Open))
        .count();
    let resolved = threads
        .iter()
        .filter(|thread| matches!(thread.resolution, Resolution::Resolved))
        .count();
    Body::Rollup(RollupBody {
        summary: format!("{need} need you / {open} open / {resolved} resolved"),
        scroll: model.selected.try_into().unwrap_or(u16::MAX),
        items: input
            .threads
            .ordered_ids()
            .into_iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let thread = input.threads.thread(id)?;
                Some(RollupItem {
                    id,
                    selected: index == model.selected,
                    state: thread_state(thread),
                    path: thread.anchor.location().path().into(),
                    hunk_header: thread.anchor.location().hunk_header().into(),
                    closed_by: thread.closed_by.as_ref().map(|actor| actor.id.clone()),
                })
            })
            .collect(),
    })
}

fn thread_state(thread: &crate::thread::ReviewThread) -> SemanticThreadState {
    if thread.needs_attention {
        SemanticThreadState::NeedsAttention
    } else if matches!(thread.resolution, Resolution::Open) {
        SemanticThreadState::Open
    } else {
        SemanticThreadState::Resolved
    }
}

fn wrapped_index(current: usize, length: usize, direction: i32) -> usize {
    ((current as i32 + direction).rem_euclid(length as i32)) as usize
}
