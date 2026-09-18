mod diff;
mod files;
mod history;
mod layout;
mod projection;

use std::path::Path;

use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::{
    app::{App, Focus, PanelLayout, StatusKind},
    diff_geometry::effective_layout,
    model::ChangeMode,
};

#[cfg(test)]
use crate::diff_geometry::{content_width as diff_content_width, display_text};
#[cfg(test)]
use diff::{highlighted_spans, unified_modified_lines_with_search};
#[cfg(test)]
use files::file_style;
#[cfg(test)]
use history::wip_is_in_branch_diff as history_wip_is_in_branch_diff;
#[cfg(test)]
use history::{commit_is_in_branch_diff as history_commit_is_in_branch_diff, graph_line};

const ACTIVE_BORDER: Color = Color::Cyan;
const INACTIVE_BORDER: Color = Color::DarkGray;
const COMPACT_LAYOUT_THRESHOLD: u16 = 120;
const WORKTREE_ITEM_HEIGHT: usize = 2;
pub(crate) const SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

pub fn render(frame: &mut Frame, app: &mut App) {
    app.apply_initial_layout(frame.area().width, COMPACT_LAYOUT_THRESHOLD);
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(frame, app, header);
    let panels = projection::panels(app);
    let show_worktrees = panels.show_worktrees;
    let [worktrees, files, diff] = panel_areas_for(
        body,
        app.view.focus,
        app.view.expanded,
        app.view.panel_layout,
        show_worktrees,
    );
    if show_worktrees && app.view.expanded && app.view.focus != Focus::Worktrees {
        render_compact_panel(
            frame,
            worktrees,
            "W",
            panels.worktree_rows.len(),
            panels.selected_worktree,
        );
    } else if show_worktrees {
        if app.history_active() {
            render_history(frame, app, worktrees);
        } else {
            render_worktrees(frame, app, worktrees);
        }
    }
    if app.view.expanded && app.view.focus != Focus::Files {
        render_compact_panel(
            frame,
            files,
            "F",
            panels.file_rows.len(),
            panels.selected_file,
        );
    } else {
        files::render(frame, app, files, &panels.file_rows, panels.selected_file);
    }
    if app.view.expanded && app.view.focus != Focus::Diff {
        render_compact_panel(
            frame,
            diff,
            "D",
            app.selected_file().map_or(0, |file| file.hunks.len()),
            panels.selected_hunk,
        );
    } else {
        render_diff(frame, app, diff);
    }
    render_footer(frame, app, footer);

    if app.view.show_help {
        render_help(frame);
    }
    if let Some(confirmation) = &app.view.delete_confirmation {
        render_delete_confirmation(frame, confirmation);
    }
}

pub fn render_snapshot(app: &mut App, width: u16, height: u16, ansi: bool) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("snapshot terminal should be created");
    terminal
        .draw(|frame| render(frame, app))
        .expect("snapshot should render");
    let buffer = terminal.backend().buffer();

    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| snapshot_cell(&buffer[(x, y)], ansi))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot_cell(cell: &ratatui::buffer::Cell, ansi: bool) -> String {
    if !ansi {
        return cell.symbol().to_string();
    }

    let mut codes = Vec::new();
    if cell.modifier.contains(Modifier::BOLD) {
        codes.push("1".to_string());
    }
    if cell.modifier.contains(Modifier::ITALIC) {
        codes.push("3".to_string());
    }
    codes.push(color_code(cell.fg, false));
    if cell.bg != Color::Reset {
        codes.push(color_code(cell.bg, true));
    }
    format!("\x1b[{}m{}\x1b[0m", codes.join(";"), cell.symbol())
}

fn color_code(color: Color, background: bool) -> String {
    let offset = if background { 10 } else { 0 };
    match color {
        Color::Reset => "39".into(),
        Color::Black => format!("{}", 30 + offset),
        Color::Red => format!("{}", 31 + offset),
        Color::Green => format!("{}", 32 + offset),
        Color::Yellow => format!("{}", 33 + offset),
        Color::Blue => format!("{}", 34 + offset),
        Color::Magenta => format!("{}", 35 + offset),
        Color::Cyan => format!("{}", 36 + offset),
        Color::Gray => format!("{}", 37 + offset),
        Color::DarkGray => format!("{}", 90 + offset),
        Color::LightRed => format!("{}", 91 + offset),
        Color::LightGreen => format!("{}", 92 + offset),
        Color::LightYellow => format!("{}", 93 + offset),
        Color::LightBlue => format!("{}", 94 + offset),
        Color::LightMagenta => format!("{}", 95 + offset),
        Color::LightCyan => format!("{}", 96 + offset),
        Color::White => format!("{}", 97 + offset),
        Color::Rgb(red, green, blue) => {
            format!(
                "{};2;{};{};{}",
                if background { 48 } else { 38 },
                red,
                green,
                blue
            )
        }
        Color::Indexed(index) => format!("{};5;{}", if background { 48 } else { 38 }, index),
    }
}

