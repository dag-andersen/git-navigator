use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::model::{ChangedFile, DiffLayout, DiffRow, DiffRowKind, FileStatus};

const TAB_WIDTH: usize = 4;

pub(crate) fn effective_layout(file: &ChangedFile, preferred: DiffLayout) -> DiffLayout {
    match file.status {
        FileStatus::Added | FileStatus::Deleted | FileStatus::Untracked => DiffLayout::Unified,
        FileStatus::Modified | FileStatus::Renamed | FileStatus::Conflicted => preferred,
    }
}

pub(crate) fn content_width(area: Rect, layout: DiffLayout) -> usize {
    let available = area.width.saturating_sub(2 + 10 + 3 + 1);
    match layout {
        DiffLayout::Split => usize::from(available / 2).max(1),
        DiffLayout::Unified => usize::from(available).max(1),
    }
}

pub(crate) fn row_height(row: &DiffRow, layout: DiffLayout, line_wrap: bool, width: usize) -> u16 {
    let text_width = match layout {
        DiffLayout::Split => width,
        DiffLayout::Unified => width.saturating_sub(2).max(1),
    };
    let line_count = |text: Option<&str>, width: usize| {
        if !line_wrap {
            return 1;
        }
        wrapped_line_count(&expand_tabs(text.unwrap_or_default()), width) as u16
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

pub(crate) fn display_text(text: Option<&str>, line_wrap: bool, width: usize) -> String {
    let expanded = expand_tabs(text.unwrap_or_default());
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

fn expand_tabs(text: &str) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut column = 0;
    for character in text.chars() {
        if character == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn wraps_long_diff_lines_at_display_width() {
        assert_eq!(display_text(Some("abcdefghij"), true, 4), "abcd\nefgh\nij");
        assert_eq!(display_text(Some("abcdefghij"), false, 4), "abcdefghij");
        assert_eq!(display_text(Some("ab界cd"), true, 4), "ab界\ncd");
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

    #[test]
    fn one_sided_files_always_use_unified_layout() {
        for status in [
            FileStatus::Added,
            FileStatus::Deleted,
            FileStatus::Untracked,
        ] {
            let file = ChangedFile::empty(PathBuf::from("file"), status);
            assert_eq!(
                effective_layout(&file, DiffLayout::Split),
                DiffLayout::Unified
            );
        }

        let modified = ChangedFile::empty(PathBuf::from("file"), FileStatus::Modified);
        assert_eq!(
            effective_layout(&modified, DiffLayout::Split),
            DiffLayout::Split
        );
    }
}
