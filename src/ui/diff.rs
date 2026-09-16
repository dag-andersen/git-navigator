use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Cell, Row, Table},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::model::{
    ChangeMode, ChangedFile, DiffLayout, DiffRow, DiffRowKind, FileStatus, HunkKind,
};

const SEARCH_MATCH_BG: Color = Color::Rgb(100, 80, 0);

pub(crate) fn effective_layout(file: &ChangedFile, preferred: DiffLayout) -> DiffLayout {
    match file.status {
        FileStatus::Added | FileStatus::Deleted | FileStatus::Untracked => DiffLayout::Unified,
        FileStatus::Modified | FileStatus::Renamed | FileStatus::Conflicted => preferred,
    }
}

pub(crate) fn render(
    file: &ChangedFile,
    mode: ChangeMode,
    layout: DiffLayout,
    line_wrap: bool,
    search_query: &str,
    area: Rect,
) -> Table<'static> {
    let rows = rows(file, mode, layout, line_wrap, search_query, area);
    let (widths, header) = match layout {
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
    Table::new(rows, widths)
        .header(
            header
                .style(Style::new().fg(Color::Gray).bold())
                .bottom_margin(1),
        )
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(Color::Rgb(35, 42, 52)))
        .highlight_symbol("›")
}

fn rows(
    file: &ChangedFile,
    mode: ChangeMode,
    layout: DiffLayout,
    line_wrap: bool,
    search_query: &str,
    area: Rect,
) -> Vec<Row<'static>> {
    let mut rendered = Vec::new();
    for hunk in &file.hunks {
        let badge_style = hunk_badge_style(hunk.kind);
        let marker = if hunk.collapsed { "▸" } else { "▾" };
        rendered.push(
            Row::new([
                Cell::from(marker),
                Cell::from(Line::from(
                    [
                        vec![Span::styled(
                            format!(" {} ", hunk.kind.label()),
                            badge_style,
                        )],
                        vec![Span::raw(" ")],
                        highlighted_spans(&hunk.header, Style::new().fg(Color::Cyan), search_query),
                    ]
                    .concat(),
                )),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::new().bg(Color::Rgb(24, 29, 37))),
        );
        if !hunk.collapsed {
            rendered.extend(hunk.rows.iter().map(|row| {
                render_row(row, mode, hunk.kind, layout, line_wrap, search_query, area)
            }));
        }
    }
    rendered
}

fn render_row(
    row: &DiffRow,
    mode: ChangeMode,
    hunk_kind: HunkKind,
    layout: DiffLayout,
    line_wrap: bool,
    search_query: &str,
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
                Cell::from(highlighted_text(&old_text, old_style, search_query)),
                Cell::from(line_number(row.new_number)).style(new_style),
                Cell::from(highlighted_text(&new_text, new_style, search_query)),
            ])
            .height(height)
        }
        DiffLayout::Unified => render_unified_row(
            row,
            old_style,
            new_style,
            line_wrap,
            search_query,
            content_width,
        ),
    }
}

fn render_unified_row(
    row: &DiffRow,
    old_style: Style,
    new_style: Style,
    line_wrap: bool,
    search_query: &str,
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
            search_query,
            content_width,
        ),
        DiffRowKind::Deleted => unified_row(
            row.old_number,
            None,
            "-",
            row.old_text.as_deref(),
            old_style,
            line_wrap,
            search_query,
            content_width,
        ),
        DiffRowKind::Added => unified_row(
            None,
            row.new_number,
            "+",
            row.new_text.as_deref(),
            new_style,
            line_wrap,
            search_query,
            content_width,
        ),
        DiffRowKind::Modified => unified_modified_row(
            row,
            old_style,
            new_style,
            line_wrap,
            search_query,
            content_width,
        ),
    }
}

#[expect(clippy::too_many_arguments)]
fn unified_row(
    old_number: Option<usize>,
    new_number: Option<usize>,
    prefix: &'static str,
    text: Option<&str>,
    style: Style,
    line_wrap: bool,
    search_query: &str,
    content_width: usize,
) -> Row<'static> {
    let text = display_text(text, line_wrap, content_width.saturating_sub(2).max(1));
    let height = text.split('\n').count() as u16;
    Row::new([
        Cell::from(line_number(old_number)).style(style),
        Cell::from(line_number(new_number)).style(style),
        Cell::from(Text::from(prefixed_lines_with_search(
            prefix,
            &text,
            style,
            search_query,
        ))),
        Cell::from(""),
    ])
    .height(height)
}

fn unified_modified_row(
    row: &DiffRow,
    old_style: Style,
    new_style: Style,
    line_wrap: bool,
    search_query: &str,
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
    let code_lines = unified_modified_lines_with_search(
        &old_text,
        &new_text,
        old_style,
        new_style,
        search_query,
    );

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

pub(crate) fn unified_modified_lines_with_search(
    old_text: &str,
    new_text: &str,
    old_style: Style,
    new_style: Style,
    query: &str,
) -> Vec<Line<'static>> {
    let mut lines = prefixed_lines_with_search("-", old_text, old_style, query);
    lines.extend(prefixed_lines_with_search("+", new_text, new_style, query));
    lines
}

fn prefixed_lines_with_search(
    prefix: &'static str,
    text: &str,
    style: Style,
    query: &str,
) -> Vec<Line<'static>> {
    text.split('\n')
        .enumerate()
        .map(|(index, line)| {
            let prefix = if index == 0 { prefix } else { " " };
            let mut spans = vec![Span::styled(format!("{prefix} "), style)];
            spans.extend(highlighted_spans(line, style, query));
            Line::from(spans)
        })
        .collect()
}

fn highlighted_text(text: &str, style: Style, query: &str) -> Text<'static> {
    Text::from(
        text.split('\n')
            .map(|line| Line::from(highlighted_spans(line, style, query)))
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn highlighted_spans(text: &str, style: Style, query: &str) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![Span::styled(text.to_string(), style)];
    }

    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();
    let mut spans = Vec::new();
    let mut start = 0;
    while let Some(relative_start) = lower_text[start..].find(&lower_query) {
        let match_start = start + relative_start;
        if match_start > start {
            spans.push(Span::styled(text[start..match_start].to_string(), style));
        }
        let match_end = match_start + lower_query.len();
        spans.push(Span::styled(
            text[match_start..match_end].to_string(),
            style.bg(SEARCH_MATCH_BG),
        ));
        start = match_end;
    }
    if start < text.len() {
        spans.push(Span::styled(text[start..].to_string(), style));
    }
    spans
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

pub(crate) fn diff_content_width(area: Rect, layout: DiffLayout) -> usize {
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

pub(crate) fn display_text(text: Option<&str>, line_wrap: bool, width: usize) -> String {
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

fn line_number(number: Option<usize>) -> String {
    number.map_or_else(String::new, |number| number.to_string())
}
