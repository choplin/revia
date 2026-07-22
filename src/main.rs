mod anchor;
mod diff;
mod thread;
mod ui;

use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use anchor::AnchorStore;
use anyhow::Result;
use clap::{ArgGroup, Parser};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use diff::{DiffFile, DiffLine, DiffLineKind, DiffRequest, DiffTarget, LoadedDiff};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
};
use thread::{Participant, ParticipantKind, ThreadStore};
use ui::{FocusArea, LayoutMode, ViewState};

/// Review Git diffs in the terminal without changing the repository.
#[derive(Debug, Parser)]
#[command(version, about)]
#[command(group(
    ArgGroup::new("target")
        .args(["staged", "commit", "range"])
        .multiple(false)
))]
struct Args {
    /// Git repository to inspect (defaults to the current directory).
    #[arg(short, long, default_value = ".")]
    repo: PathBuf,

    /// Show the index diff instead of the working-tree diff.
    #[arg(short, long)]
    staged: bool,

    /// Show the patch introduced by one commit or revision.
    #[arg(short, long)]
    commit: Option<String>,

    /// Show a Git revision range, for example main...HEAD.
    #[arg(short = 'R', long)]
    range: Option<String>,

    /// Number of unchanged lines surrounding each hunk.
    #[arg(short = 'U', long, default_value_t = 3)]
    context: usize,

    /// Write the raw Git diff to stdout instead of opening the TUI.
    #[arg(long)]
    print: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let request = DiffRequest {
        target: target_from(&args),
        context_lines: args.context,
    };
    let diff = LoadedDiff::load(&args.repo, &request)?;

    if args.print || !io::IsTerminal::is_terminal(&io::stdout()) {
        print!("{}", diff.text);
        return Ok(());
    }

    run_tui(App::new(args.repo, request, diff)?)
}

fn target_from(args: &Args) -> DiffTarget {
    if args.staged {
        DiffTarget::Staged
    } else if let Some(commit) = &args.commit {
        DiffTarget::Commit(commit.clone())
    } else if let Some(range) = &args.range {
        DiffTarget::Range(range.clone())
    } else {
        DiffTarget::WorkingTree
    }
}

struct App {
    repository: PathBuf,
    request: DiffRequest,
    diff: LoadedDiff,
    view: ViewState,
    status: Option<String>,
    syntax_set: SyntaxSet,
    theme: Theme,
    threads: ThreadStore,
    composer: Option<String>,
    reply_to: Option<u64>,
    rollup: bool,
    rollup_selected: usize,
    sidebar_visible: bool,
    show_hunk_headers: bool,
    wrap_lines: bool,
    show_help: bool,
    viewport_rows: u16,
}

impl App {
    fn new(repository: PathBuf, request: DiffRequest, diff: LoadedDiff) -> Result<Self> {
        let themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .get("base16-ocean.dark")
            .or_else(|| themes.themes.values().next())
            .expect("syntect includes a default theme")
            .clone();
        Ok(Self {
            threads: ThreadStore::open(&repository)?,
            repository,
            request,
            diff,
            view: ViewState::default(),
            status: None,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme,
            composer: None,
            reply_to: None,
            rollup: false,
            rollup_selected: 0,
            sidebar_visible: true,
            show_hunk_headers: true,
            wrap_lines: false,
            show_help: false,
            viewport_rows: 20,
        })
    }

    fn file(&self) -> Option<&DiffFile> {
        self.diff.document.files.get(self.view.selected_file)
    }

    fn move_file(&mut self, direction: i32) {
        let count = self.diff.document.files.len();
        if count == 0 {
            return;
        }
        self.view
            .select_file(wrapped_index(self.view.selected_file, count, direction));
        self.view.scroll = self.review_hunk_start_line();
    }

    /// Move through the review stream rather than stopping at a file boundary.
    fn move_review_hunk(&mut self, direction: i32) {
        let locations = self.hunk_locations();
        let Some(current) = locations.iter().position(|(file, hunk)| {
            *file == self.view.selected_file && *hunk == self.view.selected_hunk
        }) else {
            return;
        };
        let (file, hunk) = locations[wrapped_index(current, locations.len(), direction)];
        self.view.select_hunk(file, hunk, 0);
        self.view.reveal(self.review_hunk_start_line(), 20);
    }

    fn hunk_locations(&self) -> Vec<(usize, usize)> {
        self.diff
            .document
            .files
            .iter()
            .enumerate()
            .flat_map(|(file_index, file)| {
                (0..file.hunks.len()).map(move |hunk| (file_index, hunk))
            })
            .collect()
    }

