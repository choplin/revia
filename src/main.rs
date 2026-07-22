mod diff;

use std::{io, path::PathBuf};

use anyhow::Result;
use clap::{ArgGroup, Parser};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use diff::{DiffRequest, DiffTarget, LoadedDiff};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    text::Text,
    widgets::{Block, Borders, Paragraph},
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

    run_tui(diff)
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

fn run_tui(diff: LoadedDiff) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &diff);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, diff: &LoadedDiff) -> Result<()> {
    loop {
        terminal.draw(|frame| {
            let [header, body] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(frame.area());
            frame.render_widget(Paragraph::new("revia  •  q: quit  •  read-only"), header);
            frame.render_widget(
                Paragraph::new(Text::raw(diff.text.clone()))
                    .block(Block::default().borders(Borders::TOP).title("Git diff")),
                body,
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press
                && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
            {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, target_from};
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
}