pub fn interaction_areas(area: Rect, app: &mut App) -> [Rect; 3] {
    app.apply_initial_layout(area.width, COMPACT_LAYOUT_THRESHOLD);
    let [_header, body, _footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    panel_areas_for(
        body,
        app.view.focus,
        app.view.expanded,
        app.view.panel_layout,
        app.has_linked_worktrees() || app.history_active(),
    )
}

fn render_history(frame: &mut Frame, app: &mut App, area: Rect) {
    history::render(frame, app, area);
}

#[cfg(test)]
fn panel_areas(area: Rect, focus: Focus, expanded: bool, panel_layout: PanelLayout) -> [Rect; 3] {
    panel_areas_for(area, focus, expanded, panel_layout, true)
}

fn panel_areas_for(
    area: Rect,
    focus: Focus,
    expanded: bool,
    panel_layout: PanelLayout,
    show_worktrees: bool,
) -> [Rect; 3] {
    layout::panel_areas(area, focus, expanded, panel_layout, show_worktrees)
}

fn render_compact_panel(
    frame: &mut Frame,
    area: Rect,
    title: &'static str,
    item_count: usize,
    selected: Option<usize>,
) {
    let items = (0..item_count).map(|index| {
        let line = if Some(index) == selected {
            Line::styled("›●", SELECTED)
        } else {
            Line::styled(" ●", Style::new().fg(Color::DarkGray))
        };
        ListItem::new(line)
    });
    frame.render_widget(List::new(items).block(pane_block(title, false)), area);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode_label = match (app.history_commit_selected(), app.changes.mode) {
        (true, ChangeMode::Uncommitted) => " COMMIT ",
        (true, ChangeMode::Branch) => " COMMIT RANGE ",
        (false, ChangeMode::Uncommitted) => " UNCOMMITTED ",
        (false, ChangeMode::Branch) => " BRANCH ",
    };
    let mode = match app.changes.mode {
        ChangeMode::Uncommitted => Span::styled(
            mode_label,
            Style::new().fg(Color::Black).bg(Color::Yellow).bold(),
        ),
        ChangeMode::Branch => Span::styled(
            mode_label,
            Style::new().fg(Color::Black).bg(Color::Blue).bold(),
        ),
    };
    let worktree = app
        .selected_worktree()
        .map(|worktree| worktree.path.display().to_string())
        .unwrap_or_else(|| app.repository.directory.display().to_string());
    let detail = match (app.history_commit_selected(), app.changes.mode) {
        (true, ChangeMode::Uncommitted) => "selected commit compared with its parent".to_string(),
        (true, ChangeMode::Branch) => {
            format!(
                "selected commit since divergence from {}",
                app.repository.base
            )
        }
        (false, ChangeMode::Uncommitted) => "staged + unstaged + untracked".to_string(),
        (false, ChangeMode::Branch) => format!("since divergence from {}", app.repository.base),
    };

    let text = Text::from(vec![
        Line::from(vec![
            Span::styled(" git-navigator ", Style::new().bold()),
            mode,
            Span::raw("  "),
            Span::styled(worktree, Style::new().fg(Color::Gray)),
        ]),
        Line::styled(format!(" {detail}"), Style::new().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(text), area);
}

fn render_worktrees(frame: &mut Frame, app: &mut App, area: Rect) {
    let visible = app.visible_worktree_indices();
    let selected = app
        .repository
        .worktree_state
        .selected()
        .and_then(|selected| visible.iter().position(|index| *index == selected));
    let items: Vec<ListItem> = visible
        .iter()
        .map(|index| {
            let worktree = &app.repository.worktrees[*index];
            let marker = if worktree.is_current { "●" } else { " " };
            let dirty = if worktree.dirty { "*" } else { "" };
            let state = if worktree.is_missing() {
                "MISSING "
            } else if worktree.locked_reason.is_some() {
                "LOCKED "
            } else {
                ""
            };
            let state_style = if worktree.is_missing() {
                Style::new().fg(Color::LightRed).bold()
            } else if worktree.locked_reason.is_some() {
                Style::new().fg(Color::Yellow).bold()
            } else {
                Style::default()
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("{marker} "), Style::new().fg(Color::Cyan)),
                    Span::styled(
                        format!("{}{}", directory_name(&worktree.path), dirty),
                        Style::new().bold(),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(state, state_style),
                    Span::styled(
                        format!("{} @ {}", worktree.branch, worktree.head),
                        Style::new().fg(Color::DarkGray),
                    ),
                ]),
            ])
        })
        .collect();
    let list = List::new(items)
        .highlight_style(SELECTED)
        .highlight_symbol("› ");
    let viewport_length =
        (usize::from(area.height.saturating_sub(2)) / WORKTREE_ITEM_HEIGHT).max(1);
    let has_overflow = visible.len() > viewport_length;
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(selected);
    if !has_overflow {
        frame.render_stateful_widget(
            list.block(pane_block(
                &worktree_title(app, visible.len()),
                app.view.focus == Focus::Worktrees,
            )),
            area,
            &mut list_state,
        );
        *app.repository.worktree_state.offset_mut() = list_state.offset();
        return;
    }

    frame.render_widget(
        pane_block(
            &worktree_title(app, visible.len()),
            app.view.focus == Focus::Worktrees,
        ),
        area,
    );
    let content_area = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let list_area = Rect {
        width: content_area.width.saturating_sub(1),
        ..content_area
    };
    frame.render_stateful_widget(list, list_area, &mut list_state);
    *app.repository.worktree_state.offset_mut() = list_state.offset();

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█")
        .style(Style::new().fg(if app.view.focus == Focus::Worktrees {
            ACTIVE_BORDER
        } else {
            INACTIVE_BORDER
        }));
    let mut scrollbar_state = ScrollbarState::new(visible.len())
        .viewport_content_length(viewport_length)
        .position(scrollbar_position(
            app.repository.worktree_state.offset(),
            visible.len(),
            viewport_length,
        ));
    frame.render_stateful_widget(scrollbar, content_area, &mut scrollbar_state);
}

