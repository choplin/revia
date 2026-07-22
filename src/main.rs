mod anchor;
mod diff;

use std::{io, path::PathBuf};

use anyhow::Result;
use clap::{ArgGroup, Parser};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use diff::{DiffFile, DiffLine, DiffLineKind, DiffRequest, DiffTarget, LoadedDiff};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
};

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

    run_tui(App::new(args.repo, request, diff))
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
    selected_file: usize,
    selected_hunk: usize,
    scroll: u16,
    status: Option<String>,
    syntax_set: SyntaxSet,
    theme: Theme,
}

impl App {
    fn new(repository: PathBuf, request: DiffRequest, diff: LoadedDiff) -> Self {
        let themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .get("base16-ocean.dark")
            .or_else(|| themes.themes.values().next())
            .expect("syntect includes a default theme")
            .clone();
        Self {
            repository,
            request,
            diff,
            selected_file: 0,
            selected_hunk: 0,
            scroll: 0,
            status: None,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme,
        }
    }

    fn file(&self) -> Option<&DiffFile> {
        self.diff.document.files.get(self.selected_file)
    }

    fn move_file(&mut self, direction: i32) {
        let count = self.diff.document.files.len();
        if count == 0 {
            return;
        }
        self.selected_file = wrapped_index(self.selected_file, count, direction);
        self.selected_hunk = 0;
        self.scroll = 0;
    }

    fn move_hunk(&mut self, direction: i32) {
        let hunk_count = self.file().map_or(0, |file| file.hunks.len());
        if hunk_count == 0 {
            return;
        }
        self.selected_hunk = wrapped_index(self.selected_hunk, hunk_count, direction);
        self.scroll = self
            .file()
            .map_or(0, |file| file.hunk_start_line(self.selected_hunk));
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
                self.selected_file = self
                    .selected_file
                    .min(self.diff.document.files.len().saturating_sub(1));
                self.selected_hunk = 0;
                self.scroll = 0;
                self.status = Some(format!("context: {} lines", self.request.context_lines));
            }
            Err(error) => self.status = Some(format!("could not reload diff: {error}")),
        }
    }

    fn scroll(&mut self, delta: i16) {
        self.scroll = self.scroll.saturating_add_signed(delta);
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
    loop {
        terminal.draw(|frame| render(frame, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('j') | KeyCode::Down => app.scroll(1),
                KeyCode::Char('k') | KeyCode::Up => app.scroll(-1),
                KeyCode::Char('n') => app.move_hunk(1),
                KeyCode::Char('N') => app.move_hunk(-1),
                KeyCode::Char(']') | KeyCode::Tab => app.move_file(1),
                KeyCode::Char('[') | KeyCode::BackTab => app.move_file(-1),
                KeyCode::Char('}') => app.adjust_context(1),
                KeyCode::Char('{') => app.adjust_context(-1),
                _ => {}
            }
        }
    }
}

fn render(frame: &mut ratatui::Frame, app: &App) {
    let [header, content, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [sidebar, diff] =
        Layout::horizontal([Constraint::Length(30), Constraint::Min(1)]).areas(content);

    frame.render_widget(
        Paragraph::new(format!(
            "revia  •  {} files  •  context: {}",
            app.diff.document.files.len(),
            app.request.context_lines
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );

    let items = app
        .diff
        .document
        .files
        .iter()
        .map(|file| ListItem::new(format!("{}  ({})", file.path, file.hunks.len())))
        .collect::<Vec<_>>();
    let mut list_state = ListState::default();
    if !items.is_empty() {
        list_state.select(Some(app.selected_file));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::RIGHT).title("Files"))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        sidebar,
        &mut list_state,
    );

    let body = app
        .file()
        .map(|file| file_text(file, &app.syntax_set, &app.theme))
        .unwrap_or_else(|| Text::raw("No changed files."));
    frame.render_widget(
        Paragraph::new(body)
            .block(Block::default().borders(Borders::NONE).title("Diff"))
            .scroll((app.scroll, 0)),
        diff,
    );

    let footer_text = app
        .status
        .as_deref()
        .unwrap_or("j/k scroll • n/N hunk • [/]/Tab file • {/} context • q quit (read-only)");
    frame.render_widget(
        Paragraph::new(footer_text).style(Style::default().fg(Color::Gray)),
        footer,
    );
}

fn file_text(file: &DiffFile, syntax_set: &SyntaxSet, theme: &Theme) -> Text<'static> {
    let syntax = file
        .extension()
        .and_then(|extension| syntax_set.find_syntax_by_extension(extension))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = file
        .metadata
        .iter()
        .map(|line| Line::styled(line.clone(), Style::default().fg(Color::DarkGray)))
        .collect::<Vec<_>>();

    for hunk in &file.hunks {
        lines.push(Line::styled(
            hunk.header.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        for line in &hunk.lines {
            lines.push(highlight_line(line, &mut highlighter, syntax_set));
        }
    }
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
    use super::{Args, target_from, wrapped_index};
    use crate::diff::DiffTarget;

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
}
