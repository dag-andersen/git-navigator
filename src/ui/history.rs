use ratatui::{
    Frame,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};

use crate::{
    app::{
        App, Focus,
        history::{display_rows, visual_index},
    },
    model::{ChangeMode, HistorySelection},
};

const SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(ratatui::style::Modifier::BOLD);

pub(crate) fn render(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let rows = display_rows(app);
    let selected = visual_index(app, app.history_selection.as_ref());
    let items = items(app, rows);
    let branch = app
        .worktrees
        .iter()
        .find(|worktree| worktree.is_current)
        .map(|worktree| worktree.branch.as_str())
        .unwrap_or("detached HEAD");
    let title = format!("History ({}) - {branch}", app.commits.len() + 1);
    let list = List::new(items)
        .block(super::pane_block(&title, app.focus == Focus::Worktrees))
        .highlight_style(SELECTED)
        .highlight_symbol("› ");
    let mut state = app.history_state;
    state.select(selected);
    frame.render_stateful_widget(list, area, &mut state);
    *app.history_state.offset_mut() = state.offset();
}

fn items(app: &App, rows: Vec<crate::model::HistoryRow>) -> Vec<ListItem<'static>> {
    rows.into_iter()
        .map(|row| match row {
            crate::model::HistoryRow::Wip { graph } => {
                let mut line =
                    graph_line_with_marker(&graph, wip_is_in_branch_diff(app), Some('○'), None);
                line.spans.extend([
                    Span::styled("WIP ", Style::new().fg(Color::Yellow).bold()),
                    Span::raw("Uncommitted changes"),
                ]);
                ListItem::new(line)
            }
            crate::model::HistoryRow::Graph(graph) => {
                ListItem::new(graph_line(&graph, false, None))
            }
            crate::model::HistoryRow::BranchLabel {
                graph,
                names,
                connected,
            } => {
                let marker = if connected { '│' } else { ' ' };
                let mut label =
                    graph_line_with_marker(&graph, false, Some(marker), Some(Color::DarkGray));
                label.spans.push(Span::styled(
                    names.join(", "),
                    Style::new().fg(Color::DarkGray),
                ));
                ListItem::new(label)
            }
            crate::model::HistoryRow::Commit { hash } => {
                let Some(commit) = app.commits.iter().find(|commit| commit.hash == hash) else {
                    return ListItem::new(Line::raw(""));
                };
                let graph = graph_line(
                    commit.graph.last().map(String::as_str).unwrap_or("●"),
                    commit_is_in_branch_diff(app, &commit.hash),
                    base_node(app, &commit.hash),
                );
                let mut line = graph;
                line.spans.extend([
                    Span::styled(
                        format!("{} ", commit.short_hash),
                        Style::new().fg(Color::Cyan),
                    ),
                    Span::raw(commit.subject.clone()),
                ]);
                ListItem::new(line)
            }
        })
        .collect()
}

pub(crate) fn graph_line(graph: &str, active: bool, marker: Option<char>) -> Line<'static> {
    graph_line_with_marker(graph, active, marker, Some(Color::Red))
}

fn graph_line_with_marker(
    graph: &str,
    active: bool,
    marker: Option<char>,
    marker_color: Option<Color>,
) -> Line<'static> {
    let mut spans = Vec::new();
    let graph_style = Style::new().fg(Color::DarkGray);
    for character in graph.chars() {
        let style = if character == '●' {
            let color = if let Some(marker_color) = marker_color.filter(|_| marker.is_some()) {
                marker_color
            } else if active {
                Color::Blue
            } else {
                Color::Cyan
            };
            Style::new().fg(color).bold()
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
        Some('■')
    } else if app.remote_base_hash.as_deref() == Some(hash) {
        Some('□')
    } else {
        None
    }
}

pub(crate) fn wip_is_in_branch_diff(app: &App) -> bool {
    app.mode == ChangeMode::Branch
        && app.history_selection == Some(HistorySelection::Wip)
        && !app.history_range_commits.is_empty()
}

pub(crate) fn commit_is_in_branch_diff(app: &App, hash: &str) -> bool {
    if app.mode != ChangeMode::Branch {
        return false;
    }
    app.history_range_commits.contains(hash)
}
