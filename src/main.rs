mod adapter;
mod app;
mod domain;
mod presentation;
#[cfg(test)]
mod test_support;

use std::io;

use adapter::{cli::Args, diff as source_diff, runtime::Runtime, terminal::ReviaApplication};
use anyhow::{Context, Result};
use app::{Model, input::KeyboardProtocol};
use clap::Parser;

fn main() -> Result<()> {
    let args = Args::parse();
    let request = args.request();
    let captured = source_diff::capture(&args.repo, &request).with_context(|| {
        format!(
            "could not load the selected {}",
            request.source.description()
        )
    })?;

    if args.print || !io::IsTerminal::is_terminal(&io::stdout()) {
        print!("{}", captured.patch().text());
        return Ok(());
    }

    let (runtime, threads) = Runtime::open(&args.repo, &request, &captured).with_context(|| {
        format!(
            "could not open the review thread store for {}",
            args.repo.display()
        )
    })?;
    let model = Model::new(request, captured, threads)?;
    run_tui(model, runtime)
}

#[cfg(unix)]
fn run_tui(mut model: Model, runtime: Runtime) -> Result<()> {
    model.global.keyboard_protocol = KeyboardProtocol::Legacy;
    let terminal = urushi_terminal::backend::native::NativeTerminal::open()
        .context("could not open the controlling terminal")?;
    urushi_tui_app::Runtime::new(ReviaApplication::new(model, runtime))
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
