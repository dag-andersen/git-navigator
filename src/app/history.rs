use crate::{app::App, model::Commit};

pub(crate) fn selection_after_refresh(
    wip_selected: bool,
    selected_hash: Option<&str>,
    commits: &[Commit],
) -> Option<usize> {
    if wip_selected {
        return Some(0);
    }
    selected_hash
        .and_then(|hash| commits.iter().position(|commit| commit.hash == hash))
        .map(|index| index + 1)
        .or(Some(0))
}

pub(crate) fn list_index(app: &App, visible_row: usize) -> Option<usize> {
    let target_row = visible_row + app.history_state.offset();
    if target_row == 0 {
        return Some(0);
    }

    let mut row = 1;
    for (index, commit) in app.commits.iter().enumerate() {
        row += commit.graph.len().saturating_sub(1);
        if target_row == row {
            return Some(index + 1);
        }
        row += 1;
    }
    None
}
