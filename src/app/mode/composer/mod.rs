//! Comment composer mode-local Elm program.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{
        input::{BindingResolution, Key, KeyboardProtocol, PhysicalInput},
        view::{ComposerOverlay, Overlay},
    },
    domain::{
        anchor::{AnchorBasis, HunkLocation},
        thread::{ThreadChange, ThreadId, ThreadOperation, ThreadState, ThreadSuccess},
    },
};

const MAX_EDITOR_ROWS: usize = 8;

#[derive(Debug)]
pub struct Model {
    input: String,
    cursor: usize,
    reply_to: Option<ThreadId>,
    viewport_rows: u16,
    viewport_columns: u16,
    scroll: usize,
    cancel_armed: bool,
    submitting: bool,
    validation: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            reply_to: None,
            viewport_rows: 20,
            viewport_columns: 80,
            scroll: 0,
            cancel_armed: false,
            submitting: false,
            validation: None,
        }
    }
}

impl Model {
    pub fn begin(&mut self, reply_to: Option<ThreadId>) {
        self.reset();
        self.reply_to = reply_to;
    }

    pub fn set_viewport(&mut self, rows: u16, columns: u16) {
        self.viewport_rows = rows.max(1);
        self.viewport_columns = columns.max(1);
        self.ensure_cursor_visible();
    }

    fn reset(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.reply_to = None;
        self.scroll = 0;
        self.cancel_armed = false;
        self.submitting = false;
        self.validation = None;
    }