    fn review_hunk_start_line(&self) -> u16 {
        let mut lines = 0usize;
        for (file_index, file) in self.diff.document.files.iter().enumerate() {
            lines += 2 + file.metadata.len();
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                if file_index == self.view.selected_file && hunk_index == self.view.selected_hunk {
                    return lines.try_into().unwrap_or(u16::MAX);
                }
                lines +=
                    1 + hunk.lines.len() + self.threads_for(&file.path, &hunk.header).len() * 3;
            }
        }
        0
    }

    fn adjust_context(&mut self, delta: i32) {
        let context = self.request.context_lines as i32 + delta;
        if context < 0 {
            return;
        }
        self.request.context_lines = context as usize;
        match LoadedDiff::load(&self.repository, &self.request) {
            Ok(diff) => {
                self.diff = diff;
                self.view.select_file(
                    self.view
                        .selected_file
                        .min(self.diff.document.files.len().saturating_sub(1)),
                );
                self.status = Some(format!("context: {} lines", self.request.context_lines));
            }
            Err(error) => self.status = Some(format!("could not reload diff: {error}")),
        }
    }

    fn scroll(&mut self, delta: i16) {
        self.view.scroll_by(delta);
    }

    fn scroll_viewport(&mut self, direction: i16) {
        self.scroll(direction.saturating_mul(self.viewport_rows as i16));
    }

    fn scroll_half_viewport(&mut self, direction: i16) {
        self.scroll(direction.saturating_mul((self.viewport_rows / 2).max(1) as i16));
    }

    fn jump_to_stream_edge(&mut self, end: bool) {
        self.view.scroll = if end { u16::MAX } else { 0 };
    }

    fn reload_current_diff(&mut self) {
        match LoadedDiff::load(&self.repository, &self.request) {
            Ok(diff) => {
                self.diff = diff;
                self.view.select_file(
                    self.view
                        .selected_file
                        .min(self.diff.document.files.len().saturating_sub(1)),
                );
                self.status = Some("reloaded current diff".into());
            }
            Err(error) => self.status = Some(format!("could not reload diff: {error}")),
        }
    }

    fn selected_location(&self) -> Option<(String, String)> {
        let file = self.file()?;
        let hunk = file.hunks.get(self.view.selected_hunk)?;
        Some((file.path.clone(), hunk.header.clone()))
    }

    /// Return all persisted threads at the selected immutable hunk anchor.
    fn threads_for(&self, path: &str, hunk_header: &str) -> Vec<&thread::ReviewThread> {
        self.threads
            .threads()
            .iter()
            .filter(|thread| thread.anchor.path == path && thread.anchor.hunk_header == hunk_header)
            .collect()
    }

    fn selected_threads(&self) -> Vec<&thread::ReviewThread> {
        self.selected_location()
            .map(|(path, hunk)| self.threads_for(&path, &hunk))
            .unwrap_or_default()
    }

    fn move_thread(&mut self, direction: i32) {
        let count = self.selected_threads().len();
        if count == 0 {
            self.status = Some("this hunk has no threads".into());
            return;
        }
        self.view.selected_thread = wrapped_index(self.view.selected_thread, count, direction);
        self.view.focus = FocusArea::Threads;
    }

    fn move_focus(&mut self) {
        self.view.focus = self.view.focus.next();
        if self.view.focus == FocusArea::Threads && self.selected_threads().is_empty() {
            self.view.focus = FocusArea::Files;
            self.status = Some("this hunk has no threads; focus moved to files".into());
        }
    }

    fn human() -> Participant {
        Participant {
            id: "human".into(),
            kind: ParticipantKind::Human,
        }
    }

    fn begin_thread(&mut self) {
        if self.selected_location().is_none() {
            self.status = Some("select a hunk before posting a thread".into());
            return;
        }
        self.reply_to = if self.view.focus == FocusArea::Threads {
            self.current_thread_id()
        } else {
            None
        };
        self.composer = Some(String::new());
    }

    fn begin_new_thread(&mut self) {
        if self.selected_location().is_none() {
            self.status = Some("select a hunk before posting a thread".into());
            return;
        }
        self.reply_to = None;
        self.composer = Some(String::new());
    }

    /// Handle composer-local keys before the root shortcut map gets a chance to
    /// see them. Returning true means the modal consumed the event.
    fn handle_composer_key(&mut self, code: KeyCode) -> bool {
        let Some(input) = self.composer.as_mut() else {
            return false;
        };
        match code {
            KeyCode::Esc => {
                self.composer = None;
                self.reply_to = None;
            }
            KeyCode::Enter => self.submit_thread(),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(character) => input.push(character),
            _ => {}
        }
        true
    }

    fn submit_thread(&mut self) {
        let Some(body) = self.composer.take() else {
            return;
        };
        let reply_to = self.reply_to.take();
        if body.trim().is_empty() {
            self.status = Some("thread message cannot be empty".into());
            return;
        }
        let result = if let Some(id) = reply_to {
            self.threads.reply(id, Self::human(), body).map(|()| None)
        } else {
            let Some((path, hunk_header)) = self.selected_location() else {
                return;
            };
            let anchors = AnchorStore::new(&self.repository);
            let anchor = match &self.request.target {
                DiffTarget::Commit(revision) => anchors.committed(revision, path, hunk_header),
                DiffTarget::WorkingTree | DiffTarget::Staged | DiffTarget::Range(_) => {
                    anchors.snapshot_working_tree(path, hunk_header)
                }
            };
            anchor
                .and_then(|anchor| self.threads.post(anchor, Self::human(), body))
                .map(Some)
        };
        match result {
            Ok(Some(id)) => self.status = Some(format!("posted thread #{id}")),
            Ok(None) => self.status = Some("posted reply".into()),
            Err(error) => self.status = Some(format!("could not post thread: {error}")),
        }
    }

    fn current_thread_id(&self) -> Option<u64> {
        let threads = self.selected_threads();
        threads
            .get(self.view.selected_thread)
            .or_else(|| threads.last())
            .map(|thread| thread.id)
    }

    fn close_thread(&mut self) {
        match self
            .current_thread_id()
            .ok_or_else(|| anyhow::anyhow!("no thread on this hunk"))
            .and_then(|id| self.threads.close(id, &Self::human()))
        {
            Ok(()) => self.status = Some("thread closed".into()),
            Err(error) => self.status = Some(format!("could not close thread: {error}")),
        }
    }

    fn reopen_thread(&mut self) {
        match self
            .current_thread_id()
            .ok_or_else(|| anyhow::anyhow!("no thread on this hunk"))
            .and_then(|id| self.threads.reopen(id))
        {
            Ok(()) => self.status = Some("thread reopened".into()),
            Err(error) => self.status = Some(format!("could not reopen thread: {error}")),
        }
    }

    fn toggle_attention(&mut self) {
        let result = self
            .current_thread_id()
            .ok_or_else(|| anyhow::anyhow!("no thread on this hunk"))
            .and_then(|id| {
                let active = self
                    .threads
                    .threads()
                    .iter()
                    .find(|thread| thread.id == id)
                    .is_some_and(|thread| thread.needs_attention);
                self.threads.set_needs_attention(id, !active)
            });
        match result {
            Ok(()) => self.status = Some("needs-attention toggled".into()),
            Err(error) => self.status = Some(format!("could not update thread: {error}")),
        }
    }

    fn toggle_outdated(&mut self) {
        let result = self
            .current_thread_id()
            .ok_or_else(|| anyhow::anyhow!("no thread on this hunk"))
            .and_then(|id| {
                let outdated = self
                    .threads
                    .threads()
                    .iter()
                    .find(|thread| thread.id == id)
                    .is_some_and(|thread| thread.outdated);
                self.threads.set_outdated(id, !outdated)
            });
        self.status = Some(match result {
            Ok(()) => "outdated toggled".into(),
            Err(error) => format!("could not update thread: {error}"),
        });
    }

    fn rollup_ids(&self) -> Vec<u64> {
        let mut threads = self.threads.threads().iter().collect::<Vec<_>>();
        threads.sort_by_key(|thread| {
            if thread.needs_attention {
                0
            } else if matches!(thread.resolution, thread::Resolution::Open) {
                1
            } else {
                2
            }
        });
        threads.into_iter().map(|thread| thread.id).collect()
    }

    fn move_rollup(&mut self, delta: i32) {
        let count = self.rollup_ids().len();
        if count > 0 {
            self.rollup_selected = wrapped_index(self.rollup_selected, count, delta);
        }
    }

    fn move_attention(&mut self, direction: i32) {
        let attention = self
            .rollup_ids()
            .into_iter()
            .filter(|id| {
                self.threads
                    .threads()
                    .iter()
                    .find(|thread| thread.id == *id)
                    .is_some_and(|thread| thread.needs_attention)
            })
            .collect::<Vec<_>>();
        if attention.is_empty() {
            self.status = Some("no needs-attention threads".into());
            return;
        }
        let current = self.current_thread_id();
        let index = current
            .and_then(|id| attention.iter().position(|candidate| *candidate == id))
            .unwrap_or(0);
        self.jump_to_thread(attention[wrapped_index(index, attention.len(), direction)]);
    }

    fn jump_to_thread(&mut self, id: u64) {
        let Some(thread) = self.threads.threads().iter().find(|thread| thread.id == id) else {
            return;
        };
        let anchor = thread.anchor.clone();
        if let Err(error) = AnchorStore::new(&self.repository).resolve_file(&anchor) {
            self.status = Some(format!("thread #{id} anchor cannot resolve: {error}"));
            return;
        }
        let Some((file_index, file)) = self
            .diff
            .document
            .files
            .iter()
            .enumerate()
            .find(|(_, file)| file.path == anchor.path)
        else {
            self.status = Some(format!("thread #{id} anchor is not in this diff"));
            return;
        };
        let Some(hunk_index) = file
            .hunks
            .iter()
            .position(|hunk| hunk.header == anchor.hunk_header)
        else {
            self.status = Some(format!("thread #{id} hunk is not in this diff"));
            return;
        };
        self.view.select_hunk(file_index, hunk_index, 0);
        self.view.selected_thread = self
            .threads_for(&anchor.path, &anchor.hunk_header)
            .iter()
            .position(|candidate| candidate.id == id)
            .unwrap_or(0);
        self.view.focus = FocusArea::Threads;
        self.view.scroll = self.review_hunk_start_line();
        self.rollup = false;
        self.status = Some(format!("thread #{id}"));
    }

    fn jump_to_rollup_thread(&mut self) {
        let Some(id) = self.rollup_ids().get(self.rollup_selected).copied() else {
            return;
        };
        self.jump_to_thread(id);
    }

    /// Hunk-compatible global command map. Returns true only for the terminal
    /// quit action; modal and rollup input are handled before this map.
    fn handle_global_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Tab => self.move_focus(),
            KeyCode::BackTab => self.view.focus = self.view.focus.previous(),
            KeyCode::Char('j') | KeyCode::Down => self.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
            KeyCode::Char('f') | KeyCode::PageDown => self.scroll_viewport(1),
            KeyCode::Char('b') | KeyCode::PageUp => self.scroll_viewport(-1),
            KeyCode::Char(' ') if modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_viewport(-1)
            }
            KeyCode::Char(' ') => self.scroll_viewport(1),
            KeyCode::Char('d') => self.scroll_half_viewport(1),
            KeyCode::Char('u') => self.scroll_half_viewport(-1),
            KeyCode::Char('g') | KeyCode::Home => self.jump_to_stream_edge(false),
            KeyCode::Char('G') | KeyCode::End => self.jump_to_stream_edge(true),
            KeyCode::Enter if self.view.focus == FocusArea::Files => {
                self.view.focus = FocusArea::Review;
                self.view.scroll = self.review_hunk_start_line();
            }
            KeyCode::Char(']') => self.move_review_hunk(1),
            KeyCode::Char('[') => self.move_review_hunk(-1),
            KeyCode::Char('.') => self.move_file(1),
            KeyCode::Char(',') => self.move_file(-1),
            KeyCode::Char('=') => self.adjust_context(1),
            KeyCode::Char('-') => self.adjust_context(-1),
            // Revia's scoped review extensions. They do not shadow a Hunk
            // navigation command.
            KeyCode::Char('c') => self.begin_thread(),
            KeyCode::Char('C') => self.begin_new_thread(),
            KeyCode::Char('t') => self.move_thread(0),
            KeyCode::Char('x') => self.close_thread(),
            KeyCode::Char('R') => self.reopen_thread(),
            KeyCode::Char('a') => self.toggle_attention(),
            KeyCode::Char('o') => self.toggle_outdated(),
            KeyCode::Char('}') => self.move_attention(1),
            KeyCode::Char('{') => self.move_attention(-1),
            KeyCode::Char('v') => self.rollup = true,
            KeyCode::Char('1') => self.view.layout = LayoutMode::Split,
            KeyCode::Char('2') => self.view.layout = LayoutMode::Stack,
            KeyCode::Char('0') => self.view.layout = LayoutMode::Auto,
            KeyCode::Char('s') => self.sidebar_visible = !self.sidebar_visible,
            KeyCode::Char('r') => self.reload_current_diff(),
            KeyCode::Char('m') => self.show_hunk_headers = !self.show_hunk_headers,
            KeyCode::Char('w') => self.wrap_lines = !self.wrap_lines,
            _ => {}
        }
        false
    }
}

