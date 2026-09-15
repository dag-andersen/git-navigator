mod app;
mod cli;
mod git;
mod model;
mod ui;
mod watcher;

use std::{
    io::IsTerminal,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::{app::App, cli::Cli, watcher::AutoRefresh};

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
    let mut auto_refresh = AutoRefresh::new(
        &app.directory,
        app.selected_worktree()
            .map(|worktree| worktree.path.as_path()),
    );
    ratatui::run(|terminal| {
        loop {
            let now = Instant::now();
            if auto_refresh.should_refresh(now) && !app.modal_open() {
                app.refresh();
                auto_refresh.mark_refreshed(now);
                auto_refresh.watch_worktree(
                    app.selected_worktree()
                        .map(|worktree| worktree.path.as_path()),
                    now,
                );
            }
            terminal.draw(|frame| ui::render(frame, &mut app))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                if app.handle_key(key) {
                    return Ok::<(), std::io::Error>(());
                }
                auto_refresh.watch_worktree(
                    app.selected_worktree()
                        .map(|worktree| worktree.path.as_path()),
                    Instant::now(),
                );
            }
        }
    })
    .context("terminal error")
}
