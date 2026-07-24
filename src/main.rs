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

use anyhow::Result;
use app::{ActiveMode, Effect, Model};
use clap::Parser;
use cli::Args;
use crossterm::{
    event::{self, Event as CrosstermEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use diff::LoadedDiff;
use input::{Key, KeyPhase};
use ratatui::{Terminal, backend::CrosstermBackend};
use renderer::Renderer;
use runtime::Runtime;

fn main() -> Result<()> {
    let args = Args::parse();
    let request = args.request();
    let diff = LoadedDiff::load(&args.repo, &request)?;

    if args.print || !io::IsTerminal::is_terminal(&io::stdout()) {
        print!("{}", diff.text);
        return Ok(());
    }

    let (runtime, threads) = Runtime::open(&args.repo)?;
    let model = Model::new(request, diff, threads);
    run_tui(model, runtime)
}

fn run_tui(mut model: Model, runtime: Runtime) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut model, &runtime);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    model: &mut Model,
    runtime: &Runtime,
) -> Result<()> {
    let renderer = Renderer::default();
    let mut last_scroll_at: Option<Instant> = None;
    let effects = app::update(
        model,
        app::global::Event::ViewportResized(terminal.size()?.height.saturating_sub(4)),
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
            CrosstermEvent::Resize(_, height) => {
                let effects = app::update(
                    model,
                    app::global::Event::ViewportResized(height.saturating_sub(4)),
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