fn wrapped_index(current: usize, length: usize, direction: i32) -> usize {
    ((current as i32 + direction).rem_euclid(length as i32)) as usize
}

fn run_tui(mut app: App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut last_scroll_at: Option<Instant> = None;
    loop {
        terminal.draw(|frame| render(frame, app))?;

        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            let is_scroll = matches!(
                key.code,
                KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up
            ) && app.composer.is_none()
                && !app.rollup;
            if is_scroll
                && key.kind == KeyEventKind::Repeat
                && last_scroll_at.is_some_and(|last| last.elapsed() < Duration::from_millis(28))
            {
                continue;
            }
            if is_scroll {
                last_scroll_at = Some(Instant::now());
            }
            if app.handle_composer_key(key.code) {
                continue;
            }
            if app.show_help {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                    app.show_help = false;
                }
                continue;
            }
            if app.rollup {
                match key.code {
                    KeyCode::Char('v') | KeyCode::Esc => app.rollup = false,
                    KeyCode::Char('j') | KeyCode::Down => app.move_rollup(1),
                    KeyCode::Char('k') | KeyCode::Up => app.move_rollup(-1),
                    KeyCode::Enter => app.jump_to_rollup_thread(),
                    _ => {}
                }
                continue;
            }
            if app.handle_global_key(key.code, key.modifiers) {
                return Ok(());
            }
        }
    }
}

