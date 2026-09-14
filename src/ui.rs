use std::path::Path;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap,
    },
};

use crate::{
    app::{App, Focus},
    model::{ChangeMode, ChangedFile, DiffRow, DiffRowKind, FileStatus, HunkKind},
};

const ACTIVE_BORDER: Color = Color::Cyan;
const INACTIVE_BORDER: Color = Color::DarkGray;
const SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

pub fn render(frame: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [worktrees, files, diff] = Layout::horizontal([
        Constraint::Percentage(22),
        Constraint::Percentage(28),
        Constraint::Percentage(50),
    ])
    .areas(body);

    render_header(frame, app, header);
    render_worktrees(frame, app, worktrees);
    render_files(frame, app, files);
    render_diff(frame, app, diff);
    render_footer(frame, app, footer);

    if app.show_help {
        render_help(frame);
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.mode {
        ChangeMode::Uncommitted => Span::styled(
            " UNCOMMITTED ",
            Style::new().fg(Color::Black).bg(Color::Yellow).bold(),
        ),
        ChangeMode::Branch => Span::styled(
            " BRANCH ",
            Style::new().fg(Color::Black).bg(Color::Blue).bold(),
        ),
    };
    let worktree = app
        .selected_worktree()
        .map(|worktree| worktree.path.display().to_string())
        .unwrap_or_else(|| app.directory.display().to_string());
    let detail = match app.mode {
        ChangeMode::Uncommitted => "staged + unstaged + untracked".to_string(),
        ChangeMode::Branch => format!("since divergence from {}", app.base),
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
    let items: Vec<ListItem> = app
        .worktrees
        .iter()
        .map(|worktree| {
            let marker = if worktree.is_current { "●" } else { " " };
            let dirty = if worktree.dirty { "*" } else { "" };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("{marker} "), Style::new().fg(Color::Cyan)),
                    Span::styled(
                        format!("{}{}", directory_name(&worktree.path), dirty),
                        Style::new().bold(),
                    ),
                ]),
                Line::styled(
                    format!("  {} @ {}", worktree.branch, worktree.head),
                    Style::new().fg(Color::DarkGray),
                ),
            ])
        })
        .collect();
    let list = List::new(items)
        .block(pane_block("Worktrees", app.focus == Focus::Worktrees))
        .highlight_style(SELECTED)
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut app.worktree_state);
}

fn render_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .file_tree
        .iter()
        .map(|tree_row| {
            let Some(file) = tree_row
                .file_index
                .and_then(|file_index| app.files.get(file_index))
            else {
                return ListItem::new(Line::styled(
                    tree_row.label.clone(),
                    Style::new().fg(Color::Cyan).bold(),
                ));
            };

            let style = file_style(file, app.mode);
            ListItem::new(Line::from(vec![
                Span::styled(tree_row.label.clone(), style),
                Span::styled(
                    format!(
                        "  {} +{} -{}",
                        status_letter(file.status),
                        file.additions,
                        file.deletions
                    ),
                    style,
                ),
            ]))
        })
        .collect();
    let title = format!("Files ({})", app.files.len());
    let list = List::new(items)
        .block(pane_block(&title, app.focus == Focus::Files))
        .highlight_style(SELECTED)
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut app.file_state);
}

fn render_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = app
        .selected_file()
        .map(|file| format!("Diff [{}] - {}", app.diff_view.label(), file.path.display()))
        .unwrap_or_else(|| format!("Diff [{}]", app.diff_view.label()));
    let block = pane_block(&title, app.focus == Focus::Diff);

    let Some(file) = app.selected_file() else {
        let message = if app.status.is_some() {
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

    let rows = diff_rows(file, app.mode);
    let widths = [
        Constraint::Length(5),
        Constraint::Percentage(50),
        Constraint::Length(5),
        Constraint::Percentage(50),
    ];
    let header = Row::new(["Old", "Before", "New", "After"])
        .style(Style::new().fg(Color::Gray).bold())
        .bottom_margin(1);
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(Color::Rgb(35, 42, 52)))
        .highlight_symbol("›");
    frame.render_stateful_widget(table, area, &mut app.diff_state);
}

fn diff_rows(file: &ChangedFile, mode: ChangeMode) -> Vec<Row<'static>> {
    let mut rendered = Vec::new();
    for hunk in &file.hunks {
        let badge_style = hunk_badge_style(hunk.kind);
        let marker = if hunk.collapsed { "▸" } else { "▾" };
        rendered.push(
            Row::new([
                Cell::from(marker),
                Cell::from(Line::from(vec![
                    Span::styled(format!(" {} ", hunk.kind.label()), badge_style),
                    Span::raw(" "),
                    Span::styled(hunk.header.clone(), Style::new().fg(Color::Cyan)),
                ])),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::new().bg(Color::Rgb(24, 29, 37))),
        );
        if !hunk.collapsed {
            rendered.extend(
                hunk.rows
                    .iter()
                    .map(|row| render_diff_row(row, mode, hunk.kind)),
            );
        }
    }
    rendered
}

fn render_diff_row(row: &DiffRow, mode: ChangeMode, hunk_kind: HunkKind) -> Row<'static> {
    let (old_style, new_style) = diff_styles(row.kind, mode, hunk_kind);
    Row::new([
        Cell::from(line_number(row.old_number)).style(old_style),
        Cell::from(row.old_text.clone().unwrap_or_default()).style(old_style),
        Cell::from(line_number(row.new_number)).style(new_style),
        Cell::from(row.new_text.clone().unwrap_or_default()).style(new_style),
    ])
}

