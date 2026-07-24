use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::input::{Key, KeyPhase, PhysicalInput};

pub fn physical_input(event: KeyEvent) -> PhysicalInput {
    let key = match event.code {
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Esc => Key::Esc,
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
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
    let phase = match event.kind {
        KeyEventKind::Press => KeyPhase::Press,
        KeyEventKind::Repeat => KeyPhase::Repeat,
        KeyEventKind::Release => KeyPhase::Release,
    };
    PhysicalInput {
        key,
        shift: event.modifiers.contains(KeyModifiers::SHIFT),
        phase,
    }
}