fn render(frame: &mut ratatui::Frame, app: &mut App) {
    let [header, content, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let (sidebar, diff) = if app.sidebar_visible {
        let [sidebar, diff] =
            Layout::horizontal([Constraint::Length(30), Constraint::Min(1)]).areas(content);
        (Some(sidebar), diff)
    } else {
        (None, content)
    };
    app.viewport_rows = diff.height.saturating_sub(2).max(1);

    let needs_attention = app
        .threads
        .threads()
        .iter()
        .filter(|thread| thread.needs_attention)
        .count();
    let open = app
        .threads
        .threads()
        .iter()
        .filter(|thread| matches!(thread.resolution, thread::Resolution::Open))
        .count();
    let resolved = app.threads.threads().len().saturating_sub(open);
    frame.render_widget(
        Paragraph::new(format!(
            "revia  •  {} files  •  {needs_attention} need you  •  {open} open  •  {resolved} resolved",
            app.diff.document.files.len(),
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );

    let items = app
        .diff
        .document
        .files
        .iter()
        .map(|file| {
            let threads = app
                .threads
                .threads()
                .iter()
                .filter(|thread| thread.anchor.path == file.path)
                .collect::<Vec<_>>();
            let marker = if threads.iter().any(|thread| thread.needs_attention) {
                "!"
            } else if threads
                .iter()
                .any(|thread| matches!(thread.resolution, thread::Resolution::Open))
            {
                "•"
            } else if threads.is_empty() {
                " "
            } else {
                "✓"
            };
            ListItem::new(format!(
                "{marker} {}  {}h/{}t",
                file.path,
                file.hunks.len(),
                threads.len()
            ))
        })
        .collect::<Vec<_>>();
    let mut list_state = ListState::default();
    if !items.is_empty() {
        list_state.select(Some(app.view.selected_file));
    }
    if let Some(sidebar) = sidebar {
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::RIGHT).title(
                    if app.view.focus == FocusArea::Files {
                        "Files • focus"
                    } else {
                        "Files"
                    },
                ))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            sidebar,
            &mut list_state,
        );
    }

    let body = if app.rollup {
        rollup_text(app)
    } else {
        review_stream_text(app, diff.width)
    };
    let paragraph = Paragraph::new(body)
        .block(Block::default().borders(Borders::NONE).title(
            if app.view.focus == FocusArea::Review {
                "Review stream • focus"
            } else {
                "Review stream"
            },
        ))
        .scroll((app.view.scroll, 0));
    if app.wrap_lines && app.view.layout.resolved(diff.width) == LayoutMode::Stack {
        frame.render_widget(paragraph.wrap(Wrap { trim: false }), diff);
    } else {
        frame.render_widget(paragraph, diff);
    }

    let default_footer = "j/k scroll • f/b page • d/u half • g/G edge • [/] hunk • ,/. file • 1/2/0 layout • s rail • ? help";
    let footer_text = app.status.as_deref().unwrap_or(&default_footer);
    frame.render_widget(
        Paragraph::new(footer_text).style(Style::default().fg(Color::Gray)),
        footer,
    );
    if let Some(input) = &app.composer {
        let area = centered_rect(70, 7, frame.area());
        // Clear makes this a real modal surface even when the terminal itself
        // is transparent: no diff rows show through the composer.
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(input.as_str()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .style(Style::default().bg(Color::Rgb(20, 24, 34)).fg(Color::White))
                    .title(format!(
                        " {} — Enter post · Esc cancel ",
                        if app.reply_to.is_some() {
                            "Reply"
                        } else {
                            "New thread"
                        }
                    )),
            ),
            area,
        );
    }
    if app.show_help {
        let area = centered_rect(86, 18, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(HUNK_KEYBOARD_HELP)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Keyboard help — Esc to close"),
                ),
            area,
        );
    }
}

