use ratatui::{
    Frame,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};

use crate::{
    app::{App, Focus},
    model::ChangeMode,
};

const SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(ratatui::style::Modifier::BOLD);

pub(crate) fn render(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let items = items(app);
    let title = format!("History ({})", app.commits.len() + 1);
    let list = List::new(items)
        .block(super::pane_block(&title, app.focus == Focus::Worktrees))
        .highlight_style(SELECTED)
        .highlight_symbol("› ");
    let mut state = app.history_state;
    state.select(visual_index(app, app.selected_commit));
    frame.render_stateful_widget(list, area, &mut state);
    *app.history_state.offset_mut() = state.offset();
}

fn items(app: &App) -> Vec<ListItem<'static>> {
    let mut items = Vec::with_capacity(app.commits.len() + 2);
    items.push(ListItem::new(Line::from(vec![
        node(wip_is_in_branch_diff(app)),
        Span::styled("WIP ", Style::new().fg(Color::Yellow).bold()),
        Span::raw("Uncommitted changes"),
    ])));

    for (index, commit) in app.commits.iter().enumerate() {
        let active = commit_is_in_branch_diff(app, index);
        let graph_lines = commit
            .graph
            .iter()
            .take(commit.graph.len().saturating_sub(1))
            .map(|graph| graph_line(graph, active, None))
            .collect::<Vec<_>>();
        items.extend(graph_lines.into_iter().map(ListItem::new));
        let graph = graph_line(
            commit.graph.last().map(String::as_str).unwrap_or("●"),
            active,
            base_node(app, &commit.hash),
        );
        let mut commit_line = graph;
        commit_line.spans.extend([
            Span::styled(
                format!("{} ", commit.short_hash),
                Style::new().fg(Color::Cyan),
            ),
            Span::raw(commit.subject.clone()),
        ]);
        items.push(ListItem::new(commit_line));
    }
    items
}

pub(crate) fn visual_index(app: &App, selected_commit: Option<usize>) -> Option<usize> {
    let target = selected_commit?;
    let mut visual_index = 1;
    if target == 0 {
        return Some(0);
    }

    for (index, commit) in app.commits.iter().enumerate() {
        if target == index + 1 {
            return Some(visual_index + commit.graph.len().saturating_sub(1));
        }
        visual_index += commit.graph.len();
    }
    None
}

fn node(active: bool) -> Span<'static> {
    if active {
        Span::styled("● ", Style::new().fg(Color::LightGreen).bold())
    } else {
        Span::styled("● ", Style::new().fg(Color::Cyan))
    }
}

pub(crate) fn graph_line(graph: &str, active: bool, marker: Option<char>) -> Line<'static> {
    let mut spans = Vec::new();
    let graph_style = Style::new().fg(Color::DarkGray);
    for character in graph.chars() {
        let style = if character == '●' {
            Style::new()
                .fg(if active {
                    Color::LightGreen
                } else {
                    Color::Cyan
                })
                .bold()
        } else {
            graph_style
        };
        let symbol = marker
            .filter(|_| character == '●')
            .map_or_else(|| character.to_string(), |marker| marker.to_string());
        spans.push(Span::styled(symbol.to_string(), style));
    }
    spans.push(Span::raw(
        " ".repeat(4usize.saturating_sub(graph.chars().count())),
    ));
    Line::from(spans)
}

fn base_node(app: &App, hash: &str) -> Option<char> {
    if app.local_base_hash.as_deref() == Some(hash) {
        Some('⬥')
    } else if app.remote_base_hash.as_deref() == Some(hash) {
        Some('⬦')
    } else {
        None
    }
}

pub(crate) fn wip_is_in_branch_diff(app: &App) -> bool {
    app.mode == ChangeMode::Branch
        && app.selected_commit == Some(0)
        && !app.history_range_commits.is_empty()
}

pub(crate) fn commit_is_in_branch_diff(app: &App, commit_index: usize) -> bool {
    if app.mode != ChangeMode::Branch {
        return false;
    }
    app.commits
        .get(commit_index)
        .is_some_and(|commit| app.history_range_commits.contains(&commit.hash))
}