fn scrollbar_position(offset: usize, content_length: usize, viewport_length: usize) -> usize {
    let max_offset = content_length.saturating_sub(viewport_length);
    let max_position = content_length.saturating_sub(1);
    offset
        .min(max_offset)
        .checked_mul(max_position)
        .and_then(|position| position.checked_div(max_offset))
        .unwrap_or(0)
}

fn render_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let effective_layout = app
        .selected_file()
        .map(|file| effective_layout(file, app.view.diff_layout))
        .unwrap_or(app.view.diff_layout);
    let automatic_layout = effective_layout != app.view.diff_layout;
    let title = app
        .selected_file()
        .map(|file| {
            format!(
                "Diff [{}, {}{}{}] - {}",
                app.changes.diff_view.label(),
                effective_layout.label(),
                if automatic_layout { " AUTO" } else { "" },
                if app.view.line_wrap { ", WRAP" } else { "" },
                file.path.display()
            )
        })
        .unwrap_or_else(|| {
            format!(
                "Diff [{}, {}{}]",
                app.changes.diff_view.label(),
                effective_layout.label(),
                if app.view.line_wrap { ", WRAP" } else { "" }
            )
        });
    let title = if let Some(search) = &app.view.search {
        if search.focus == Focus::Diff {
            let position = app
                .diff_search_match_position()
                .map_or_else(String::new, |(current, total)| {
                    format!(" ({current}/{total})")
                });
            format!("{title} /{}/{}", search.query, position)
        } else {
            title
        }
    } else {
        title
    };
    let block = pane_block(&title, app.view.focus == Focus::Diff);

    let Some(file) = app.selected_file() else {
        let message = if app.view.status.is_some() {
            "Unable to load changes"
        } else {
            "No changed files"
        };
        frame.render_widget(
            Paragraph::new(message)
                .block(block)
                .alignment(Alignment::Center)
                .style(Style::new().fg(Color::DarkGray)),
            area,
        );
        return;
    };

    if file.binary {
        frame.render_widget(
            Paragraph::new("Binary file - diff is not displayed")
                .block(block)
                .alignment(Alignment::Center)
                .style(Style::new().fg(Color::Yellow)),
            area,
        );
        return;
    }

    if file.hunks.is_empty() {
        frame.render_widget(
            Paragraph::new("No textual changes")
                .block(block)
                .alignment(Alignment::Center)
                .style(Style::new().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let search_query = app
        .view
        .search
        .as_ref()
        .filter(|search| search.focus == Focus::Diff)
        .map_or("", |search| search.query.as_str());
    let table = diff::render(
        file,
        app.changes.mode,
        effective_layout,
        app.view.line_wrap,
        search_query,
        area,
    )
    .block(block);
    frame.render_stateful_widget(table, area, &mut app.changes.diff_state);
}

fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn pane_block<'a>(title: &'a str, active: bool) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if active {
            ACTIVE_BORDER
        } else {
            INACTIVE_BORDER
        }))
        .title(format!(" {title} "))
}

fn worktree_title(app: &App, visible_count: usize) -> String {
    let count = if app.view.worktree_filter.is_empty() {
        visible_count.to_string()
    } else {
        format!("{visible_count}/{}", app.repository.worktrees.len())
    };
    panel_title(
        &format!("Worktrees ({count})"),
        &app.view.worktree_filter,
        app.view
            .search
            .as_ref()
            .is_some_and(|search| search.focus == Focus::Worktrees),
    )
}