const HUNK_KEYBOARD_HELP: &str = "Hunk-compatible navigation\n\n  j/k  ↑/↓      scroll one row\n  f/Space, b    page down/up\n  d/u            half page\n  g/G Home/End   start/end\n  [/]            previous/next hunk\n  ,/.            previous/next file\n  1/2/0          split/stack/auto\n  s              toggle file rail\n  r              reload diff\n  m / w          hunk headers / wrapping\n\nrevia review extensions: c compose, t select thread, x resolve, R reopen, a attention, {/} attention jump.";

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let width = area
        .width
        .saturating_mul(width_percent)
        .saturating_div(100)
        .max(24);
    let width = width.min(area.width);
    let height = height.min(area.height.saturating_sub(2)).max(3);
    Rect {
        x: area.x.saturating_add(area.width.saturating_sub(width) / 2),
        y: area
            .y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    }
}

/// Build the one continuous reading surface: Git order first, anchored threads inline.
fn review_stream_text(app: &App, available_width: u16) -> Text<'static> {
    if app.diff.document.files.is_empty() {
        return Text::raw("No changed files.");
    }
    let mut lines = Vec::new();
    for (file_index, file) in app.diff.document.files.iter().enumerate() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("── {} ──", file.path),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        lines.extend(
            file.metadata
                .iter()
                .map(|line| Line::styled(line.clone(), Style::default().fg(Color::DarkGray))),
        );
        let syntax = file
            .extension()
            .and_then(|extension| app.syntax_set.find_syntax_by_extension(extension))
            .unwrap_or_else(|| app.syntax_set.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(syntax, &app.theme);
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let selected =
                file_index == app.view.selected_file && hunk_index == app.view.selected_hunk;
            let hunk_style = if selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            };
            if app.show_hunk_headers {
                lines.push(Line::styled(
                    format!("{} {}", if selected { "▶" } else { " " }, hunk.header),
                    hunk_style,
                ));
            }
            let layout = app.view.layout.resolved(available_width);
            if layout == LayoutMode::Split {
                lines.extend(split_hunk_lines(&hunk.lines, available_width, selected));
            } else {
                lines.extend(
                    hunk.lines
                        .iter()
                        .map(|line| highlight_line(line, &mut highlighter, &app.syntax_set)),
                );
            }
            for (thread_index, thread) in
                app.threads_for(&file.path, &hunk.header).iter().enumerate()
            {
                let active = selected
                    && thread_index == app.view.selected_thread
                    && app.view.focus == FocusArea::Threads;
                let state = if thread.needs_attention {
                    "NEEDS ATTENTION"
                } else if matches!(thread.resolution, thread::Resolution::Open) {
                    "OPEN"
                } else {
                    "RESOLVED"
                };
                let card_style = if active {
                    Style::default().bg(Color::DarkGray).fg(Color::Yellow)
                } else if thread.needs_attention {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let latest = thread
                    .messages
                    .last()
                    .map(|message| format!("{}: {}", message.author.id, message.body))
                    .unwrap_or_default();
                lines.push(Line::styled(
                    format!(
                        "  ┌ #{:03} {state}{}",
                        thread.id,
                        if thread.outdated { " · outdated" } else { "" }
                    ),
                    card_style,
                ));
                lines.push(Line::styled(format!("  │ {latest}"), card_style));
                lines.push(Line::styled(
                    "  └ c reply · x resolve · r reopen · a attention",
                    card_style,
                ));
            }
        }
    }
    Text::from(lines)
}

