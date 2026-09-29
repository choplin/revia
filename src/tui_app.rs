use std::{cell::RefCell, sync::Arc};

use urushi::View;
use urushi_tui_app::{
    Application, Effect as TuiEffect, Input, KeyCode, KeyKind, Modifiers, Subscription, Surface,
};

use crate::{
    app::{self, Effect as AppEffect, EffectResult, Model},
    input::{Key, KeyPhase, KeyboardProtocol, PhysicalInput},
    runtime::Runtime,
    urushi_renderer::UrushiRenderer,
};

pub(crate) struct ReviaApplication {
    initial_model: RefCell<Option<Model>>,
    runtime: Arc<Runtime>,
    renderer: UrushiRenderer,
}

impl ReviaApplication {
    pub(crate) fn new(model: Model, runtime: Runtime) -> Self {
        Self {
            initial_model: RefCell::new(Some(model)),
            runtime: Arc::new(runtime),
            renderer: UrushiRenderer::default(),
        }
    }
}

pub(crate) struct RuntimeModel {
    app: Model,
    surface: Surface,
}

pub(crate) enum Message {
    Input(Input),
    Surface(Surface),
    EffectCompleted(EffectResult),
}

impl Application for ReviaApplication {
    type Model = RuntimeModel;
    type Message = Message;

    fn init(&self) -> (Self::Model, TuiEffect<Self::Message>) {
        let app = self
            .initial_model
            .borrow_mut()
            .take()
            .expect("a ReviaApplication is initialized exactly once");
        (
            RuntimeModel {
                app,
                surface: Surface::default(),
            },
            TuiEffect::none(),
        )
    }

    fn update(&self, model: &mut Self::Model, message: Self::Message) -> TuiEffect<Self::Message> {
        let effects = match message {
            Message::Surface(surface) => {
                model.surface = surface;
                app::update(
                    &mut model.app,
                    app::global::Event::ViewportResized {
                        rows: cell_u16(surface.size.rows()).saturating_sub(3),
                        columns: cell_u16(surface.size.columns()),
                    },
                )
            }
            Message::Input(Input::Key(key)) if is_interrupt(key.code, key.modifiers) => {
                app::update(&mut model.app, app::global::Event::Quit)
            }
            Message::Input(Input::Key(key)) => {
                if key.kind != KeyKind::Press {
                    model.app.global.keyboard_protocol = KeyboardProtocol::Kitty;
                }
                let (_, effects) = app::handle_input(&mut model.app, physical_input(key));
                effects
            }
            Message::Input(Input::Paste(_) | Input::Focus(_) | Input::Mouse(_)) => Vec::new(),
            Message::EffectCompleted(result) => app::update(&mut model.app, result),
        };

        if !model.app.is_running() {
            return TuiEffect::shutdown();
        }
        runtime_effects(Arc::clone(&self.runtime), &model.app, effects)
    }

    fn view(&self, model: &Self::Model) -> View {
        self.renderer
            .view(&app::view(&model.app), model.surface.size)
    }

    fn subscriptions(&self, _model: &Self::Model) -> Subscription<Self::Message> {
        Subscription::batch([
            Subscription::surface(Message::Surface),
            Subscription::input(Message::Input),
        ])
    }
}

fn runtime_effects(
    runtime: Arc<Runtime>,
    model: &Model,
    effects: Vec<AppEffect>,
) -> TuiEffect<Message> {
    TuiEffect::batch(effects.into_iter().map(|effect| {
        let runtime = Arc::clone(&runtime);
        let threads = model.global.threads.clone();
        TuiEffect::perform(move || Message::EffectCompleted(runtime.perform(effect, &threads)))
    }))
}

fn physical_input(key: urushi_tui_app::KeyEvent) -> PhysicalInput {
    let word_modifier =
        key.modifiers.contains(Modifiers::ALT) || key.modifiers.contains(Modifiers::CONTROL);
    let key_code = match key.code {
        KeyCode::Enter if key.modifiers.contains(Modifiers::CONTROL) => Key::Submit,
        KeyCode::Char('j' | 'J') if key.modifiers.contains(Modifiers::CONTROL) => Key::Submit,
        KeyCode::Char(_) if key.modifiers.contains(Modifiers::CONTROL) => Key::Other,
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Escape => Key::Esc,
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace if word_modifier => Key::DeleteWordBackward,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left if word_modifier => Key::WordLeft,
        KeyCode::Right if word_modifier => Key::WordRight,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        _ => Key::Other,
    };
    let phase = match key.kind {
        KeyKind::Press => KeyPhase::Press,
        KeyKind::Repeat => KeyPhase::Repeat,
        KeyKind::Release => KeyPhase::Release,
    };
    PhysicalInput {
        key: key_code,
        shift: key.modifiers.contains(Modifiers::SHIFT),
        phase,
    }
}

fn is_interrupt(code: KeyCode, modifiers: Modifiers) -> bool {
    matches!(code, KeyCode::Char('c') | KeyCode::Char('C'))
        && modifiers.contains(Modifiers::CONTROL)
}

fn cell_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use urushi_tui_app::{KeyEvent, KeyKind};

    use super::*;

    #[test]
    fn maps_runtime_keys_to_the_existing_input_vocabulary() {
        assert_eq!(
            physical_input(KeyEvent::new(KeyCode::Left).with_modifiers(Modifiers::ALT)),
            PhysicalInput {
                key: Key::WordLeft,
                shift: false,
                phase: KeyPhase::Press,
            }
        );
        assert_eq!(
            physical_input(KeyEvent::new(KeyCode::Enter).with_modifiers(Modifiers::CONTROL)),
            PhysicalInput {
                key: Key::Submit,
                shift: false,
                phase: KeyPhase::Press,
            }
        );
        assert_eq!(
            physical_input(KeyEvent::new(KeyCode::Char('x')).with_kind(KeyKind::Release)).phase,
            KeyPhase::Release
        );
    }
}
