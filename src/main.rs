mod anchor;
mod app;
mod cli;
mod crossterm_adapter;
mod diff;
mod input;
mod mode;
mod presentation;
mod render_loop;
mod renderer;
mod review;
mod runtime;
mod semantic;
mod symbols;
mod syntax;
mod thread;
mod ui;

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use app::{Effect, Model};
use clap::Parser;
use cli::Args;
use crossterm::event::{self, Event as CrosstermEvent};
use diff::LoadedDiff;
use input::KeyPhase;
use render_loop::PhysicalRenderer;
use runtime::Runtime;

const LOGICAL_FRAME_INTERVAL: Duration = Duration::from_millis(33);
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(8);

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
    model.global.keyboard_protocol = session.keyboard_protocol();
    let renderer = PhysicalRenderer::start()?;
    let app_result = run_app(&renderer, &mut model, &runtime);
    let render_result = renderer.finish();
    let result = app_result.and(render_result);
    session.finish(result)
}

fn run_app(renderer: &PhysicalRenderer, model: &mut Model, runtime: &Runtime) -> Result<()> {
    let (width, height) = crossterm::terminal::size()?;
    let effects = app::update(
        model,
        app::global::Event::ViewportResized {
            rows: height.saturating_sub(3),
            columns: width,
        },
    );
    dispatch_effects(model, runtime, effects);
    if !renderer.try_publish(app::view(model)) {
        bail!("terminal renderer rejected the initial frame");
    }
    let mut last_logical_frame_at = Instant::now();
    let mut dirty = false;

    while model.is_running() && renderer.is_running() {
        if dirty
            && renderer.is_ready()
            && last_logical_frame_at.elapsed() >= LOGICAL_FRAME_INTERVAL
            && renderer.try_publish(app::view(model))
        {
            dirty = false;
            last_logical_frame_at = Instant::now();
        }

        let poll_interval = if dirty && renderer.is_ready() {
            LOGICAL_FRAME_INTERVAL
                .saturating_sub(last_logical_frame_at.elapsed())
                .min(INPUT_POLL_INTERVAL)
        } else {
            INPUT_POLL_INTERVAL
        };
        if !event::poll(poll_interval)? {
            continue;
        }
        let mut drained = 0;
        loop {
            dirty |= handle_terminal_event(model, runtime, event::read()?)?;
            drained += 1;
            if drained >= 64 || !model.is_running() || !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
    Ok(())
}

fn handle_terminal_event(
    model: &mut Model,
    runtime: &Runtime,
    event: CrosstermEvent,
) -> Result<bool> {
    match event {
        CrosstermEvent::Key(key) => {
            if crossterm_adapter::is_interrupt(key) {
                bail!("interrupted by Ctrl-C");
            }
            let input = crossterm_adapter::physical_input(key);
            if input.phase == KeyPhase::Release {
                return Ok(false);
            }
            let (_, effects) = app::handle_input(model, input);
            dispatch_effects(model, runtime, effects);
            Ok(true)
        }
        CrosstermEvent::Resize(width, height) => {
            let effects = app::update(
                model,
                app::global::Event::ViewportResized {
                    rows: height.saturating_sub(3),
                    columns: width,
                },
            );
            dispatch_effects(model, runtime, effects);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn dispatch_effects(model: &mut Model, runtime: &Runtime, initial: Vec<Effect>) {
    let mut effects = VecDeque::from(initial);
    while let Some(effect) = effects.pop_front() {
        let result = runtime.perform(effect, &model.global.threads);
        effects.extend(app::update(model, result));
    }
}
