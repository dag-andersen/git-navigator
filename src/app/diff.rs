use ratatui::layout::{Position, Rect};

use crate::{
    diff_geometry::{content_width, row_height},
    model::{ChangedFile, DiffLayout, HunkId},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionState {
    pub(crate) hunk_id: HunkId,
    pub(crate) row_in_hunk: usize,
    pub(crate) scroll_offset: usize,
    pub(crate) collapsed_hunks: Vec<HunkId>,
}

pub(crate) fn first_row(
    files: &[ChangedFile],
    tree: &[crate::model::FileTreeRow],
    lookup: &crate::app::files::FileLookup,
    selected: Option<usize>,
) -> Option<usize> {
    selected
        .and_then(|row| tree.get(row))
        .and_then(|row| lookup.file_indices.get(&row.path))
        .and_then(|index| files.get(*index))
        .or_else(|| {
            selected
                .and_then(|row| tree.get(row))
                .and_then(|row| files.iter().find(|file| file.path == row.path))
        })
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
        if hunk.id == position.hunk_id {
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