/// A visual row in the side-by-side diff.  Removed and added runs are paired in
/// order, which keeps a replacement aligned without pretending Git supplied a
/// line-level correspondence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SplitRow {
    old: Option<DiffLine>,
    new: Option<DiffLine>,
}

fn split_rows(lines: &[DiffLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        match lines[index].kind {
            DiffLineKind::Removed | DiffLineKind::Added => {
                let mut removed = Vec::new();
                while index < lines.len() && lines[index].kind == DiffLineKind::Removed {
                    removed.push(lines[index].clone());
                    index += 1;
                }
                let mut added = Vec::new();
                while index < lines.len() && lines[index].kind == DiffLineKind::Added {
                    added.push(lines[index].clone());
                    index += 1;
                }
                let count = removed.len().max(added.len());
                for row in 0..count {
                    rows.push(SplitRow {
                        old: removed.get(row).cloned(),
                        new: added.get(row).cloned(),
                    });
                }
            }
            _ => {
                let line = lines[index].clone();
                rows.push(SplitRow {
                    old: Some(line.clone()),
                    new: Some(line),
                });
                index += 1;
            }
        }
    }
    rows
}

fn split_hunk_lines(
    lines: &[DiffLine],
    available_width: u16,
    selected: bool,
) -> Vec<Line<'static>> {
    let column_width = usize::from(available_width.saturating_sub(5) / 2).max(12);
    split_rows(lines)
        .into_iter()
        .map(|row| {
            let old = split_cell(row.old.as_ref(), '-', column_width, selected);
            let new = split_cell(row.new.as_ref(), '+', column_width, selected);
            Line::from(vec![
                old,
                Span::styled(" │ ", selected_style(selected)),
                new,
            ])
        })
        .collect()
}