fn diff_styles(kind: DiffRowKind, mode: ChangeMode, hunk_kind: HunkKind) -> (Style, Style) {
    let context = Style::new().fg(Color::Gray);
    let (deleted, added) = if mode == ChangeMode::Uncommitted && hunk_kind == HunkKind::Staged {
        (
            Style::new().fg(Color::LightRed),
            Style::new().fg(Color::LightGreen),
        )
    } else {
        (Style::new().fg(Color::Red), Style::new().fg(Color::Green))
    };

    match kind {
        DiffRowKind::Context => (context, context),
        DiffRowKind::Added => (Style::default(), added),
        DiffRowKind::Deleted => (deleted, Style::default()),
        DiffRowKind::Modified => (deleted, added),
    }
}

fn hunk_badge_style(kind: HunkKind) -> Style {
    match kind {
        HunkKind::Staged => Style::new().fg(Color::Black).bg(Color::LightGreen).bold(),
        HunkKind::Unstaged => Style::new().fg(Color::Black).bg(Color::Yellow).bold(),
        HunkKind::Combined => Style::new().fg(Color::White).bg(Color::Blue).bold(),
        HunkKind::FullFile => Style::new().fg(Color::Black).bg(Color::Cyan).bold(),
        HunkKind::Untracked => Style::new().fg(Color::Black).bg(Color::Cyan).bold(),
    }
}

fn file_style(file: &ChangedFile, mode: ChangeMode) -> Style {
    if mode == ChangeMode::Branch {
        match (file.additions > 0, file.deletions > 0) {
            (true, true) => Style::new().fg(Color::Blue),
            (true, false) => Style::new().fg(Color::Green),
            (false, true) => Style::new().fg(Color::Red),
            (false, false) => Style::new().fg(Color::Gray),
        }
    } else {
        match file.status {
            FileStatus::Added | FileStatus::Untracked => Style::new().fg(Color::Green),
            FileStatus::Deleted => Style::new().fg(Color::Red),
            FileStatus::Conflicted => Style::new().fg(Color::LightRed),
            FileStatus::Modified | FileStatus::Renamed => Style::new().fg(Color::Yellow),
        }
    }
}

fn status_letter(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "A",
        FileStatus::Deleted => "D",
        FileStatus::Modified => "M",
        FileStatus::Renamed => "R",
        FileStatus::Untracked => "?",
        FileStatus::Conflicted => "U",
    }
}

fn line_number(number: Option<usize>) -> String {
    number.map_or_else(String::new, |number| number.to_string())
}

fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn pane_block<'a>(title: &'a str, active: bool) -> Block<'a> {
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

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(status) = &app.status {
        Line::from(vec![
            Span::styled(" Error: ", Style::new().fg(Color::White).bg(Color::Red)),
            Span::styled(status, Style::new().fg(Color::LightRed)),
        ])
    } else {
        Line::from(vec![
            Span::styled("←/→", Style::new().fg(Color::Cyan)),
            Span::raw(" panes  "),
            Span::styled("↑/↓", Style::new().fg(Color::Cyan)),
            Span::raw(" navigate  "),
            Span::styled("Tab", Style::new().fg(Color::Cyan)),
            Span::raw(" mode  "),
            Span::styled("v", Style::new().fg(Color::Cyan)),
            Span::raw(" view  "),
            Span::styled("Enter", Style::new().fg(Color::Cyan)),
            Span::raw(" fold  "),
            Span::styled("r", Style::new().fg(Color::Cyan)),
            Span::raw(" refresh  "),
            Span::styled("?", Style::new().fg(Color::Cyan)),
            Span::raw(" help  "),
            Span::styled("q", Style::new().fg(Color::Cyan)),
            Span::raw(" quit"),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
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
        help_line("v", "Toggle hunks or full-file diff"),
        help_line("r", "Refresh worktrees and changes"),
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
    use crate::model::{DiffHunk, DiffView, HunkKind, Worktree};

    #[test]
    fn branch_file_colors_follow_change_shape() {
        let mut file = ChangedFile::empty("file".into(), FileStatus::Modified);
        file.additions = 1;
        assert_eq!(file_style(&file, ChangeMode::Branch).fg, Some(Color::Green));
        file.deletions = 1;
        assert_eq!(file_style(&file, ChangeMode::Branch).fg, Some(Color::Blue));
        file.additions = 0;
        assert_eq!(file_style(&file, ChangeMode::Branch).fg, Some(Color::Red));
    }

    #[test]
    fn renders_the_three_pane_interface() {
        let mut file = ChangedFile::empty(PathBuf::from("src/main.rs"), FileStatus::Modified);
        file.additions = 1;
        file.deletions = 1;
        file.hunks.push(DiffHunk {
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
            directory: PathBuf::from("/repo"),
            base: "main".into(),
            mode: ChangeMode::Uncommitted,
            diff_view: DiffView::Hunks,
            focus: Focus::Worktrees,
            worktrees: vec![Worktree {
                path: PathBuf::from("/repo"),
                branch: "feature".into(),
                head: "12345678".into(),
                dirty: true,
                is_current: true,
            }],
            files: vec![file],
            file_tree: vec![crate::model::FileTreeRow {
                label: "└── src/main.rs".into(),
                file_index: Some(0),
            }],
            worktree_state,
            file_state,
            diff_state,
            show_help: false,
            status: None,
        };
        let backend = TestBackend::new(140, 30);
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
        assert!(rendered.contains("Files (1)"));
        assert!(rendered.contains("src/main.rs"));
        assert!(rendered.contains("STAGED"));
        assert!(rendered.contains("before"));
        assert!(rendered.contains("after"));
    }
}