pub(crate) fn file_title(app: &App, visible_count: usize) -> String {
    let count = if app.view.file_filter.is_empty() {
        app.changes.files.len().to_string()
    } else {
        format!("{visible_count}/{}", app.changes.files.len())
    };
    panel_title(
        &format!("Files ({count})"),
        &app.view.file_filter,
        app.view
            .search
            .as_ref()
            .is_some_and(|search| search.focus == Focus::Files),
    )
}

fn panel_title(title: &str, filter: &str, searching: bool) -> String {
    if searching && filter.is_empty() {
        format!("{title} //")
    } else if filter.is_empty() {
        title.to_string()
    } else {
        format!("{title} /{filter}/")
    }
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(search) = &app.view.search {
        search_line(search.focus, &search.query)
    } else if let Some(status) = &app.view.status {
        status_line(status.kind, &status.text)
    } else {
        navigation_line(app.view.focus, app.view.expanded)
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn search_line(focus: Focus, query: &str) -> Line<'static> {
    let panel = match focus {
        Focus::Worktrees => "worktrees",
        Focus::Files => "files",
        Focus::Diff => "diff",
    };
    Line::from(vec![
        Span::styled(
            format!(" Search {panel}: "),
            Style::new().fg(Color::Cyan).bold(),
        ),
        Span::styled(format!("/{query}"), Style::new().fg(Color::White).bold()),
        Span::styled(
            if focus == Focus::Diff {
                "  ↑/↓/Enter next  Esc cancel"
            } else {
                "  Enter apply  Esc cancel"
            },
            Style::new().fg(Color::DarkGray),
        ),
    ])
}

fn navigation_line(focus: Focus, expanded: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled("←/→", Style::new().fg(Color::Cyan)),
        Span::raw(" panes  "),
        Span::styled("↑/↓", Style::new().fg(Color::Cyan)),
        Span::raw(" navigate  "),
        Span::styled("Tab", Style::new().fg(Color::Cyan)),
        Span::raw(" mode  "),
        Span::styled("Space", Style::new().fg(Color::Cyan)),
        Span::raw(if expanded { " minimize  " } else { " expand  " }),
        Span::styled("t", Style::new().fg(Color::Cyan)),
        Span::raw(" layout  "),
        Span::styled("r", Style::new().fg(Color::Cyan)),
        Span::raw(" refresh  "),
    ];
    if focus == Focus::Diff {
        spans.extend([
            Span::styled("Enter", Style::new().fg(Color::Cyan)),
            Span::raw(" fold  "),
            Span::styled("v", Style::new().fg(Color::Cyan)),
            Span::raw(" view  "),
            Span::styled("s", Style::new().fg(Color::Cyan)),
            Span::raw(" split/unified  "),
            Span::styled("w", Style::new().fg(Color::Cyan)),
            Span::raw(" wrap  "),
            Span::styled("c", Style::new().fg(Color::Cyan)),
            Span::raw(" copy location  "),
        ]);
    }
    if focus == Focus::Worktrees {
        spans.extend([
            Span::styled("h", Style::new().fg(Color::Cyan)),
            Span::raw(" history  "),
            Span::styled("d", Style::new().fg(Color::Cyan)),
            Span::raw(" clean worktree  "),
        ]);
    }
    if focus == Focus::Files {
        spans.extend([
            Span::styled("h", Style::new().fg(Color::Cyan)),
            Span::raw(" history  "),
        ]);
    }
    spans.extend([
        Span::styled("?", Style::new().fg(Color::Cyan)),
        Span::raw(" help  "),
        Span::styled("q", Style::new().fg(Color::Cyan)),
        Span::raw(" quit"),
    ]);
    Line::from(spans)
}

fn status_line<'a>(kind: StatusKind, message: &'a str) -> Line<'a> {
    match kind {
        StatusKind::Info => Line::from(vec![
            Span::styled(" Success: ", Style::new().fg(Color::Black).bg(Color::Green)),
            Span::styled(message, Style::new().fg(Color::LightGreen)),
        ]),
        StatusKind::Error => Line::from(vec![
            Span::styled(" Error: ", Style::new().fg(Color::White).bg(Color::Red)),
            Span::styled(message, Style::new().fg(Color::LightRed)),
        ]),
    }
}

