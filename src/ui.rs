use std::path::Path;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{App, Focus, PanelLayout, StatusKind},
    model::{ChangeMode, ChangedFile, DiffLayout, DiffRow, DiffRowKind, FileStatus, HunkKind},
};

const ACTIVE_BORDER: Color = Color::Cyan;
const INACTIVE_BORDER: Color = Color::DarkGray;
const COMPACT_LAYOUT_THRESHOLD: u16 = 120;
const WORKTREE_ITEM_HEIGHT: usize = 2;
const SELECTED: Style = Style::new()
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
    let [worktrees, files, diff] = panel_areas(body, app.focus, app.expanded, app.panel_layout);
    if app.expanded && app.focus != Focus::Worktrees {
        render_compact_panel(
            frame,
            worktrees,
            "W",
            app.worktrees.len(),
            app.worktree_state.selected(),
        );
    } else {
        render_worktrees(frame, app, worktrees);
    }
    if app.expanded && app.focus != Focus::Files {
        render_compact_panel(
            frame,
            files,
            "F",
            app.files.len(),
            app.selected_file_index(),
        );
    } else {
        render_files(frame, app, files);
    }
    if app.expanded && app.focus != Focus::Diff {
        render_compact_panel(
            frame,
            diff,
            "D",
            app.selected_file().map_or(0, |file| file.hunks.len()),
            app.selected_hunk_index(),
        );
    } else {
        render_diff(frame, app, diff);
    }
    render_footer(frame, app, footer);

    if app.show_help {
        render_help(frame);
    }
    if let Some(confirmation) = &app.delete_confirmation {
        render_delete_confirmation(frame, confirmation);
    }
}

fn panel_areas(area: Rect, focus: Focus, expanded: bool, panel_layout: PanelLayout) -> [Rect; 3] {
    if expanded {
        let constraints = match focus {
            Focus::Worktrees => [
                Constraint::Fill(1),
                Constraint::Length(5),
                Constraint::Length(5),
            ],
            Focus::Files => [
                Constraint::Length(5),
                Constraint::Fill(1),
                Constraint::Length(5),
            ],
            Focus::Diff => [
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Fill(1),
            ],
        };
        return Layout::horizontal(constraints).areas(area);
    }

    match panel_layout {
        PanelLayout::Columns => Layout::horizontal([
            Constraint::Percentage(18),
            Constraint::Percentage(22),
            Constraint::Percentage(60),
        ])
        .areas(area),
        PanelLayout::SidebarLeft => {
            let [sidebar, diff] =
                Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)])
                    .areas(area);
            let [worktrees, files] =
                Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .areas(sidebar);
            [worktrees, files, diff]
        }
        PanelLayout::SidebarTop => {
            let [top, diff] =
                Layout::vertical([Constraint::Percentage(25), Constraint::Percentage(75)])
                    .areas(area);
            let [worktrees, files] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(top);
            [worktrees, files, diff]
        }
    }
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
    let has_overflow = app.worktrees.len() > viewport_length;
    if !has_overflow {
        frame.render_stateful_widget(
            list.block(pane_block("Worktrees", app.focus == Focus::Worktrees)),
            area,
            &mut app.worktree_state,
        );
        return;
    }

    frame.render_widget(pane_block("Worktrees", app.focus == Focus::Worktrees), area);
    let content_area = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let list_area = Rect {
        width: content_area.width.saturating_sub(1),
        ..content_area
    };
    frame.render_stateful_widget(list, list_area, &mut app.worktree_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█")
        .style(Style::new().fg(if app.focus == Focus::Worktrees {
            ACTIVE_BORDER
        } else {
            INACTIVE_BORDER
        }));
    let mut scrollbar_state = ScrollbarState::new(app.worktrees.len())
        .viewport_content_length(viewport_length)
        .position(scrollbar_position(
            app.worktree_state.offset(),
            app.worktrees.len(),
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
    let effective_layout = app
        .selected_file()
        .map(|file| effective_diff_layout(file, app.diff_layout))
        .unwrap_or(app.diff_layout);
    let automatic_layout = effective_layout != app.diff_layout;
    let title = app
        .selected_file()
        .map(|file| {
            format!(
                "Diff [{}, {}{}{}] - {}",
                app.diff_view.label(),
                effective_layout.label(),
                if automatic_layout { " AUTO" } else { "" },
                if app.line_wrap { ", WRAP" } else { "" },
                file.path.display()
            )
        })
        .unwrap_or_else(|| {
            format!(
                "Diff [{}, {}{}]",
                app.diff_view.label(),
                effective_layout.label(),
                if app.line_wrap { ", WRAP" } else { "" }
            )
        });
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

    let rows = diff_rows(file, app.mode, effective_layout, app.line_wrap, area);
    let (widths, header) = match effective_layout {
        DiffLayout::Split => (
            vec![
                Constraint::Length(5),
                Constraint::Percentage(50),
                Constraint::Length(5),
                Constraint::Percentage(50),
            ],
            Row::new(["Old", "Before", "New", "After"]),
        ),
        DiffLayout::Unified => (
            vec![
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Fill(1),
                Constraint::Length(0),
            ],
            Row::new(["Old", "New", "Change", ""]),
        ),
    };
    let header = header
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

fn effective_diff_layout(file: &ChangedFile, preferred: DiffLayout) -> DiffLayout {
    match file.status {
        FileStatus::Added | FileStatus::Deleted | FileStatus::Untracked => DiffLayout::Unified,
        FileStatus::Modified | FileStatus::Renamed | FileStatus::Conflicted => preferred,
    }
}

fn diff_rows(
    file: &ChangedFile,
    mode: ChangeMode,
    layout: DiffLayout,
    line_wrap: bool,
    area: Rect,
) -> Vec<Row<'static>> {
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
                    .map(|row| render_diff_row(row, mode, hunk.kind, layout, line_wrap, area)),
            );
        }
    }
    rendered
}

