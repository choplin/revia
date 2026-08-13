use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
    event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};

use crate::input::{Key, KeyPhase, KeyboardProtocol, PhysicalInput};

pub fn physical_input(event: KeyEvent) -> PhysicalInput {
    let word_modifier = event
        .modifiers
        .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL);
    let key = match event.code {
        KeyCode::Enter if event.modifiers.contains(KeyModifiers::CONTROL) => Key::Submit,
        KeyCode::Char('j' | 'J') if event.modifiers.contains(KeyModifiers::CONTROL) => Key::Submit,
        KeyCode::Char(_) if event.modifiers.contains(KeyModifiers::CONTROL) => Key::Other,
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Esc => Key::Esc,
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

/// Return whether an input event represents the interactive Ctrl-C interrupt.
pub fn is_interrupt(event: KeyEvent) -> bool {
    matches!(event.code, KeyCode::Char('c') | KeyCode::Char('C'))
        && event.modifiers.contains(KeyModifiers::CONTROL)
}

pub trait TerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn supports_keyboard_enhancement(&mut self) -> io::Result<bool>;
    fn enter_alternate_screen(&mut self) -> io::Result<()>;
    fn enable_keyboard_enhancement(&mut self) -> io::Result<()>;
    fn disable_keyboard_enhancement(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
}

pub struct TerminalSession<C: TerminalControl> {
    control: C,
    raw_mode_enabled: bool,
    alternate_screen_enabled: bool,
    keyboard_enhancement_enabled: bool,
}

impl<C: TerminalControl> TerminalSession<C> {
    fn acquire(mut control: C) -> Result<Self> {
        control
            .enable_raw_mode()
            .context("could not enable terminal raw mode")?;
        let mut session = Self {
            control,
            raw_mode_enabled: true,
            alternate_screen_enabled: false,
            keyboard_enhancement_enabled: false,
        };
        // Enhancement detection is optional. If the terminal cannot answer the
        // query, legacy input remains a complete and usable fallback.
        let keyboard_enhancement_supported = session
            .control
            .supports_keyboard_enhancement()
            .unwrap_or(false);
        session.alternate_screen_enabled = true;
        session
            .control
            .enter_alternate_screen()
            .context("could not enter terminal alternate screen")?;
        if keyboard_enhancement_supported {
            session.keyboard_enhancement_enabled = true;
            session
                .control
                .enable_keyboard_enhancement()
                .context("could not enable modified-key reporting")?;
        }
        Ok(session)
    }

    pub fn keyboard_protocol(&self) -> KeyboardProtocol {
        if self.keyboard_enhancement_enabled {
            KeyboardProtocol::Kitty
        } else {
            KeyboardProtocol::Legacy
        }
    }

    pub fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        let cleanup = self.restore();
        match result {
            Ok(value) => {
                cleanup.context("could not fully restore the terminal state")?;
                Ok(value)
            }
            Err(error) => {
                if let Err(cleanup) = cleanup {
                    eprintln!("warning: could not fully restore terminal state: {cleanup}");
                }
                Err(error)
            }
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;
        if self.keyboard_enhancement_enabled {
            restore_step(
                &mut first_error,
                self.control.disable_keyboard_enhancement(),
            );
            self.keyboard_enhancement_enabled = false;
        }
        if self.alternate_screen_enabled {
            restore_step(&mut first_error, self.control.show_cursor());
            restore_step(&mut first_error, self.control.leave_alternate_screen());
            self.alternate_screen_enabled = false;
        }
        if self.raw_mode_enabled {
            restore_step(&mut first_error, self.control.disable_raw_mode());
            self.raw_mode_enabled = false;
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl<C: TerminalControl> Drop for TerminalSession<C> {
    fn drop(&mut self) {
        // This is the best available cleanup path during unwinding. A process
        // terminated directly by an OS signal cannot run Drop handlers.
        let _ = self.restore();
    }
}

fn restore_step(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

pub struct CrosstermControl<W> {
    writer: W,
}

impl<W> CrosstermControl<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> TerminalControl for CrosstermControl<W> {
    fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn supports_keyboard_enhancement(&mut self) -> io::Result<bool> {
        supports_keyboard_enhancement()
    }

    fn enter_alternate_screen(&mut self) -> io::Result<()> {
        execute!(self.writer, EnterAlternateScreen)
    }

    fn enable_keyboard_enhancement(&mut self) -> io::Result<()> {
        execute!(
            self.writer,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
    }

    fn disable_keyboard_enhancement(&mut self) -> io::Result<()> {
        execute!(self.writer, PopKeyboardEnhancementFlags)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(self.writer, Show)
    }

    fn leave_alternate_screen(&mut self) -> io::Result<()> {
        execute!(self.writer, LeaveAlternateScreen)
    }

    fn disable_raw_mode(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }
}

impl<W: Write> TerminalSession<CrosstermControl<W>> {
    pub fn start(writer: W) -> Result<Self> {
        Self::acquire(CrosstermControl::new(writer))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, io};

    use anyhow::anyhow;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::input::{Key, KeyboardProtocol};

    use super::{TerminalControl, TerminalSession, is_interrupt, physical_input};

    #[derive(Clone)]
    struct FakeControl {
        calls: std::rc::Rc<RefCell<Vec<&'static str>>>,
        fail_at: Option<&'static str>,
        keyboard_enhancement_supported: bool,
    }

    impl FakeControl {
        fn new(fail_at: Option<&'static str>) -> (Self, std::rc::Rc<RefCell<Vec<&'static str>>>) {
            let calls = std::rc::Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    calls: calls.clone(),
                    fail_at,
                    keyboard_enhancement_supported: true,
                },
                calls,
            )
        }

        fn step(&mut self, step: &'static str) -> io::Result<()> {
            self.calls.borrow_mut().push(step);
            if self.fail_at == Some(step) {
                Err(io::Error::other(format!("injected {step} failure")))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalControl for FakeControl {
        fn enable_raw_mode(&mut self) -> io::Result<()> {
            self.step("enable_raw")
        }

        fn supports_keyboard_enhancement(&mut self) -> io::Result<bool> {
            self.step("detect_keys")?;
            Ok(self.keyboard_enhancement_supported)
        }

        fn enter_alternate_screen(&mut self) -> io::Result<()> {
            self.step("enter_alt")
        }

        fn enable_keyboard_enhancement(&mut self) -> io::Result<()> {
            self.step("enable_keys")
        }

        fn disable_keyboard_enhancement(&mut self) -> io::Result<()> {
            self.step("disable_keys")
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.step("show_cursor")
        }

        fn leave_alternate_screen(&mut self) -> io::Result<()> {
            self.step("leave_alt")
        }

        fn disable_raw_mode(&mut self) -> io::Result<()> {
            self.step("disable_raw")
        }
    }

    #[test]
    fn restores_every_acquired_capability_after_initialization_failure() {
        let (control, calls) = FakeControl::new(Some("enter_alt"));
        let error = match TerminalSession::acquire(control) {
            Ok(_) => panic!("initialization should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("alternate screen"));
        assert_eq!(
            &*calls.borrow(),
            &[
                "enable_raw",
                "detect_keys",
                "enter_alt",
                "show_cursor",
                "leave_alt",
                "disable_raw"
            ]
        );
    }

    #[test]
    fn restores_keyboard_mode_when_enhancement_initialization_fails() {
        let (control, calls) = FakeControl::new(Some("enable_keys"));
        let error = match TerminalSession::acquire(control) {
            Ok(_) => panic!("initialization should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("modified-key reporting"));
        assert_eq!(
            &*calls.borrow(),
            &[
                "enable_raw",
                "detect_keys",
                "enter_alt",
                "enable_keys",
                "disable_keys",
                "show_cursor",
                "leave_alt",
                "disable_raw"
            ]
        );
    }

    #[test]
    fn unsupported_keyboard_enhancement_keeps_legacy_input_without_push_or_pop() {
        let (mut control, calls) = FakeControl::new(None);
        control.keyboard_enhancement_supported = false;
        let mut session = TerminalSession::acquire(control).unwrap();

        assert_eq!(session.keyboard_protocol(), KeyboardProtocol::Legacy);
        session.finish(Ok(())).unwrap();
        assert_eq!(
            &*calls.borrow(),
            &[
                "enable_raw",
                "detect_keys",
                "enter_alt",
                "show_cursor",
                "leave_alt",
                "disable_raw"
            ]
        );
    }

    #[test]
    fn keyboard_detection_failure_falls_back_to_legacy_input() {
        let (control, calls) = FakeControl::new(Some("detect_keys"));
        let mut session = TerminalSession::acquire(control).unwrap();

        assert_eq!(session.keyboard_protocol(), KeyboardProtocol::Legacy);
        session.finish(Ok(())).unwrap();
        assert_eq!(
            &*calls.borrow(),
            &[
                "enable_raw",
                "detect_keys",
                "enter_alt",
                "show_cursor",
                "leave_alt",
                "disable_raw"
            ]
        );
    }

    #[test]
    fn restores_after_draw_and_event_loop_errors_without_hiding_them() {
        for failure in ["draw failed", "event loop failed"] {
            let (control, calls) = FakeControl::new(None);
            let mut session = TerminalSession::acquire(control).unwrap();

            let error = session.finish::<()>(Err(anyhow!(failure))).unwrap_err();

            assert_eq!(error.to_string(), failure);
            assert_eq!(
                &*calls.borrow(),
                &[
                    "enable_raw",
                    "detect_keys",
                    "enter_alt",
                    "enable_keys",
                    "disable_keys",
                    "show_cursor",
                    "leave_alt",
                    "disable_raw"
                ]
            );
        }
    }

    #[test]
    fn restoration_attempts_every_step_when_one_cleanup_step_fails() {
        let (control, calls) = FakeControl::new(Some("show_cursor"));
        let mut session = TerminalSession::acquire(control).unwrap();

        let error = session
            .finish::<()>(Err(anyhow!("draw failed")))
            .unwrap_err();

        assert_eq!(error.to_string(), "draw failed");
        assert_eq!(
            &*calls.borrow(),
            &[
                "enable_raw",
                "detect_keys",
                "enter_alt",
                "enable_keys",
                "disable_keys",
                "show_cursor",
                "leave_alt",
                "disable_raw"
            ]
        );
    }

    #[test]
    fn drop_restores_terminal_during_panic_unwinding() {
        let (control, calls) = FakeControl::new(None);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _session = TerminalSession::acquire(control).unwrap();
            panic!("original panic diagnostic");
        }));

        assert!(panic.is_err());
        assert_eq!(
            &*calls.borrow(),
            &[
                "enable_raw",
                "detect_keys",
                "enter_alt",
                "enable_keys",
                "disable_keys",
                "show_cursor",
                "leave_alt",
                "disable_raw"
            ]
        );
    }

    #[test]
    fn recognizes_ctrl_c_as_an_interrupt() {
        assert!(is_interrupt(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn maps_terminal_editor_commands_to_semantic_keys() {
        for (code, modifiers, expected) in [
            (KeyCode::Enter, KeyModifiers::CONTROL, Key::Submit),
            (KeyCode::Char('j'), KeyModifiers::CONTROL, Key::Submit),
            (KeyCode::Char('s'), KeyModifiers::CONTROL, Key::Other),
            (KeyCode::Left, KeyModifiers::NONE, Key::Left),
            (KeyCode::Right, KeyModifiers::ALT, Key::WordRight),
            (KeyCode::Left, KeyModifiers::CONTROL, Key::WordLeft),
            (KeyCode::Delete, KeyModifiers::NONE, Key::Delete),
            (
                KeyCode::Backspace,
                KeyModifiers::ALT,
                Key::DeleteWordBackward,
            ),
        ] {
            assert_eq!(physical_input(KeyEvent::new(code, modifiers)).key, expected);
        }
    }
}
