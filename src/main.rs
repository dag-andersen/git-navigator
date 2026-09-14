mod app;
mod cli;
mod git;
mod model;
mod ui;

use std::{io::IsTerminal, time::Duration};

use anyhow::{Context, Result, bail};
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::{app::App, cli::Cli};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let directory = cli
        .directory
        .canonicalize()
        .with_context(|| format!("cannot open {}", cli.directory.display()))?;

    if !directory.is_dir() {
        bail!("{} is not a directory", directory.display());
    }
    if !std::io::stdout().is_terminal() {
        bail!("git-navigator requires an interactive terminal");
    }

    let mut app = App::load(directory, cli.base)?;
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| ui::render(frame, &mut app))?;
            if event::poll(Duration::from_millis(250))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && app.handle_key(key)
            {
                return Ok::<(), std::io::Error>(());
            }
        }
    })
    .context("terminal error")
}