fn split_cell(
    line: Option<&DiffLine>,
    marker: char,
    width: usize,
    selected: bool,
) -> Span<'static> {
    let (prefix, text, color) = match line {
        Some(line) => {
            let prefix = match line.kind {
                DiffLineKind::Added => '+',
                DiffLineKind::Removed => '-',
                DiffLineKind::Context => ' ',
                DiffLineKind::Meta => '\\',
            };
            (prefix, line.text.as_str(), marker_color(line.kind))
        }
        None => (' ', "", Color::DarkGray),
    };
    let clipped = if text.chars().count() > width.saturating_sub(2) {
        format!(
            "{}…",
            text.chars()
                .take(width.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        text.to_owned()
    };
    Span::styled(
        format!(
            "{prefix}{marker} {:width$}",
            clipped,
            width = width.saturating_sub(2)
        ),
        selected_style(selected).fg(color),
    )
}

fn selected_style(selected: bool) -> Style {
    if selected {
        Style::default().bg(Color::Rgb(42, 52, 74))
    } else {
        Style::default()
    }
}

fn rollup_text(app: &App) -> Text<'static> {
    let threads = app.threads.threads();
    let need = threads
        .iter()
        .filter(|thread| thread.needs_attention)
        .count();
    let open = threads
        .iter()
        .filter(|thread| {
            !thread.needs_attention && matches!(thread.resolution, thread::Resolution::Open)
        })
        .count();
    let resolved = threads
        .iter()
        .filter(|thread| matches!(thread.resolution, thread::Resolution::Resolved))
        .count();
    let mut lines = vec![Line::styled(
        format!("{need} need you / {open} open / {resolved} resolved"),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    for (index, id) in app.rollup_ids().into_iter().enumerate() {
        let thread = threads
            .iter()
            .find(|thread| thread.id == id)
            .expect("id came from threads");
        let state = if thread.needs_attention {
            "NEEDS ATTENTION"
        } else if matches!(thread.resolution, thread::Resolution::Open) {
            "OPEN"
        } else {
            "RESOLVED"
        };
        let provenance = thread
            .closed_by
            .as_ref()
            .map_or(String::new(), |actor| format!(" — closed by {}", actor.id));
        let marker = if index == app.rollup_selected {
            "> "
        } else {
            "  "
        };
        lines.push(Line::styled(
            format!(
                "{marker}#{id} [{state}] {} {}{provenance}",
                thread.anchor.path, thread.anchor.hunk_header
            ),
            if index == app.rollup_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            },
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k select • Enter jump • v/Esc return",
        Style::default().fg(Color::Gray),
    ));
    Text::from(lines)
}

fn highlight_line(
    line: &DiffLine,
    highlighter: &mut HighlightLines<'_>,
    syntax_set: &SyntaxSet,
) -> Line<'static> {
    let marker = match line.kind {
        DiffLineKind::Added => "+",
        DiffLineKind::Removed => "-",
        DiffLineKind::Context => " ",
        DiffLineKind::Meta => "\\",
    };
    let background = match line.kind {
        DiffLineKind::Added => Some(Color::Rgb(24, 54, 35)),
        DiffLineKind::Removed => Some(Color::Rgb(65, 29, 34)),
        DiffLineKind::Context | DiffLineKind::Meta => None,
    };
    let base = background.map_or_else(Style::default, |background| Style::default().bg(background));
    let mut spans = vec![Span::styled(marker, base.fg(marker_color(line.kind)))];

    match highlighter.highlight_line(&line.text, syntax_set) {
        Ok(ranges) => {
            spans.extend(ranges.into_iter().map(|(style, text)| {
                Span::styled(text.to_owned(), merge_syntect_style(base, style))
            }))
        }
        Err(_) => spans.push(Span::styled(line.text.clone(), base)),
    }
    Line::from(spans)
}

fn marker_color(kind: DiffLineKind) -> Color {
    match kind {
        DiffLineKind::Added => Color::Green,
        DiffLineKind::Removed => Color::Red,
        DiffLineKind::Context => Color::DarkGray,
        DiffLineKind::Meta => Color::Yellow,
    }
}

fn merge_syntect_style(base: Style, source: SyntectStyle) -> Style {
    let foreground = source.foreground;
    base.fg(Color::Rgb(foreground.r, foreground.g, foreground.b))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{App, Args, render, review_stream_text, split_rows, target_from, wrapped_index};
    use crate::{
        anchor::Anchor,
        diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff},
    };

    #[test]
    fn selects_working_tree_by_default() {
        let args = Args {
            repo: ".".into(),
            staged: false,
            commit: None,
            range: None,
            context: 3,
            print: false,
        };
        assert_eq!(target_from(&args), DiffTarget::WorkingTree);
    }

    #[test]
    fn navigation_wraps_at_each_end() {
        assert_eq!(wrapped_index(0, 3, -1), 2);
        assert_eq!(wrapped_index(2, 3, 1), 0);
    }

    static REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn repository() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("revia-review-stream-{nonce}-{sequence}"));
        fs::create_dir_all(&path).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        path
    }

    #[test]
    fn review_stream_keeps_files_in_git_order_and_places_threads_inline() {
        let repository = repository();
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 3,
        };
        let mut app = App::new(
            repository,
            request,
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
        )
        .unwrap();
        let id = app
            .threads
            .post(
                Anchor {
                    revision: "deadbeef".into(),
                    path: "a.rs".into(),
                    hunk_header: "@@ -1 +1 @@".into(),
                },
                App::human(),
                "Please keep the invariant.",
            )
            .unwrap();
        app.threads.set_needs_attention(id, true).unwrap();

        let rendered = review_stream_text(&app, 120)
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("── a.rs ──\n"));
        assert!(rendered.contains("NEEDS ATTENTION"));
        assert!(rendered.contains("Please keep the invariant."));
        assert!(rendered.find("── a.rs ──").unwrap() < rendered.find("── b.rs ──").unwrap());

        app.move_review_hunk(1);
        assert_eq!(app.view.selected_file, 1);
        assert_eq!(app.view.selected_hunk, 0);
    }

    #[test]
    fn split_rows_align_replacements_without_losing_one_sided_changes() {
        let rows = split_rows(&[
            crate::diff::DiffLine {
                kind: crate::diff::DiffLineKind::Removed,
                text: "old one".into(),
            },
            crate::diff::DiffLine {
                kind: crate::diff::DiffLineKind::Removed,
                text: "old two".into(),
            },
            crate::diff::DiffLine {
                kind: crate::diff::DiffLineKind::Added,
                text: "new one".into(),
            },
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].old.as_ref().unwrap().text, "old one");
        assert_eq!(rows[0].new.as_ref().unwrap().text, "new one");
        assert_eq!(rows[1].old.as_ref().unwrap().text, "old two");
        assert!(rows[1].new.is_none());
    }

    #[test]
    fn composer_consumes_text_editing_and_escape_before_global_shortcuts() {
        let repository = repository();
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 3,
        };
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let mut app = App::new(
            repository,
            request,
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
        )
        .unwrap();
        app.begin_new_thread();
        assert!(app.handle_composer_key(KeyCode::Char('q')));
        assert_eq!(app.composer.as_deref(), Some("q"));
        assert!(app.handle_composer_key(KeyCode::Backspace));
        assert_eq!(app.composer.as_deref(), Some(""));
        assert!(app.handle_composer_key(KeyCode::Esc));
        assert!(app.composer.is_none());
    }

    #[test]
    fn hunk_global_keys_keep_the_same_meaning_across_focus_areas() {
        let repository = repository();
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 3,
        };
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let mut app = App::new(
            repository,
            request,
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
        )
        .unwrap();
        app.viewport_rows = 10;
        app.view.focus = crate::ui::FocusArea::Files;
        app.handle_global_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.view.scroll, 1);
        app.handle_global_key(KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(app.view.scroll, 11);
        app.handle_global_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert_eq!(app.view.scroll, 6);
        app.handle_global_key(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(app.view.scroll, u16::MAX);
        app.handle_global_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(app.view.scroll, 0);
        app.handle_global_key(KeyCode::Char(' '), KeyModifiers::SHIFT);
        assert_eq!(app.view.scroll, 0);

        app.handle_global_key(KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(app.view.layout, crate::ui::LayoutMode::Stack);
        app.handle_global_key(KeyCode::Char('1'), KeyModifiers::NONE);
        assert_eq!(app.view.layout, crate::ui::LayoutMode::Split);
        app.handle_global_key(KeyCode::Char('0'), KeyModifiers::NONE);
        assert_eq!(app.view.layout, crate::ui::LayoutMode::Auto);

        assert!(app.sidebar_visible);
        app.handle_global_key(KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(!app.sidebar_visible);
        app.handle_global_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(!app.show_hunk_headers);
        app.handle_global_key(KeyCode::Char('w'), KeyModifiers::NONE);
        assert!(app.wrap_lines);
        app.handle_global_key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(app.show_help);
    }

    #[test]
    fn rendered_fixture_exposes_split_stack_review_state_and_inline_threads() {
        let repository = repository();
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 3,
        };
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old_a\n+new_a\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old_b\n+new_b\n";
        let mut app = App::new(
            repository,
            request,
            LoadedDiff {
                text: raw.into(),
                document: DiffDocument::parse(raw),
            },
        )
        .unwrap();
        let attention = app
            .threads
            .post(
                Anchor {
                    revision: "deadbeef".into(),
                    path: "a.rs".into(),
                    hunk_header: "@@ -1 +1 @@".into(),
                },
                App::human(),
                "Needs a human decision.",
            )
            .unwrap();
        app.threads.set_needs_attention(attention, true).unwrap();
        let resolved = app
            .threads
            .post(
                Anchor {
                    revision: "deadbeef".into(),
                    path: "b.rs".into(),
                    hunk_header: "@@ -1 +1 @@".into(),
                },
                App::human(),
                "Already addressed.",
            )
            .unwrap();
        app.threads.close(resolved, &App::human()).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let split = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(split.contains("1 need you"));
        assert!(split.contains("NEEDS ATTENTION"));
        assert!(split.contains("RESOLVED"));
        assert!(split.contains(" │ "));

        app.view.layout = crate::ui::LayoutMode::Stack;
        let mut terminal = Terminal::new(TestBackend::new(80, 32)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let stack = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(stack.contains("-old_a"));
        assert!(stack.contains("+new_a"));
    }
}
