use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    app::{App, Focus},
    model::{ChangeMode, ChangedFile, FileStatus},
};

pub(crate) fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let visible = app.visible_file_rows();
    let selected = app
        .file_state
        .selected()
        .and_then(|selected| visible.iter().position(|index| *index == selected));
    let items: Vec<ListItem> = visible
        .iter()
        .map(|index| {
            let tree_row = &app.file_tree[*index];
            let Some(file) = tree_row
                .file_index
                .and_then(|file_index| app.files.get(file_index))
            else {
                return ListItem::new(Line::styled(
                    app.file_tree_label(*index, &visible),
                    Style::new().fg(Color::Cyan).bold(),
                ));
            };

            let style = file_style(file, app.mode);
            ListItem::new(Line::from(vec![
                Span::styled(app.file_tree_label(*index, &visible), style),
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
        .block(super::pane_block(&title, app.focus == Focus::Files))
        .highlight_style(super::SELECTED)
        .highlight_symbol("› ");
    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
    *app.file_state.offset_mut() = list_state.offset();
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