    #[cfg(test)]
    pub fn input(&self) -> &str {
        &self.input
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn reply_to(&self) -> Option<ThreadId> {
        self.reply_to
    }

    fn edit(&mut self, change: impl FnOnce(&mut String, &mut usize)) {
        if self.submitting {
            self.validation = Some("Posting is in progress; wait for the result.".into());
            return;
        }
        change(&mut self.input, &mut self.cursor);
        self.cancel_armed = false;
        self.validation = None;
        self.ensure_cursor_visible();
    }

    fn move_cursor(&mut self, cursor: usize) {
        if self.submitting {
            return;
        }
        self.cursor = cursor.min(self.input.len());
        self.cancel_armed = false;
        self.validation = None;
        self.ensure_cursor_visible();
    }

    fn editor_width(&self) -> usize {
        let overlay_width = self
            .viewport_columns
            .saturating_mul(70)
            .saturating_div(100)
            .max(24)
            .min(self.viewport_columns);
        usize::from(overlay_width.saturating_sub(2).max(1))
    }

    fn editor_row_capacity(&self) -> usize {
        usize::from(self.viewport_rows.saturating_sub(4).max(1)).min(MAX_EDITOR_ROWS)
    }

    fn ensure_cursor_visible(&mut self) {
        let layout = visual_layout(&self.input, self.cursor, self.editor_width());
        let capacity = self.editor_row_capacity();
        if layout.cursor_row < self.scroll {
            self.scroll = layout.cursor_row;
        } else if layout.cursor_row >= self.scroll.saturating_add(capacity) {
            self.scroll = layout.cursor_row.saturating_sub(capacity.saturating_sub(1));
        }
        self.scroll = self.scroll.min(layout.lines.len().saturating_sub(capacity));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Cancel,
    Submit,
    InsertNewline,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    MoveLeft,
    MoveRight,
    MoveWordLeft,
    MoveWordRight,
    MoveUp,
    MoveDown,
    MoveHome,
    MoveEnd,
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
    pub anchor_basis: AnchorBasis,
    pub selected_location: Option<HunkLocation>,
    pub operation_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Close,
    FocusThread(ThreadId),
    SetStatus(String),
    ReplaceThreads(ThreadState),
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    if input.phase == crate::app::input::KeyPhase::Release
        || input.phase == crate::app::input::KeyPhase::Repeat
            && matches!(input.key, Key::Esc | Key::Submit)
    {
        return BindingResolution::Consume;
    }
    let event = match input.key {
        Key::Esc => Event::Cancel,
        Key::Submit => Event::Submit,
        Key::Enter => Event::InsertNewline,
        Key::Backspace => Event::DeleteBackward,
        Key::Delete => Event::DeleteForward,
        Key::DeleteWordBackward => Event::DeleteWordBackward,
        Key::Left => Event::MoveLeft,
        Key::Right => Event::MoveRight,
        Key::WordLeft => Event::MoveWordLeft,
        Key::WordRight => Event::MoveWordRight,
        Key::Up => Event::MoveUp,
        Key::Down => Event::MoveDown,
        Key::Home => Event::MoveHome,
        Key::End => Event::MoveEnd,
        Key::Char(character) => Event::InsertCharacter(character),
        _ => return BindingResolution::Consume,
    };
    if matches!(event, Event::Cancel | Event::InsertCharacter(_)) {
        BindingResolution::Override(event)
    } else {
        BindingResolution::Handle(event)
    }
}

pub fn update(model: &mut Model, event: Event, input: UpdateInput) -> Update {
    let mut result = Update::default();
    match event {
        Event::Cancel => cancel(model, &mut result),
        Event::Submit => submit(model, input, &mut result),
        Event::InsertNewline => model.edit(|text, cursor| insert(text, cursor, '\n')),
        Event::DeleteBackward => model.edit(delete_backward),
        Event::DeleteForward => model.edit(delete_forward),
        Event::DeleteWordBackward => model.edit(delete_word_backward),
        Event::MoveLeft => model.move_cursor(previous_grapheme(&model.input, model.cursor)),
        Event::MoveRight => model.move_cursor(next_grapheme(&model.input, model.cursor)),
        Event::MoveWordLeft => model.move_cursor(previous_word(&model.input, model.cursor)),
        Event::MoveWordRight => model.move_cursor(next_word(&model.input, model.cursor)),
        Event::MoveUp => model.move_cursor(vertical_cursor(&model.input, model.cursor, -1)),
        Event::MoveDown => model.move_cursor(vertical_cursor(&model.input, model.cursor, 1)),
        Event::MoveHome => model.move_cursor(line_start(&model.input, model.cursor)),
        Event::MoveEnd => model.move_cursor(line_end(&model.input, model.cursor)),
        Event::InsertCharacter(character) => {
            model.edit(|text, cursor| insert(text, cursor, character));
        }
        Event::EffectCompleted(Outcome::ThreadsChanged { result: outcome }) => {
            model.submitting = false;
            match outcome {
                Ok(change) => {
                    let focus_thread = match change.success {
                        ThreadSuccess::Posted(id) => Some(id),
                        ThreadSuccess::Replied => model.reply_to,
                        ThreadSuccess::Closed
                        | ThreadSuccess::Reopened
                        | ThreadSuccess::AttentionToggled
                        | ThreadSuccess::OutdatedToggled => None,
                    };
                    model.reset();
                    status(&mut result, success_status(change.success));
                    result.intents.push(Intent::ReplaceThreads(change.state));
                    if let Some(id) = focus_thread {
                        result.intents.push(Intent::FocusThread(id));
                    }
                    result.intents.push(Intent::Close);
                }
                Err(error) => {
                    model.validation = Some("Post failed. Draft preserved; Ctrl-J retries.".into());
                    status(&mut result, error);
                }
            }
        }
    }
    result
}

fn cancel(model: &mut Model, result: &mut Update) {
    if model.submitting {
        model.validation = Some("Posting is in progress; wait for the result.".into());
    } else if model.input.is_empty() || model.cancel_armed {
        model.reset();
        status(result, "cancelled comment draft");
        result.intents.push(Intent::Close);
    } else {
        model.cancel_armed = true;
        model.validation = Some("Unsaved draft. Press Esc again to discard it.".into());
        status(result, "press Esc again to discard the comment draft");
    }
}

fn submit(model: &mut Model, input: UpdateInput, result: &mut Update) {
    model.cancel_armed = false;
    if model.submitting || input.operation_pending {
        model.validation = Some("Another operation is still in progress.".into());
        status(result, "cannot post while another operation is pending");
        return;
    }
    if model.input.trim().is_empty() {
        model.validation = Some("Message cannot be empty.".into());
        status(result, "comment cannot be empty");
        return;
    }
    let Some(location) = input.selected_location else {
        model.validation = Some("The target hunk is unavailable. Draft preserved.".into());
        status(result, "select a hunk before posting a comment");
        return;
    };
    model.submitting = true;
    model.validation = Some("Posting…".into());
    result
        .effects
        .push(Effect::ChangeThreads(ThreadOperation::Submit {
            anchor_basis: Box::new(input.anchor_basis),
            location,
            body: model.input.clone(),
            reply_to: model.reply_to,
        }));
}

fn insert(text: &mut String, cursor: &mut usize, character: char) {
    text.insert(*cursor, character);
    *cursor = grapheme_boundary_at_or_after(text, *cursor + character.len_utf8());
}

fn grapheme_boundary_at_or_after(text: &str, tentative: usize) -> usize {
    UnicodeSegmentation::grapheme_indices(text, true)
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .find(|boundary| *boundary >= tentative)
        .unwrap_or(text.len())
}

fn delete_backward(text: &mut String, cursor: &mut usize) {
    let previous = previous_grapheme(text, *cursor);
    if previous < *cursor {
        text.drain(previous..*cursor);
        *cursor = previous;
    }
}

fn delete_forward(text: &mut String, cursor: &mut usize) {
    let next = next_grapheme(text, *cursor);
    if next > *cursor {
        text.drain(*cursor..next);
    }
}

fn delete_word_backward(text: &mut String, cursor: &mut usize) {
    let previous = previous_word(text, *cursor);
    if previous < *cursor {
        text.drain(previous..*cursor);
        *cursor = previous;
    }
}

fn graphemes(text: &str) -> Vec<(usize, &str)> {
    UnicodeSegmentation::grapheme_indices(text, true).collect()
}

fn previous_grapheme(text: &str, cursor: usize) -> usize {
    graphemes(&text[..cursor])
        .last()
        .map_or(cursor, |(index, _)| *index)
}

fn next_grapheme(text: &str, cursor: usize) -> usize {
    UnicodeSegmentation::grapheme_indices(&text[cursor..], true)
        .next()
        .map_or(cursor, |(_, grapheme)| cursor + grapheme.len())
}

fn is_word(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
}

fn previous_word(text: &str, cursor: usize) -> usize {
    let items = graphemes(&text[..cursor]);
    let mut index = items.len();
    while index > 0 && !is_word(items[index - 1].1) {
        index -= 1;
    }
    while index > 0 && is_word(items[index - 1].1) {
        index -= 1;
    }
    items.get(index).map_or(cursor, |(offset, _)| *offset)
}

fn next_word(text: &str, cursor: usize) -> usize {
    let items = graphemes(&text[cursor..]);
    let mut index = 0;
    while index < items.len() && is_word(items[index].1) {
        index += 1;
    }
    while index < items.len() && !is_word(items[index].1) {
        index += 1;
    }
    items
        .get(index)
        .map_or(text.len(), |(offset, _)| cursor + *offset)
}

fn line_start(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind('\n').map_or(0, |index| index + 1)
}

fn line_end(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .find('\n')
        .map_or(text.len(), |index| cursor + index)
}

fn vertical_cursor(text: &str, cursor: usize, direction: i8) -> usize {
    let start = line_start(text, cursor);
    let column = UnicodeWidthStr::width(&text[start..cursor]);
    let target = if direction < 0 {
        if start == 0 {
            return cursor;
        }
        let end = start - 1;
        (line_start(text, end), end)
    } else {
        let end = line_end(text, cursor);
        if end == text.len() {
            return cursor;
        }
        let start = end + 1;
        (start, line_end(text, start))
    };
    cursor_at_column(text, target.0, target.1, column)
}

fn cursor_at_column(text: &str, start: usize, end: usize, column: usize) -> usize {
    let mut width = 0;
    for (offset, grapheme) in UnicodeSegmentation::grapheme_indices(&text[start..end], true) {
        let next = width + UnicodeWidthStr::width(grapheme);
        if next > column {
            return start + offset;
        }
        width = next;
    }
    end
}

struct VisualLayout {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_column: usize,
}

fn visual_layout(text: &str, cursor: usize, width: usize) -> VisualLayout {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width: usize = 0;
    let mut cursor_position = None;
    for (index, grapheme) in UnicodeSegmentation::grapheme_indices(text, true) {
        if grapheme == "\n" {
            let wrapped = line_width >= width;
            if wrapped {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            if cursor == index {
                cursor_position = Some((lines.len(), line_width));
            }
            if !wrapped {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            continue;
        }
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if line_width > 0 && line_width.saturating_add(grapheme_width) > width {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        if cursor == index {
            cursor_position = Some((lines.len(), line_width));
        }
        line.push_str(grapheme);
        line_width = line_width.saturating_add(grapheme_width);
    }
    if cursor == text.len() && line_width >= width {
        lines.push(std::mem::take(&mut line));
        line_width = 0;
    }
    if cursor == text.len() {
        cursor_position = Some((lines.len(), line_width));
    }
    lines.push(line);
    let (cursor_row, cursor_column) = cursor_position.unwrap_or((0, 0));
    VisualLayout {
        lines,
        cursor_row,
        cursor_column,
    }
}

fn status(result: &mut Update, message: impl Into<String>) {
    result.intents.push(Intent::SetStatus(message.into()));
}

fn success_status(success: ThreadSuccess) -> String {
    match success {
        ThreadSuccess::Posted(id) => format!("posted comment #{id}"),
        ThreadSuccess::Replied => "posted reply".into(),
        ThreadSuccess::Closed => "thread closed".into(),
        ThreadSuccess::Reopened => "thread reopened".into(),
        ThreadSuccess::AttentionToggled => "needs-attention toggled".into(),
        ThreadSuccess::OutdatedToggled => "outdated toggled".into(),
    }
}

pub fn view(model: &Model, target: Option<&str>, keyboard_protocol: KeyboardProtocol) -> Overlay {
    let layout = visual_layout(&model.input, model.cursor, model.editor_width());
    let visible_rows = layout.lines.len().min(model.editor_row_capacity()).max(1);
    Overlay::Composer(ComposerOverlay {
        context: model.reply_to.map_or_else(
            || target.map_or_else(|| "Comment".into(), |target| format!("Comment • {target}")),
            |id| format!("Reply • thread #{id}"),
        ),
        lines: layout.lines,
        cursor_row: layout.cursor_row,
        cursor_column: layout.cursor_column,
        scroll: model.scroll,
        height: u16::try_from(visible_rows.saturating_add(3)).unwrap_or(u16::MAX),
        message: model.validation.clone(),
        instructions: if keyboard_protocol.supports_ctrl_enter() {
            "Ctrl-Enter/Ctrl-J post · Enter newline · Esc cancel".into()
        } else {
            "Ctrl-J post · Enter newline · Esc cancel".into()
        },
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        app::input::{BindingResolution, Key, KeyPhase, KeyboardProtocol, PhysicalInput},
        domain::anchor::AnchorBasis,
    };

    use super::{Event, Model, Overlay, UpdateInput, view};

    fn input(key: Key) -> PhysicalInput {
        PhysicalInput {
            key,
            shift: false,
            phase: KeyPhase::Press,
        }
    }

    fn update(model: &mut Model, event: Event) -> super::Update {
        super::update(
            model,
            event,
            UpdateInput {
                anchor_basis: AnchorBasis::Unavailable,
                selected_location: None,
                operation_pending: false,
            },
        )
    }

    #[test]
    fn character_overrides_global_shortcuts_in_composer() {
        assert_eq!(
            super::bindings(input(Key::Char('q'))),
            BindingResolution::Override(Event::InsertCharacter('q'))
        );
    }

    #[test]
    fn edits_at_grapheme_boundaries_and_moves_by_unicode_words() {
        let mut model = Model::default();
        for character in "cafe\u{301} 画面".chars() {
            update(&mut model, Event::InsertCharacter(character));
        }
        update(&mut model, Event::MoveWordLeft);
        assert_eq!(model.cursor(), "cafe\u{301} ".len());
        update(&mut model, Event::DeleteWordBackward);
        assert_eq!(model.input(), "画面");
        assert_eq!(model.cursor(), 0);
        update(&mut model, Event::DeleteForward);
        assert_eq!(model.input(), "面");
    }

    #[test]
    fn insertion_preserves_context_sensitive_grapheme_boundaries() {
        let mut joined = Model::default();
        for character in "👩👩".chars() {
            update(&mut joined, Event::InsertCharacter(character));
        }
        update(&mut joined, Event::MoveLeft);
        update(&mut joined, Event::InsertCharacter('\u{200d}'));
        assert_eq!(joined.input(), "👩‍👩");
        assert_eq!(joined.cursor(), joined.input().len());
        update(&mut joined, Event::DeleteBackward);
        assert_eq!(joined.input(), "");

        let mut combining = Model::default();
        for character in "ex".chars() {
            update(&mut combining, Event::InsertCharacter(character));
        }
        update(&mut combining, Event::MoveLeft);
        update(&mut combining, Event::InsertCharacter('\u{301}'));
        assert_eq!(combining.input(), "e\u{301}x");
        assert_eq!(combining.cursor(), "e\u{301}".len());
        update(&mut combining, Event::DeleteBackward);
        assert_eq!(combining.input(), "x");

        let mut regional_indicators = Model::default();
        for character in "🇯🇵🇺".chars() {
            update(&mut regional_indicators, Event::InsertCharacter(character));
        }
        update(&mut regional_indicators, Event::MoveLeft);
        update(&mut regional_indicators, Event::InsertCharacter('🇸'));
        assert_eq!(regional_indicators.input(), "🇯🇵🇸🇺");
        assert_eq!(
            regional_indicators.cursor(),
            regional_indicators.input().len()
        );
        update(&mut regional_indicators, Event::DeleteBackward);
        assert_eq!(regional_indicators.input(), "🇯🇵");
    }

    #[test]
    fn exact_width_line_places_newline_boundary_on_the_continuation_row() {
        let text = format!("{}\n", "x".repeat(8));
        let layout = super::visual_layout(&text, 8, 8);

        assert_eq!(layout.lines, vec!["xxxxxxxx", ""]);
        assert_eq!((layout.cursor_row, layout.cursor_column), (1, 0));
    }

    #[test]
    fn visual_layout_wraps_wide_graphemes_and_keeps_cursor_visible() {
        let mut model = Model::default();
        model.set_viewport(6, 24);
        for character in "abc画面\nsecond\nthird\nfourth".chars() {
            update(&mut model, Event::InsertCharacter(character));
        }
        let super::Overlay::Composer(overlay) = super::view(&model, None, KeyboardProtocol::Legacy)
        else {
            panic!("composer overlay")
        };
        assert!(overlay.lines.len() > 2);
        assert!(overlay.cursor_row >= overlay.scroll);
        assert!(overlay.cursor_row < overlay.scroll + 2);
    }

    #[test]
    fn instructions_only_advertise_ctrl_enter_for_enhanced_input() {
        let model = Model::default();
        let Overlay::Composer(legacy) = view(&model, None, KeyboardProtocol::Legacy) else {
            panic!("composer overlay")
        };
        let Overlay::Composer(kitty) = view(&model, None, KeyboardProtocol::Kitty) else {
            panic!("composer overlay")
        };

        assert_eq!(
            legacy.instructions,
            "Ctrl-J post · Enter newline · Esc cancel"
        );
        assert_eq!(
            kitty.instructions,
            "Ctrl-Enter/Ctrl-J post · Enter newline · Esc cancel"
        );
    }
}
