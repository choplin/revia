use crate::{
    anchor::HunkLocation,
    diff::DiffTarget,
    input::{BindingResolution, Key, PhysicalInput},
    semantic::Overlay,
    thread::{ThreadChange, ThreadId, ThreadOperation, ThreadState, ThreadSuccess},
};

#[derive(Debug, Default)]
pub struct Model {
    input: String,
    reply_to: Option<ThreadId>,
}

impl Model {
    pub fn begin(&mut self, reply_to: Option<ThreadId>) {
        self.input.clear();
        self.reply_to = reply_to;
    }

    fn reset(&mut self) {
        self.input.clear();
        self.reply_to = None;
    }

    #[cfg(test)]
    pub fn input(&self) -> &str {
        &self.input
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Cancel,
    Submit,
    DeleteCharacter,
    InsertCharacter(char),
    EffectCompleted(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ChangeThreads(ThreadOperation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    ThreadsChanged {
        result: Result<ThreadChange, String>,
    },
}

#[derive(Debug, Clone)]
pub struct UpdateInput {
    pub target: DiffTarget,
    pub selected_location: Option<HunkLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Close,
    SetStatus(String),
    ReplaceThreads(ThreadState),
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    match input.key {
        Key::Esc => BindingResolution::Override(Event::Cancel),
        Key::Enter => BindingResolution::Handle(Event::Submit),
        Key::Backspace => BindingResolution::Handle(Event::DeleteCharacter),
        Key::Char(character) => BindingResolution::Override(Event::InsertCharacter(character)),
        _ => BindingResolution::Consume,
    }
}

pub fn update(model: &mut Model, event: Event, input: UpdateInput) -> Update {
    let mut result = Update::default();
    match event {
        Event::Cancel => {
            model.reset();
            result.intents.push(Intent::Close);
        }
        Event::Submit => {
            let body = model.input.clone();
            let reply_to = model.reply_to;
            model.reset();
            result.intents.push(Intent::Close);
            if body.trim().is_empty() {
                status(&mut result, "thread message cannot be empty");
            } else if let Some(location) = input.selected_location {
                result
                    .effects
                    .push(Effect::ChangeThreads(ThreadOperation::Submit {
                        target: input.target,
                        location,
                        body,
                        reply_to,
                    }));
            } else {
                status(&mut result, "select a hunk before posting a thread");
            }
        }
        Event::DeleteCharacter => {
            model.input.pop();
        }
        Event::InsertCharacter(character) => model.input.push(character),
        Event::EffectCompleted(Outcome::ThreadsChanged { result: outcome }) => match outcome {
            Ok(change) => {
                status(&mut result, success_status(change.success));
                result.intents.push(Intent::ReplaceThreads(change.state));
            }
            Err(error) => status(&mut result, error),
        },
    }
    result
}

fn status(result: &mut Update, message: impl Into<String>) {
    result.intents.push(Intent::SetStatus(message.into()));
}

fn success_status(success: ThreadSuccess) -> String {
    match success {
        ThreadSuccess::Posted(id) => format!("posted thread #{id}"),
        ThreadSuccess::Replied => "posted reply".into(),
        ThreadSuccess::Closed => "thread closed".into(),
        ThreadSuccess::Reopened => "thread reopened".into(),
        ThreadSuccess::AttentionToggled => "needs-attention toggled".into(),
        ThreadSuccess::OutdatedToggled => "outdated toggled".into(),
    }
}

pub fn view(model: &Model) -> Overlay {
    Overlay::Composer {
        input: model.input.clone(),
        replying: model.reply_to.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use crate::input::{BindingResolution, Key, KeyPhase, PhysicalInput};

    use super::Event;

    #[test]
    fn character_overrides_global_shortcuts_in_composer() {
        let input = PhysicalInput {
            key: Key::Char('q'),
            shift: false,
            phase: KeyPhase::Press,
        };
        assert_eq!(
            super::bindings(input),
            BindingResolution::Override(Event::InsertCharacter('q'))
        );
    }
}