fn render_help(frame: &mut Frame) {
    let area = centered_rect(62, 70, frame.area());
    frame.render_widget(Clear, area);
    let help = Text::from(vec![
        Line::styled("Keyboard", Style::new().bold().fg(Color::Cyan)),
        Line::from(""),
        help_line("Left / h", "Focus the pane to the left"),
        help_line("Right / l", "Focus the pane to the right"),
        help_line("Up / k", "Move up or scroll the diff"),
        help_line("Down / j", "Move down or scroll the diff"),
        help_line("Page Up/Down", "Scroll the diff by ten rows"),
        help_line("Home / End", "Jump to the start or end of the diff"),
        help_line("Enter", "Collapse or expand the current hunk"),
        help_line("Tab", "Switch change mode"),
        help_line("Space", "Expand or restore the focused panel"),
        help_line("t", "Cycle panel layout"),
        help_line("v", "Toggle hunks or full-file diff"),
        help_line("s", "Toggle split or unified diff layout"),
        help_line("w", "Toggle wrapping of long diff lines"),
        help_line("c", "Copy selected file path and line"),
        help_line("/", "Search the focused panel"),
        help_line("r", "Refresh worktrees and changes"),
        help_line("o", "Open the selected worktree in the default editor"),
        help_line("d", "Clean up the selected worktree"),
        help_line("? / Esc", "Close this help"),
        help_line("q", "Quit"),
        Line::from(""),
        Line::styled(
            "git-navigator is read-only and never modifies repository state.",
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(help)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(" Help "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn help_line<'a>(key: &'a str, description: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{key:>14}  "), Style::new().fg(Color::Yellow)),
        Span::raw(description),
    ])
}

fn render_delete_confirmation(frame: &mut Frame, confirmation: &crate::app::DeleteConfirmation) {
    let area = centered_rect(66, 30, frame.area());
    frame.render_widget(Clear, area);
    let action = if confirmation.prune_only {
        "The worktree's .git link is missing. This removes only its stale Git metadata."
    } else {
        "This permanently deletes the clean worktree directory and its Git metadata."
    };
    let text = Text::from(vec![
        Line::styled("Remove worktree?", Style::new().fg(Color::LightRed).bold()),
        Line::from(""),
        Line::from(format!("Branch: {}", confirmation.branch)),
        Line::from(format!("Path:   {}", confirmation.path.display())),
        Line::from(""),
        Line::styled(action, Style::new().fg(Color::Yellow)),
        if confirmation.prune_only {
            Line::from("Any remaining directory and files are left untouched.")
        } else {
            Line::from("The worktree directory will be deleted.")
        },
        Line::from("The branch itself is not deleted."),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::new().fg(Color::LightRed).bold()),
            Span::raw(" confirm   "),
            Span::styled("n / Esc", Style::new().fg(Color::Cyan).bold()),
            Span::raw(" cancel"),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(Color::LightRed))
                    .title(" Confirm cleanup "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn centered_rect(horizontal: u16, vertical: u16, area: Rect) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(horizontal)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(vertical)])
        .flex(Flex::Center)
        .areas(area);
    area
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend, widgets::ListState};

    use super::*;
    use crate::model::{
        ChangedFile, DiffHunk, DiffLayout, DiffRow, DiffRowKind, DiffView, FileStatus,
        FileTreeRowKind, HistorySelection, HunkKind, Worktree,
    };

    #[test]
    fn branch_file_colors_follow_change_shape() {
        let mut file = ChangedFile::empty("file".into(), FileStatus::Modified);
        file.additions = 1;
        assert_eq!(file_style(&file, ChangeMode::Branch).fg, Some(Color::Green));
        file.deletions = 1;
        assert_eq!(
            file_style(&file, ChangeMode::Branch).fg,
            Some(Color::Rgb(255, 165, 0))
        );
        file.additions = 0;
        assert_eq!(file_style(&file, ChangeMode::Branch).fg, Some(Color::Red));
    }

    #[test]
    fn informational_status_uses_a_success_label() {
        let line = status_line(StatusKind::Info, "Pruned stale worktree metadata");
        assert_eq!(line.spans[0].content.as_ref(), " Success: ");
        assert_eq!(line.spans[0].style.bg, Some(Color::Green));
        assert_eq!(
            line.spans[1].content.as_ref(),
            "Pruned stale worktree metadata"
        );
    }

    #[test]
    fn error_status_uses_an_error_label() {
        let line = status_line(StatusKind::Error, "Cleanup failed");
        assert_eq!(line.spans[0].content.as_ref(), " Error: ");
        assert_eq!(line.spans[0].style.bg, Some(Color::Red));
    }

    #[test]
    fn cleanup_shortcut_is_only_shown_for_worktree_focus() {
        let worktree_footer = line_text(&navigation_line(Focus::Worktrees, false));
        let files_footer = line_text(&navigation_line(Focus::Files, false));
        let diff_footer = line_text(&navigation_line(Focus::Diff, false));

        assert!(worktree_footer.contains("d clean worktree"));
        assert!(!files_footer.contains("d clean worktree"));
        assert!(!diff_footer.contains("d clean worktree"));
        assert!(!worktree_footer.contains("v view"));
        assert!(!worktree_footer.contains("s split/unified"));
        assert!(!worktree_footer.contains("w wrap"));
        assert!(!worktree_footer.contains("Enter fold"));
        assert!(!files_footer.contains("v view"));
        assert!(!files_footer.contains("s split/unified"));
        assert!(!files_footer.contains("w wrap"));
        assert!(!files_footer.contains("Enter fold"));
        assert!(diff_footer.contains("v view"));
        assert!(diff_footer.contains("s split/unified"));
        assert!(diff_footer.contains("w wrap"));
        assert!(diff_footer.contains("Enter fold"));
        assert!(!diff_footer.contains("wrap:on"));
        assert!(!diff_footer.contains("wrap:off"));
    }

    #[test]
    fn expansion_shortcut_describes_the_next_action() {
        assert!(
            navigation_line(Focus::Diff, false)
                .to_string()
                .contains("Space expand")
        );
        assert!(
            navigation_line(Focus::Diff, true)
                .to_string()
                .contains("Space minimize")
        );
    }

    #[test]
    fn searchable_panel_titles_include_the_active_filter() {
        assert_eq!(
            panel_title("Files (4/4)", "some-search-word", false),
            "Files (4/4) /some-search-word/"
        );
        assert_eq!(panel_title("Files (4)", "", false), "Files (4)");
        assert_eq!(panel_title("Files (4)", "", true), "Files (4) //");
    }

    #[test]
    fn diff_search_highlights_all_case_insensitive_matches() {
        let spans = highlighted_spans("Error error ERROR", Style::new().fg(Color::Red), "error");
        assert_eq!(
            spans.iter().filter(|span| span.style.bg.is_some()).count(),
            3
        );
    }

    #[test]
    fn sidebar_layout_puts_files_below_worktrees_and_widens_diff() {
        let area = Rect::new(0, 0, 100, 40);
        let [worktrees, files, diff] =
            panel_areas(area, Focus::Worktrees, false, PanelLayout::SidebarLeft);

        assert_eq!(worktrees.x, 0);
        assert_eq!(files.x, 0);
        assert_eq!(worktrees.width, 25);
        assert_eq!(files.width, 25);
        assert_eq!(files.y, worktrees.bottom());
        assert_eq!(diff.x, 25);
        assert_eq!(diff.width, 75);
        assert_eq!(diff.height, 40);
    }

    #[test]
    fn layouts_hide_worktrees_when_no_linked_worktree_exists_and_history_is_inactive() {
        let area = Rect::new(0, 0, 100, 40);
        let [worktrees, files, diff] =
            panel_areas_for(area, Focus::Files, false, PanelLayout::Columns, false);

        assert_eq!(worktrees, Rect::default());
        assert_eq!(files.width, 25);
        assert_eq!(diff.x, files.right());
        assert_eq!(diff.width, 75);
    }

    #[test]
    fn history_layouts_keep_the_first_panel_visible_without_linked_worktrees() {
        let area = Rect::new(0, 0, 100, 40);
        let [history, files, diff] =
            panel_areas_for(area, Focus::Files, false, PanelLayout::SidebarLeft, true);

        assert_eq!(history.width, 25);
        assert_eq!(files.width, 25);
        assert_eq!(files.y, history.bottom());
        assert_eq!(diff.x, 25);
        assert_eq!(diff.width, 75);
    }

    #[test]
    fn vertical_layout_puts_diff_below_full_width_top_panels() {
        let area = Rect::new(0, 0, 100, 40);
        let [worktrees, files, diff] =
            panel_areas(area, Focus::Worktrees, false, PanelLayout::SidebarTop);

        assert_eq!(worktrees.x, 0);
        assert_eq!(worktrees.y, 0);
        assert_eq!(worktrees.width, 50);
        assert_eq!(files.x, worktrees.right());
        assert_eq!(files.y, 0);
        assert_eq!(files.width, 50);
        assert_eq!(diff.x, 0);
        assert_eq!(diff.y, 10);
        assert_eq!(diff.width, 100);
        assert_eq!(diff.height, 30);
    }

    #[test]
    fn expanded_layout_ignores_the_panel_layout_preference() {
        let area = Rect::new(0, 0, 100, 40);
        let columns = panel_areas(area, Focus::Diff, true, PanelLayout::Columns);
        let sidebar_left = panel_areas(area, Focus::Diff, true, PanelLayout::SidebarLeft);
        let sidebar_top = panel_areas(area, Focus::Diff, true, PanelLayout::SidebarTop);
        assert_eq!(columns, sidebar_left);
        assert_eq!(columns, sidebar_top);
    }

    #[test]
    fn wraps_long_diff_lines_at_display_width() {
        assert_eq!(display_text(Some("abcdefghij"), true, 4), "abcd\nefgh\nij");
        assert_eq!(display_text(Some("abcdefghij"), false, 4), "abcdefghij");
        assert_eq!(display_text(Some("ab界cd"), true, 4), "ab界\ncd");
    }

    #[test]
    fn unified_modified_rows_stack_deleted_before_added() {
        let lines = unified_modified_lines_with_search(
            "before",
            "after",
            Style::new().fg(Color::Red),
            Style::new().fg(Color::Green),
            "",
        );

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].to_string(), "- before");
        assert_eq!(lines[1].to_string(), "+ after");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn unified_content_uses_the_available_width() {
        let area = Rect::new(0, 0, 100, 20);
        assert!(
            diff_content_width(area, DiffLayout::Unified)
                > diff_content_width(area, DiffLayout::Split)
        );
    }

    #[test]
    fn one_sided_files_always_use_unified_layout() {
        for status in [
            FileStatus::Added,
            FileStatus::Deleted,
            FileStatus::Untracked,
        ] {
            let file = ChangedFile::empty("file".into(), status);
            assert_eq!(
                effective_layout(&file, DiffLayout::Split),
                DiffLayout::Unified
            );
        }

        let modified = ChangedFile::empty("file".into(), FileStatus::Modified);
        assert_eq!(
            effective_layout(&modified, DiffLayout::Split),
            DiffLayout::Split
        );
    }

    #[test]
    fn expands_tabs_to_four_column_stops() {
        assert_eq!(display_text(Some("\tvalue"), false, 80), "    value");
        assert_eq!(display_text(Some("ab\tvalue"), false, 80), "ab  value");
        assert_eq!(
            display_text(Some("abcd\tvalue"), false, 80),
            "abcd    value"
        );
        assert_eq!(display_text(Some("\tabcdef"), true, 6), "    ab\ncdef");
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn renders_the_three_pane_interface() {
        let mut file = ChangedFile::empty(PathBuf::from("src/main.rs"), FileStatus::Modified);
        file.additions = 1;
        file.deletions = 1;
        file.hunks.push(DiffHunk {
            id: crate::model::HunkId::synthetic("render-test"),
            header: "@@ -1 +1 @@".into(),
            kind: HunkKind::Staged,
            collapsed: false,
            rows: vec![DiffRow {
                old_number: Some(1),
                new_number: Some(1),
                old_text: Some("before".into()),
                new_text: Some("after".into()),
                kind: DiffRowKind::Modified,
            }],
        });
        let mut worktree_state = ListState::default();
        worktree_state.select(Some(0));
        let mut file_state = ListState::default();
        file_state.select(Some(0));
        let mut diff_state = ratatui::widgets::TableState::default();
        diff_state.select(Some(0));
        let mut app = App {
            repository: crate::app::RepositoryState {
                directory: PathBuf::from("/repo"),
                base: "main".into(),
                worktrees: vec![
                    Worktree {
                        path: PathBuf::from("/repo"),
                        branch: "feature".into(),
                        head: "12345678".into(),
                        dirty: true,
                        is_current: true,
                        is_main: true,
                        available: true,
                        prunable_reason: None,
                        locked_reason: None,
                    },
                    Worktree {
                        path: PathBuf::from("/missing-worktree"),
                        branch: "stale".into(),
                        head: "87654321".into(),
                        dirty: false,
                        is_current: false,
                        is_main: false,
                        available: false,
                        prunable_reason: Some("gitdir file points to non-existent location".into()),
                        locked_reason: None,
                    },
                ],
                worktree_state,
            },
            changes: crate::app::ChangeState {
                mode: ChangeMode::Uncommitted,
                diff_view: DiffView::Hunks,
                files: vec![file],
                file_tree: vec![crate::model::FileTreeRow {
                    label: "└── src/main.rs".into(),
                    path: std::path::PathBuf::from("src/main.rs"),
                    kind: FileTreeRowKind::File,
                }],
                file_lookup: crate::app::files::FileLookup::default(),
                file_state,
                diff_state,
                selected_file_path: Some(PathBuf::from("src/main.rs")),
            },
            history: crate::app::HistoryState {
                list_state: ListState::default(),
                worktree_panel: crate::app::WorktreePanel::Worktrees,
                commits: Vec::new(),
                range_commits: std::collections::HashSet::new(),
                local_base_hash: None,
                remote_base_hash: None,
                branch_tips: std::collections::HashMap::new(),
                head_hash: None,
                selection: None,
                preferred_file: None,
            },
            view: crate::app::ViewState {
                diff_layout: DiffLayout::Split,
                line_wrap: false,
                expanded: false,
                initial_layout_applied: true,
                panel_layout: PanelLayout::Columns,
                focus: Focus::Worktrees,
                show_help: false,
                delete_confirmation: None,
                status: None,
                worktree_filter: String::new(),
                file_filter: String::new(),
                search: None,
            },
        };
        app.repository
            .worktrees
            .extend((1..=3).map(|index| Worktree {
                path: PathBuf::from(format!("/demo-worktree-{index}")),
                branch: format!("demo-{index}"),
                head: format!("0000000{index}"),
                dirty: true,
                is_current: false,
                is_main: false,
                available: true,
                prunable_reason: None,
                locked_reason: None,
            }));
        let backend = TestBackend::new(140, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal should be created");

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("interface should render");

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Worktrees"));
        assert!(rendered.contains("MISSING"));
        assert!(rendered.contains("Files (1)"));
        assert!(rendered.contains("src/main.rs"));
        assert!(rendered.contains("STAGED"));
        assert!(rendered.contains("before"));
        assert!(rendered.contains("after"));
        assert!(rendered.contains('▲'));
        assert!(rendered.contains('▼'));

        app.repository
            .worktree_state
            .select(Some(app.repository.worktrees.len() - 1));
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("interface should render after scrolling");
        let body = Rect::new(0, 2, 140, 9);
        let [worktrees, _, _] = panel_areas(
            body,
            app.view.focus,
            app.view.expanded,
            app.view.panel_layout,
        );
        let content_area = worktrees.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        let scrollbar_x = content_area.right() - 1;
        assert!(
            terminal.backend().buffer()[(scrollbar_x, content_area.bottom() - 2)].symbol() == "█"
        );

        assert_eq!(scrollbar_position(0, 5, 3), 0);
        assert_eq!(scrollbar_position(1, 5, 3), 2);
        assert_eq!(scrollbar_position(2, 5, 3), 4);

        app.history.worktree_panel = crate::app::WorktreePanel::History;
        app.history.commits = vec![crate::model::Commit {
            hash: "base-hash".into(),
            parents: vec![],
            short_hash: "base-has".into(),
            subject: "base commit".into(),
            graph: vec!["●".into()],
        }];
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("history should render");
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("base-has"));
        assert!(rendered.contains("History (2) - feature"));
        assert!(rendered.contains("base-has"));
    }

    #[test]
    fn branch_history_marks_commits_between_selection_and_base() {
        let commits = vec![
            crate::model::Commit {
                hash: "head".into(),
                parents: vec![],
                short_hash: "head".into(),
                subject: "head commit".into(),
                graph: vec!["●".into()],
            },
            crate::model::Commit {
                hash: "middle".into(),
                parents: vec![],
                short_hash: "middle".into(),
                subject: "middle commit".into(),
                graph: vec!["│".into(), "●".into()],
            },
            crate::model::Commit {
                hash: "base".into(),
                parents: vec![],
                short_hash: "base".into(),
                subject: "base commit".into(),
                graph: vec!["●".into()],
            },
            crate::model::Commit {
                hash: "older".into(),
                parents: vec![],
                short_hash: "older".into(),
                subject: "older commit".into(),
                graph: vec!["●".into()],
            },
        ];
        let mut app = App {
            repository: crate::app::RepositoryState {
                directory: PathBuf::from("/repo"),
                base: "main".into(),
                worktrees: vec![],
                worktree_state: ListState::default(),
            },
            changes: crate::app::ChangeState {
                mode: ChangeMode::Branch,
                diff_view: DiffView::Hunks,
                files: vec![],
                file_tree: vec![],
                file_lookup: crate::app::files::FileLookup::default(),
                file_state: ListState::default(),
                diff_state: ratatui::widgets::TableState::default(),
                selected_file_path: None,
            },
            history: crate::app::HistoryState {
                list_state: ListState::default(),
                worktree_panel: crate::app::WorktreePanel::History,
                commits,
                range_commits: ["middle".to_string()].into_iter().collect(),
                local_base_hash: None,
                remote_base_hash: None,
                branch_tips: std::collections::HashMap::new(),
                head_hash: None,
                selection: Some(HistorySelection::Commit {
                    hash: "middle".into(),
                }),
                preferred_file: None,
            },
            view: crate::app::ViewState {
                diff_layout: DiffLayout::Split,
                line_wrap: false,
                expanded: false,
                initial_layout_applied: true,
                panel_layout: PanelLayout::Columns,
                focus: Focus::Worktrees,
                show_help: false,
                delete_confirmation: None,
                status: None,
                worktree_filter: String::new(),
                file_filter: String::new(),
                search: None,
            },
        };

        assert!(!history_commit_is_in_branch_diff(&app, "head"));
        assert!(history_commit_is_in_branch_diff(&app, "middle"));
        assert!(!history_commit_is_in_branch_diff(&app, "base"));
        assert!(!history_commit_is_in_branch_diff(&app, "older"));

        app.history.selection = Some(HistorySelection::Wip);
        app.history.range_commits.clear();
        assert!(!history_wip_is_in_branch_diff(&app));
    }

    #[test]
    fn branch_history_highlights_only_commit_nodes() {
        let line = graph_line("│╱ ●", true, None);

        assert_eq!(line.spans[0].style.fg, Some(Color::DarkGray));
        assert_eq!(line.spans[1].style.fg, Some(Color::DarkGray));
        assert_eq!(line.spans[2].style.fg, Some(Color::DarkGray));
        assert_eq!(line.spans[3].style.fg, Some(Color::Blue));
    }
}
