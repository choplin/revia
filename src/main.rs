mod anchor;
mod app;
mod cli;
mod crossterm_adapter;
mod diff;
mod input;
mod mode;
mod presentation;
mod renderer;
mod review;
mod runtime;
mod semantic;
mod thread;
mod ui;

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use app::{ActiveMode, Effect, Model};
use clap::Parser;
use cli::Args;
use crossterm::event::{self, Event as CrosstermEvent};
use diff::LoadedDiff;
use input::{Key, KeyPhase};
use ratatui::{Terminal, backend::CrosstermBackend};
use renderer::Renderer;
use runtime::Runtime;

fn main() -> Result<()> {
    let args = Args::parse();
    let request = args.request();
    let diff = LoadedDiff::load(&args.repo, &request).with_context(|| {
        format!(
            "could not load the selected {} diff from {}",
            request.target.description(),
            args.repo.display()
        )
    })?;

    if args.print || !io::IsTerminal::is_terminal(&io::stdout()) {
        print!("{}", diff.text);
        return Ok(());
    }

    let (runtime, threads) = Runtime::open(&args.repo).with_context(|| {
        format!(
            "could not open the review thread store for {}",
            args.repo.display()
        )
    })?;
    let model = Model::new(request, diff, threads);
    run_tui(model, runtime)
}

fn run_tui(mut model: Model, runtime: Runtime) -> Result<()> {
    let mut session = crossterm_adapter::TerminalSession::start(io::stdout())?;
    let backend = CrosstermBackend::new(session.writer_mut());
    let mut terminal = Terminal::new(backend).context("could not initialize terminal renderer")?;

    let result = run_app(&mut terminal, &mut model, &runtime);
    drop(terminal);
    session.finish(result)
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<&mut io::Stdout>>,
    model: &mut Model,
    runtime: &Runtime,
) -> Result<()> {
    let renderer = Renderer::default();
    let mut last_scroll_at: Option<Instant> = None;
    let size = terminal.size()?;
    let effects = app::update(
        model,
        app::global::Event::ViewportResized {
            rows: size.height.saturating_sub(4),
            columns: size.width,
        },
    );
    dispatch_effects(model, runtime, effects);
    while model.is_running() {
        let semantic_view = app::view(model);
        terminal.draw(|frame| renderer.render(frame, &semantic_view))?;

        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        match event::read()? {
            CrosstermEvent::Key(key) => {
                if crossterm_adapter::is_interrupt(key) {
                    bail!("interrupted by Ctrl-C");
                }
                let input = crossterm_adapter::physical_input(key);
                if input.phase == KeyPhase::Release {
                    continue;
                }
                let is_scroll = matches!(
                    input.key,
                    Key::Char('j') | Key::Down | Key::Char('k') | Key::Up
                ) && model.active_mode == ActiveMode::Review;
                if is_scroll
                    && input.phase == KeyPhase::Repeat
                    && last_scroll_at.is_some_and(|last| last.elapsed() < Duration::from_millis(28))
                {
                    continue;
                }
                if is_scroll {
                    last_scroll_at = Some(Instant::now());
                }
                let (_, effects) = app::handle_input(model, input);
                dispatch_effects(model, runtime, effects);
            }
            CrosstermEvent::Resize(width, height) => {
                let effects = app::update(
                    model,
                    app::global::Event::ViewportResized {
                        rows: height.saturating_sub(4),
                        columns: width,
                    },
                );
                dispatch_effects(model, runtime, effects);
            }
            _ => {}
        }
    }
    Ok(())
}

fn dispatch_effects(model: &mut Model, runtime: &Runtime, initial: Vec<Effect>) {
    let mut effects = VecDeque::from(initial);
    while let Some(effect) = effects.pop_front() {
        let result = runtime.perform(effect, &model.global.threads);
        effects.extend(app::update(model, result));
    }
}