fn render_diff_row(
    row: &DiffRow,
    mode: ChangeMode,
    hunk_kind: HunkKind,
    layout: DiffLayout,
    line_wrap: bool,
    area: Rect,
) -> Row<'static> {
    let content_width = diff_content_width(area, layout);
    let (old_style, new_style) = diff_styles(row.kind, mode, hunk_kind);
    match layout {
        DiffLayout::Split => {
            let old_text = display_text(row.old_text.as_deref(), line_wrap, content_width);
            let new_text = display_text(row.new_text.as_deref(), line_wrap, content_width);
            let height = old_text
                .lines()
                .count()
                .max(new_text.lines().count())
                .max(1) as u16;
            Row::new([
                Cell::from(line_number(row.old_number)).style(old_style),
                Cell::from(old_text).style(old_style),
                Cell::from(line_number(row.new_number)).style(new_style),
                Cell::from(new_text).style(new_style),
            ])
            .height(height)
        }
        DiffLayout::Unified => {
            render_unified_diff_row(row, old_style, new_style, line_wrap, content_width)
        }
    }
}

fn render_unified_diff_row(
    row: &DiffRow,
    old_style: Style,
    new_style: Style,
    line_wrap: bool,
    content_width: usize,
) -> Row<'static> {
    match row.kind {
        DiffRowKind::Context => unified_row(
            row.old_number,
            row.new_number,
            " ",
            row.new_text.as_deref().or(row.old_text.as_deref()),
            old_style,
            line_wrap,
            content_width,
        ),
        DiffRowKind::Deleted => unified_row(
            row.old_number,
            None,
            "-",
            row.old_text.as_deref(),
            old_style,
            line_wrap,
            content_width,
        ),
        DiffRowKind::Added => unified_row(
            None,
            row.new_number,
            "+",
            row.new_text.as_deref(),
            new_style,
            line_wrap,
            content_width,
        ),
        DiffRowKind::Modified => {
            unified_modified_row(row, old_style, new_style, line_wrap, content_width)
        }
    }
}

fn unified_row(
    old_number: Option<usize>,
    new_number: Option<usize>,
    prefix: &'static str,
    text: Option<&str>,
    style: Style,
    line_wrap: bool,
    content_width: usize,
) -> Row<'static> {
    let text = display_text(text, line_wrap, content_width.saturating_sub(2).max(1));
    let height = text.split('\n').count() as u16;
    Row::new([
        Cell::from(line_number(old_number)).style(style),
        Cell::from(line_number(new_number)).style(style),
        Cell::from(Text::from(prefixed_lines(prefix, &text, style))),
        Cell::from(""),
    ])
    .height(height)
}

fn unified_modified_row(
    row: &DiffRow,
    old_style: Style,
    new_style: Style,
    line_wrap: bool,
    content_width: usize,
) -> Row<'static> {
    let old_text = display_text(
        row.old_text.as_deref(),
        line_wrap,
        content_width.saturating_sub(2).max(1),
    );
    let new_text = display_text(
        row.new_text.as_deref(),
        line_wrap,
        content_width.saturating_sub(2).max(1),
    );
    let old_height = old_text.split('\n').count();
    let new_height = new_text.split('\n').count();
    let code_lines = unified_modified_lines(&old_text, &new_text, old_style, new_style);

    Row::new([
        Cell::from(number_lines(
            row.old_number,
            0,
            old_height,
            new_height,
            old_style,
        )),
        Cell::from(number_lines(
            row.new_number,
            old_height,
            new_height,
            0,
            new_style,
        )),
        Cell::from(Text::from(code_lines)),
        Cell::from(""),
    ])
    .height((old_height + new_height) as u16)
}

