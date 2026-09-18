use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    app::{App, Focus},
    model::{ChangeMode, ChangedFile, FileStatus, FileTreeRow},
};

pub(crate) fn render(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    visible: &[usize],
    selected: Option<usize>,
) {
    let items: Vec<ListItem> = visible
        .iter()
        .map(|index| {
            let tree_row = app
                .changes
                .file_tree_row(*index)
                .expect("file tree row should exist");
            if tree_row.is_directory() {
                return ListItem::new(Line::styled(
                    tree_label(app.changes.file_tree(), *index, visible),
                    Style::new().fg(Color::Cyan).bold(),
                ));
            }
            let Some(file) = app.changes.file_for_tree_row(*index) else {
                panic!("file tree lookup missing file {}", tree_row.path.display());
            };

            let style = file_style(file, app.changes.mode);
            ListItem::new(Line::from(vec![
                Span::styled(tree_label(app.changes.file_tree(), *index, visible), style),
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
    let title = super::file_title(app, visible.len());
    let list = List::new(items)
        .block(super::pane_block(&title, app.view.focus == Focus::Files))
        .highlight_style(super::SELECTED)
        .highlight_symbol("› ");
    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
    *app.changes.file_state.offset_mut() = list_state.offset();
}

pub(crate) fn tree_label(tree: &[FileTreeRow], row_index: usize, visible_rows: &[usize]) -> String {
    let path = &tree[row_index].path;
    let depth = path.components().count();
    let mut label = String::new();
    for ancestor_depth in 1..depth {
        let ancestor = path.components().take(ancestor_depth).fold(
            std::path::PathBuf::new(),
            |mut path, component| {
                path.push(component.as_os_str());
                path
            },
        );
        let has_later_sibling = visible_rows.iter().any(|index| {
            tree[*index].path.parent() == ancestor.parent()
                && tree[*index].path != ancestor
                && *index
                    > tree
                        .iter()
                        .position(|row| row.path == ancestor)
                        .unwrap_or(0)
        });
        label.push_str(if has_later_sibling { "│   " } else { "    " });
    }
    let has_later_sibling = visible_rows.iter().any(|index| {
        *index > row_index
            && tree[*index].path.parent() == path.parent()
            && tree[*index].path != *path
    });
    label.push_str(if has_later_sibling {
        "├── "
    } else {
        "└── "
    });
    label.push_str(&path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    ));
    label
}

pub(crate) fn file_style(file: &ChangedFile, mode: ChangeMode) -> Style {
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
