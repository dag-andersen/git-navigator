use ratatui::layout::{Position, Rect};
use unicode_width::UnicodeWidthChar;

use crate::model::{ChangedFile, DiffLayout, DiffRowKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionState {
    pub(crate) hunk_header: String,
    pub(crate) row_in_hunk: usize,
    pub(crate) scroll_offset: usize,
    pub(crate) collapsed_hunks: Vec<String>,
}

pub(crate) fn first_row(
    files: &[ChangedFile],
    tree: &[crate::model::FileTreeRow],
    selected: Option<usize>,
) -> Option<usize> {
    selected
        .and_then(|row| tree.get(row))
        .and_then(|row| row.file_index)
        .and_then(|index| files.get(index))
        .filter(|file| !file.hunks.is_empty())
        .map(|_| 0)
}

pub(crate) fn hunk_at_row(file: &ChangedFile, row: usize) -> Option<usize> {
    let mut start = 0;
    for (index, hunk) in file.hunks.iter().enumerate() {
        let height = 1 + usize::from(!hunk.collapsed) * hunk.rows.len();
        if row < start + height {
            return Some(index);
        }
        start += height;
    }
    None
}

pub(crate) fn row_at(file: &ChangedFile, row: usize) -> Option<&crate::model::DiffRow> {
    let mut current = 0;
    for hunk in &file.hunks {
        if current == row {
            return hunk.rows.first();
        }
        current += 1;
        if !hunk.collapsed {
            if row < current + hunk.rows.len() {
                return hunk.rows.get(row - current);
            }
            current += hunk.rows.len();
        }
    }
    None
}

pub(crate) fn row_position(file: &ChangedFile, row: usize) -> Option<(usize, usize)> {
    let mut start = 0;
    for (index, hunk) in file.hunks.iter().enumerate() {
        let height = 1 + usize::from(!hunk.collapsed) * hunk.rows.len();
        if row < start + height {
            return Some((index, row.saturating_sub(start)));
        }
        start += height;
    }
    None
}

pub(crate) fn row_for_position(file: &ChangedFile, position: &PositionState) -> Option<usize> {
    let mut start = 0;
    for hunk in &file.hunks {
        if hunk.header == position.hunk_header {
            let max_row = if hunk.collapsed { 0 } else { hunk.rows.len() };
            return Some(start + position.row_in_hunk.min(max_row));
        }
        start += 1 + usize::from(!hunk.collapsed) * hunk.rows.len();
    }
    None
}

pub(crate) fn row_at_position(
    file: &ChangedFile,
    layout: DiffLayout,
    line_wrap: bool,
    offset: usize,
    position: Position,
    area: Rect,
) -> Option<usize> {
    let rows_area_top = area.y.saturating_add(3);
    if position.y < rows_area_top {
        return None;
    }

    let width = content_width(area, layout);
    let mut y = rows_area_top;
    let mut global_row = 0;
    for hunk in &file.hunks {
        if global_row >= offset {
            if position.y < y.saturating_add(1) {
                return Some(global_row);
            }
            y = y.saturating_add(1);
        }
        global_row += 1;
        if hunk.collapsed {
            continue;
        }
        for row in &hunk.rows {
            let height = row_height(row, layout, line_wrap, width);
            if global_row >= offset {
                if position.y < y.saturating_add(height) {
                    return Some(global_row);
                }
                y = y.saturating_add(height);
            }
            global_row += 1;
        }
    }
    None
}

pub(crate) fn content_width(area: Rect, layout: DiffLayout) -> usize {
    let available = area.width.saturating_sub(2 + 10 + 3 + 1);
    match layout {
        DiffLayout::Split => usize::from(available / 2).max(1),
        DiffLayout::Unified => usize::from(available).max(1),
    }
}

fn row_height(
    row: &crate::model::DiffRow,
    layout: DiffLayout,
    line_wrap: bool,
    width: usize,
) -> u16 {
    let text_width = match layout {
        DiffLayout::Split => width,
        DiffLayout::Unified => width.saturating_sub(2).max(1),
    };
    let line_count = |text: Option<&str>, width: usize| {
        if !line_wrap {
            return 1;
        }
        expand_tabs(text.unwrap_or_default(), 4)
            .split('\n')
            .map(|line| wrapped_line_count(line, width))
            .sum::<usize>()
            .max(1) as u16
    };
    match layout {
        DiffLayout::Split => line_count(row.old_text.as_deref(), text_width)
            .max(line_count(row.new_text.as_deref(), text_width)),
        DiffLayout::Unified => match row.kind {
            DiffRowKind::Modified => line_count(row.old_text.as_deref(), text_width)
                .saturating_add(line_count(row.new_text.as_deref(), text_width)),
            _ => line_count(
                row.new_text.as_deref().or(row.old_text.as_deref()),
                text_width,
            ),
        },
    }
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    text.split('\n')
        .map(|line| {
            let mut current_width = 0;
            let mut count = 1;
            for character in line.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if current_width > 0 && current_width + character_width > width {
                    count += 1;
                    current_width = 0;
                }
                current_width += character_width;
            }
            count
        })
        .sum::<usize>()
        .max(1)
}

pub(crate) fn expand_tabs(text: &str, tab_width: usize) -> String {
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
