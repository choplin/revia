use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    input::{BindingResolution, Key, KeyPhase, PhysicalInput},
    semantic::{HelpOverlay, Overlay},
};

const MAX_OVERLAY_HEIGHT: u16 = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Context {
    #[default]
    Review,
    Threads,
    Files,
    SearchResults,
}

#[derive(Debug)]
pub struct Model {
    context: Context,
    scroll: usize,
    viewport_rows: u16,
    viewport_columns: u16,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            context: Context::Review,
            scroll: 0,
            viewport_rows: 20,
            viewport_columns: 80,
        }
    }
}

impl Model {
    pub fn begin(&mut self, context: Context) {
        self.context = context;
        self.scroll = 0;
        self.clamp();
    }

    pub fn set_viewport(&mut self, rows: u16, columns: u16) {
        self.viewport_rows = rows.max(1);
        self.viewport_columns = columns.max(1);
        self.clamp();
    }

    fn content(&self) -> Vec<String> {
        wrap_help_text(&help_text(self.context), self.content_width())
    }

    fn content_width(&self) -> usize {
        let overlay_width = self
            .viewport_columns
            .saturating_mul(86)
            .saturating_div(100)
            .max(24)
            .min(self.viewport_columns);
        usize::from(overlay_width.saturating_sub(2).max(1))
    }

    fn visible_rows(&self) -> usize {
        let terminal_rows = self.viewport_rows.saturating_add(5);
        let overlay_rows = MAX_OVERLAY_HEIGHT
            .min(terminal_rows.saturating_sub(2))
            .max(3);
        usize::from(overlay_rows.saturating_sub(3).max(1))
    }

    fn max_scroll(&self) -> usize {
        self.content().len().saturating_sub(self.visible_rows())
    }

