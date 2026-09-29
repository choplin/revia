mod anchor;
mod app;
mod cli;
mod diff;
mod input;
mod mode;
mod presentation;
mod renderer;
mod review;
mod runtime;
mod semantic;
mod styled_text;
mod symbols;
mod syntax;
mod thread;
mod tui_app;
mod ui;
mod urushi_renderer;

use std::io;

use anyhow::{Context, Result};
use app::Model;
use clap::Parser;
use cli::Args;
use diff::LoadedDiff;
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

#[cfg(unix)]
fn run_tui(mut model: Model, runtime: Runtime) -> Result<()> {
    model.global.keyboard_protocol = input::KeyboardProtocol::Legacy;
    let terminal = urushi_terminal::backend::native::NativeTerminal::open()
        .context("could not open the controlling terminal")?;
    urushi_tui_app::Runtime::new(tui_app::ReviaApplication::new(model, runtime))
        .backend(terminal)
        .keyboard_enhancement(None)
        .run()
        .context("could not run the terminal application")?;
    Ok(())
}

#[cfg(not(unix))]
fn run_tui(_model: Model, _runtime: Runtime) -> Result<()> {
    anyhow::bail!("the native Urushi terminal backend requires Unix")
}