fn unified_modified_lines(
    old_text: &str,
    new_text: &str,
    old_style: Style,
    new_style: Style,
) -> Vec<Line<'static>> {
    let mut lines = prefixed_lines("-", old_text, old_style);
    lines.extend(prefixed_lines("+", new_text, new_style));
    lines
}

fn prefixed_lines(prefix: &'static str, text: &str, style: Style) -> Vec<Line<'static>> {
    text.split('\n')
        .enumerate()
        .map(|(index, line)| {
            Line::styled(
                format!("{} {line}", if index == 0 { prefix } else { " " }),
                style,
            )
        })
        .collect()
}

fn number_lines(
    number: Option<usize>,
    leading_blanks: usize,
    own_height: usize,
    trailing_blanks: usize,
    style: Style,
) -> Text<'static> {
    let mut lines = if leading_blanks > 0 {
        vec![Line::raw(""); leading_blanks]
    } else {
        Vec::new()
    };
    lines.push(Line::styled(line_number(number), style));
    lines.extend((1..own_height).map(|_| Line::raw("")));
    lines.extend((0..trailing_blanks).map(|_| Line::raw("")));
    Text::from(lines)
}

fn diff_content_width(area: Rect, layout: DiffLayout) -> usize {
    const BORDER_WIDTH: u16 = 2;
    const COLUMN_SPACING: u16 = 3;
    const HIGHLIGHT_SYMBOL_WIDTH: u16 = 1;
    let available = area
        .width
        .saturating_sub(BORDER_WIDTH + 10 + COLUMN_SPACING + HIGHLIGHT_SYMBOL_WIDTH);
    match layout {
        DiffLayout::Split => usize::from(available / 2).max(1),
        DiffLayout::Unified => usize::from(available).max(1),
    }
}

fn display_text(text: Option<&str>, line_wrap: bool, width: usize) -> String {
    let text = text.unwrap_or_default();
    let expanded = expand_tabs(text, 4);
    if !line_wrap || UnicodeWidthStr::width(expanded.as_str()) <= width {
        return expanded;
    }

    let mut wrapped = String::new();
    let mut current_width = 0;
    for character in expanded.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current_width > 0 && current_width + character_width > width {
            wrapped.push('\n');
            current_width = 0;
        }
        wrapped.push(character);
        current_width += character_width;
    }
    wrapped
}

fn expand_tabs(text: &str, tab_width: usize) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut column = 0;
    for character in text.chars() {
        if character == '\t' {
            let spaces = tab_width - column % tab_width;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    expanded
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
            (true, true) => Style::new().fg(Color::Rgb(255, 165, 0)),
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
        status_line(status.kind, &status.text)
    } else {
        navigation_line(app.focus, app.expanded)
    };
    frame.render_widget(Paragraph::new(line), area);
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
        ]);
    }
    if focus == Focus::Worktrees {
        spans.extend([
            Span::styled("d", Style::new().fg(Color::Cyan)),
            Span::raw(" clean worktree  "),
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
        help_line("r", "Refresh worktrees and changes"),
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
    use crate::model::{DiffHunk, DiffLayout, DiffView, HunkKind, Worktree};

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
        let lines = unified_modified_lines(
            "before",
            "after",
            Style::new().fg(Color::Red),
            Style::new().fg(Color::Green),
        );

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].to_string(), "- before");
        assert_eq!(lines[1].to_string(), "+ after");
        assert_eq!(lines[0].style.fg, Some(Color::Red));
        assert_eq!(lines[1].style.fg, Some(Color::Green));
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
                effective_diff_layout(&file, DiffLayout::Split),
                DiffLayout::Unified
            );
        }

        let modified = ChangedFile::empty("file".into(), FileStatus::Modified);
        assert_eq!(
            effective_diff_layout(&modified, DiffLayout::Split),
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
            diff_layout: DiffLayout::Split,
            line_wrap: false,
            expanded: false,
            initial_layout_applied: true,
            panel_layout: PanelLayout::Columns,
            focus: Focus::Worktrees,
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
            files: vec![file],
            file_tree: vec![crate::model::FileTreeRow {
                label: "└── src/main.rs".into(),
                file_index: Some(0),
            }],
            worktree_state,
            file_state,
            diff_state,
            show_help: false,
            delete_confirmation: None,
            status: None,
        };
        app.worktrees.extend((1..=3).map(|index| Worktree {
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

        app.worktree_state.select(Some(app.worktrees.len() - 1));
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("interface should render after scrolling");
        let body = Rect::new(0, 2, 140, 9);
        let [worktrees, _, _] = panel_areas(body, app.focus, app.expanded, app.panel_layout);
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
    }
}