    fn clamp(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn scroll_by(&mut self, delta: i16) {
        self.scroll = if delta < 0 {
            self.scroll
                .saturating_sub(usize::from(delta.unsigned_abs()))
        } else {
            self.scroll.saturating_add(delta as usize)
        };
        self.clamp();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Close,
    MoveRows(i16),
    MovePage(i16),
    JumpToEdge { end: bool },
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
    if input.phase == KeyPhase::Release
        || input.phase == KeyPhase::Repeat && matches!(input.key, Key::Esc | Key::Char('?'))
    {
        return BindingResolution::Consume;
    }
    match input.key {
        Key::Esc | Key::Char('?') => BindingResolution::Override(Event::Close),
        Key::Char('j') | Key::Down => BindingResolution::Override(Event::MoveRows(1)),
        Key::Char('k') | Key::Up => BindingResolution::Override(Event::MoveRows(-1)),
        Key::Char('f') | Key::PageDown => BindingResolution::Override(Event::MovePage(1)),
        Key::Char('b') | Key::PageUp => BindingResolution::Override(Event::MovePage(-1)),
        Key::Char('g') | Key::Home => BindingResolution::Override(Event::JumpToEdge { end: false }),
        Key::Char('G') | Key::End => BindingResolution::Override(Event::JumpToEdge { end: true }),
        _ => BindingResolution::Consume,
    }
}

pub fn update(model: &mut Model, event: Event) -> Update {
    let mut result = Update::default();
    match event {
        Event::Close => {
            result
                .intents
                .push(Intent::SetStatus("closed keyboard help"));
            result.intents.push(Intent::Close);
        }
        Event::MoveRows(delta) => model.scroll_by(delta),
        Event::MovePage(direction) => {
            let page = model.visible_rows().min(i16::MAX as usize) as i16;
            model.scroll_by(direction.saturating_mul(page));
        }
        Event::JumpToEdge { end } => {
            model.scroll = if end { model.max_scroll() } else { 0 };
        }
    }
    result
}

pub fn view(model: &Model) -> Overlay {
    let lines = model.content();
    let total = lines.len();
    let visible = model.visible_rows();
    let scroll = model.scroll.min(total.saturating_sub(visible));
    let first = total.min(scroll.saturating_add(1));
    let last = total.min(scroll.saturating_add(visible));
    Overlay::Help(HelpOverlay {
        lines,
        scroll,
        position_hint: format!(
            "{first}-{last}/{total}  j/k scroll  f/b page  g/G edges  Esc/? close"
        ),
    })
}

fn help_text(context: Context) -> String {
    let context_label = match context {
        Context::Review => "diff",
        Context::Threads => "inline thread",
        Context::Files => "file rail",
        Context::SearchResults => "search results",
    };
    let search = marker(context == Context::SearchResults);
    let thread = marker(context == Context::Threads);
    let rail = marker(context == Context::Files);
    let stream = marker(context != Context::Files);
    let vertical_navigation = if context == Context::Files {
        "Previous / next file-tree item"
    } else {
        "Move by row"
    };
    let search_exit = match context {
        Context::SearchResults => "Cancel search",
        Context::Threads => "Return to the diff",
        Context::Review | Context::Files => "Quit review",
    };
    format!(
        "Commands from: {context_label}\n◆ available here   · unavailable in this context\n\nNavigation\n◆ j / k, ↑ / ↓ — {vertical_navigation}\n◆ f / Space, b — Page down / up\n◆ d / u — Half-page down / up\n◆ g / G — Jump to first / last diff row\n◆ [ / ] — Previous / next hunk\n{stream} , / . — Previous / next file\n{rail} , / . — Previous / next file-tree page\n{rail} < / >, Home / End — Jump to first / last file-tree item\n◆ / — Search the full diff\n{search} n / N — Previous / next match (wraps)\n\nView\n◆ F — Cycle review filter\n◆ A — Show all changes\n◆ 1 / 2 / 0 — Split / stack / automatic layout\n◆ s — Toggle file list\n{rail} Enter — Open a file or collapse / expand a directory\n{rail} ← / → — Switch between file tree and diff\n{rail} ` — Toggle flat / tree file view\n{rail} - / = — Collapse / expand all file directories\n◆ m — Toggle file headers\n◆ w — Toggle line wrapping\n{stream} = / - — More / less diff context\n◆ r — Reload and keep the current filter\n\nReview actions\n◆ Tab — Switch between diff and file list\n◆ t / T — Open next / previous thread\n◆ c / C — Reply / start a comment\n{thread} x / R — Resolve / reopen the selected thread\n{thread} a / o — Set attention / open flags\n{thread} e — Fold resolved comments\n◆ {{ / }} — Previous / next item needing attention\n◆ v — Open thread rollup\n\nGlobal / exit\n◆ ? — Open or close this help\n◆ q — Quit review\n◆ Esc — {search_exit}\n\nIn help\n◆ j / k — Scroll by row\n◆ f / b — Scroll by page\n◆ g / G — Jump to top / bottom\n◆ Esc / ? — Close help and return to {context_label}"
    )
}

fn marker(valid: bool) -> &'static str {
    if valid { "◆" } else { "·" }
}

fn wrap_help_text(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            rows.push(String::new());
            continue;
        }
        let mut row = String::new();
        let mut used = 0;
        for grapheme in line.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if !row.is_empty() && used + grapheme_width > width {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            if grapheme_width <= width {
                row.push_str(grapheme);
                used += grapheme_width;
            }
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use crate::input::{Key, KeyPhase, PhysicalInput};
    use crate::semantic::Overlay;

    use super::{Context, Event, Model, bindings, help_text, update, view};

    fn input(key: Key) -> PhysicalInput {
        PhysicalInput {
            key,
            shift: false,
            phase: KeyPhase::Press,
        }
    }

    #[test]
    fn navigation_reaches_every_help_section_at_wide_narrow_and_minimum_sizes() {
        for (rows, columns) in [(19, 120), (15, 48), (3, 48)] {
            let mut model = Model::default();
            model.set_viewport(rows, columns);
            model.begin(Context::Threads);
            let Overlay::Help(first) = view(&model) else {
                panic!("help overlay")
            };
            assert_eq!(first.scroll, 0);
            assert!(first.lines.iter().any(|line| line.contains("Navigation")));

            let global_row = first
                .lines
                .iter()
                .position(|line| line.contains("Global / exit"))
                .expect("global section");
            for _ in 0..global_row {
                update(&mut model, Event::MoveRows(1));
            }
            let Overlay::Help(global) = view(&model) else {
                panic!("help overlay")
            };
            let visible = &global.lines
                [global.scroll..global.lines.len().min(global.scroll + model.visible_rows())];
            assert!(visible.iter().any(|line| line.contains("Global / exit")));

            update(&mut model, Event::JumpToEdge { end: true });
            let Overlay::Help(last) = view(&model) else {
                panic!("help overlay")
            };
            assert!(last.scroll > 0);
            let visible = &last.lines[last.scroll..];
            assert!(visible.iter().any(|line| line.contains("Close help")));
        }
    }

    #[test]
    fn help_bindings_scroll_locally_and_keep_close_on_press_only() {
        for (key, expected) in [
            (Key::Char('j'), Event::MoveRows(1)),
            (Key::Down, Event::MoveRows(1)),
            (Key::Char('k'), Event::MoveRows(-1)),
            (Key::Up, Event::MoveRows(-1)),
            (Key::Char('f'), Event::MovePage(1)),
            (Key::PageDown, Event::MovePage(1)),
            (Key::Char('b'), Event::MovePage(-1)),
            (Key::PageUp, Event::MovePage(-1)),
            (Key::Char('g'), Event::JumpToEdge { end: false }),
            (Key::Home, Event::JumpToEdge { end: false }),
            (Key::Char('G'), Event::JumpToEdge { end: true }),
            (Key::End, Event::JumpToEdge { end: true }),
        ] {
            assert_eq!(
                bindings(input(key)),
                crate::input::BindingResolution::Override(expected)
            );
        }
        let mut repeated = input(Key::Esc);
        repeated.phase = KeyPhase::Repeat;
        assert!(matches!(
            bindings(repeated),
            crate::input::BindingResolution::Consume
        ));
    }

    #[test]
    fn review_help_names_the_current_comment_view_and_wrap_commands() {
        let help = help_text(Context::Review);
        assert!(help.contains("c / C — Reply / start a comment"));
        assert!(help.contains("1 / 2 / 0 — Split / stack / automatic layout"));
        assert!(help.contains("w — Toggle line wrapping"));
        assert!(!help.contains("Esc quit review"));
    }

    #[test]
    fn begin_resets_scroll_and_resize_clamps_it_to_the_new_viewport() {
        let mut model = Model::default();
        model.set_viewport(3, 48);
        model.begin(Context::Review);
        update(&mut model, Event::JumpToEdge { end: true });
        assert!(model.scroll > 0);

        model.set_viewport(19, 120);
        assert!(model.scroll <= model.max_scroll());
        model.begin(Context::SearchResults);
        assert_eq!(model.scroll, 0);
    }
}
