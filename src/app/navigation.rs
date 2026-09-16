use std::path::Path;

use ratatui::layout::{Position, Rect};

use super::Focus;
use crate::model::Worktree;

pub(crate) fn moved_selection(
    selected: Option<usize>,
    length: usize,
    delta: isize,
) -> Option<usize> {
    if length == 0 {
        return None;
    }
    let current = selected.unwrap_or(0);
    Some(current.saturating_add_signed(delta).min(length - 1))
}

pub(crate) fn worktree_index(worktrees: &[Worktree], path: &Path) -> Option<usize> {
    worktrees.iter().position(|worktree| worktree.path == path)
}

pub(crate) fn mouse_focus(
    position: Position,
    areas: [Rect; 3],
    show_worktrees: bool,
) -> Option<Focus> {
    if show_worktrees && areas[0].contains(position) {
        Some(Focus::Worktrees)
    } else if areas[1].contains(position) {
        Some(Focus::Files)
    } else if areas[2].contains(position) {
        Some(Focus::Diff)
    } else {
        None
    }
}
