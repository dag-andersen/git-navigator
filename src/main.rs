mod app;
mod cli;
mod git;
mod model;
mod ui;
mod watcher;

use std::{
    io::{IsTerminal, stdout},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
};

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
    let mut terminal = ratatui::init();
    let result = execute!(stdout(), EnableMouseCapture)
        .and_then(|()| run_app(&mut terminal, &mut app, &mut auto_refresh));
    let cleanup_result = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();

    result.and(cleanup_result).context("terminal error")
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    auto_refresh: &mut AutoRefresh,
) -> std::io::Result<()> {
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
        let size = terminal.size()?;
        let areas = ui::interaction_areas(
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
            app,
        );
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press && app.handle_key(key) => {
                    return Ok(());
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse, areas),
                _ => {}
            }
            auto_refresh.watch_worktree(
                app.selected_worktree()
                    .map(|worktree| worktree.path.as_path()),
                Instant::now(),
            );
        }
    }
}
