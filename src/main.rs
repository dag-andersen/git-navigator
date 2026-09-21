mod app;
mod cli;
mod control;
mod diff_geometry;
mod editor;
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

use crate::{
    app::{App, Focus, HistoryPanel},
    cli::{Cli, Command, RenderMode, StartupFocus},
    watcher::AutoRefresh,
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Command::Ctl { control }) = cli.command {
        return control::run_ctl(control);
    }
    let directory = cli
        .directory
        .clone()
        .context("a repository directory is required")?;
    let directory_display = directory.display().to_string();
    let directory = directory
        .canonicalize()
        .with_context(|| format!("cannot open {directory_display}"))?;

    if !directory.is_dir() {
        bail!("{} is not a directory", directory.display());
    }

    let mut app = App::load(directory, cli.base.clone())?;
    app.changes.mode = match cli.mode {
        RenderMode::Uncommitted => crate::model::ChangeMode::Uncommitted,
        RenderMode::Branch => crate::model::ChangeMode::Branch,
    };
    if cli.focus.is_none() {
        app.history.history_panel = if app.has_linked_worktrees() {
            HistoryPanel::Worktrees
        } else {
            HistoryPanel::History
        };
        app.view.focus = Focus::Worktrees;
    }
    if cli.render {
        if cli.width == 0 || cli.height == 0 {
            bail!("render width and height must be greater than zero");
        }
        app.prepare_render(
            matches!(cli.focus, Some(StartupFocus::History)),
            cli.commit.as_deref(),
        )?;
        apply_startup_panel(&mut app, &cli);
        println!(
            "{}",
            ui::render_snapshot(&mut app, cli.width, cli.height, cli.ansi)
        );
        return Ok(());
    }
    if !std::io::stdout().is_terminal() {
        bail!("git-navigator requires an interactive terminal");
    }
    if cli.focus == Some(StartupFocus::History) {
        app.prepare_render(true, None)?;
    }
    apply_startup_panel(&mut app, &cli);

    let mut auto_refresh = AutoRefresh::new(
        &app.repository.directory,
        app.selected_worktree()
            .map(|worktree| worktree.path.as_path()),
    );
    let session = control::Session::start(&app.repository.directory)?;
    let mut terminal = ratatui::init();
    let result = execute!(stdout(), EnableMouseCapture).and_then(|()| {
        run_app(
            &mut terminal,
            &mut app,
            &mut auto_refresh,
            session.receiver(),
        )
    });
    let cleanup_result = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();

    result.and(cleanup_result).context("terminal error")
}

fn apply_startup_panel(app: &mut App, cli: &Cli) {
    let Some(focus) = cli.focus else {
        return;
    };
    let panel = match focus {
        StartupFocus::History => {
            app.history.history_panel = HistoryPanel::History;
            Focus::Worktrees
        }
        StartupFocus::Worktrees => {
            app.history.history_panel = HistoryPanel::Worktrees;
            Focus::Worktrees
        }
        StartupFocus::Files => Focus::Files,
        StartupFocus::Diff => Focus::Diff,
    };
    app.view.focus = panel;
    app.view.expanded = true;
    app.view.initial_layout_applied = true;
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    auto_refresh: &mut AutoRefresh,
    control_rx: &std::sync::mpsc::Receiver<control::ControlMessage>,
) -> std::io::Result<()> {
    loop {
        while let Ok(message) = control_rx.try_recv() {
            let response = app.apply_control(message.request);
            let _ = message
                .response
                .send(control::Response::from_result(response));
        }
        auto_refresh.watch_worktree(
            app.selected_worktree()
                .map(|worktree| worktree.path.as_path()),
            Instant::now(),
        );
        let now = Instant::now();
        if !app.modal_open()
            && let Some(paths) = auto_refresh.refresh_paths(now)
        {
            if app.view.follow_changes {
                app.refresh_following(&paths);
            } else {
                app.refresh();
            }
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
