use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result};
use arboard::Clipboard;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::{Position, Rect},
    widgets::{ListState, TableState},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    git,
    model::{
        ChangeMode, ChangedFile, Commit, DiffLayout, DiffRowKind, DiffView, FileTreeRow, Worktree,
    },
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Focus {
    #[default]
    Worktrees,
    Files,
    Diff,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PanelLayout {
    #[default]
    Columns,
    SidebarLeft,
    SidebarTop,
}

impl PanelLayout {
    fn toggle(self) -> Self {
        match self {
            Self::Columns => Self::SidebarLeft,
            Self::SidebarLeft => Self::SidebarTop,
            Self::SidebarTop => Self::Columns,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteConfirmation {
    pub path: PathBuf,
    pub branch: String,
    pub prune_only: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusKind {
    Info,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusMessage {
    pub kind: StatusKind,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchState {
    pub focus: Focus,
    pub query: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WorktreePanel {
    #[default]
    Worktrees,
    History,
}

impl Focus {
    fn left(self, first_panel_visible: bool) -> Self {
        if !first_panel_visible {
            return match self {
                Self::Diff => Self::Files,
                Self::Files | Self::Worktrees => Self::Files,
            };
        }
        match self {
            Self::Worktrees => Self::Worktrees,
            Self::Files => Self::Worktrees,
            Self::Diff => Self::Files,
        }
    }

    fn right(self, first_panel_visible: bool) -> Self {
        if !first_panel_visible {
            return match self {
                Self::Files | Self::Worktrees => Self::Diff,
                Self::Diff => Self::Diff,
            };
        }
        match self {
            Self::Worktrees => Self::Files,
            Self::Files => Self::Diff,
            Self::Diff => Self::Diff,
        }
    }
}

#[cfg(test)]
mod focus_tests {
    use super::Focus;

    #[test]
    fn focus_navigation_handles_two_and_three_panel_layouts() {
        assert_eq!(Focus::Diff.left(false), Focus::Files);
        assert_eq!(Focus::Files.left(false), Focus::Files);
        assert_eq!(Focus::Files.right(false), Focus::Diff);

        assert_eq!(Focus::Diff.left(true), Focus::Files);
        assert_eq!(Focus::Files.left(true), Focus::Worktrees);
        assert_eq!(Focus::Worktrees.right(true), Focus::Files);
        assert_eq!(Focus::Files.right(true), Focus::Diff);
    }
}

pub struct App {
    pub directory: PathBuf,
    pub base: String,
    pub mode: ChangeMode,
    pub diff_view: DiffView,
    pub diff_layout: DiffLayout,
    pub line_wrap: bool,
    pub expanded: bool,
    pub initial_layout_applied: bool,
    pub panel_layout: PanelLayout,
    pub focus: Focus,
    pub worktrees: Vec<Worktree>,
    pub files: Vec<ChangedFile>,
    pub file_tree: Vec<FileTreeRow>,
    pub worktree_state: ListState,
    pub history_state: ListState,
    pub file_state: ListState,
    pub diff_state: TableState,
    pub show_help: bool,
    pub delete_confirmation: Option<DeleteConfirmation>,
    pub status: Option<StatusMessage>,
    pub worktree_filter: String,
    pub file_filter: String,
    pub search: Option<SearchState>,
    pub worktree_panel: WorktreePanel,
    pub commits: Vec<Commit>,
    pub history_base_commit: Option<String>,
    pub selected_commit: Option<usize>,
    pub history_preferred_file: Option<PathBuf>,
}

impl App {
    pub fn load(directory: PathBuf, base: String) -> Result<Self> {
        let worktrees = git::discover_worktrees(&directory)?;
        let selected_worktree = worktrees
            .iter()
            .position(|worktree| worktree.is_current)
            .unwrap_or(0);
        let files = git::load_changes(
            &worktrees[selected_worktree].path,
            ChangeMode::Uncommitted,
            DiffView::Hunks,
            &base,
        )
        .with_context(|| {
            format!(
                "failed to inspect {}",
                worktrees[selected_worktree].path.display()
            )
        })?;
        let has_linked_worktrees = worktrees.iter().any(|worktree| !worktree.is_main);
        let commits = if has_linked_worktrees {
            Vec::new()
        } else {
            git::commit_history(&worktrees[selected_worktree].path).with_context(|| {
                format!(
                    "failed to load history for {}",
                    worktrees[selected_worktree].path.display()
                )
            })?
        };
        let history_base_commit = if has_linked_worktrees {
            None
        } else {
            git::history_base_commit(&worktrees[selected_worktree].path, &base).ok()
        };

        let mut worktree_state = ListState::default();
        worktree_state.select(Some(selected_worktree));
        let mut history_state = ListState::default();
        history_state.select(Some(0));
        let file_tree = build_file_tree(&files);
        let mut file_state = ListState::default();
        file_state.select(first_file_row(&file_tree));
        let mut diff_state = TableState::default();
        diff_state.select(first_diff_row(&files, &file_tree, file_state.selected()));

        let focus = Focus::Files;
        let history_preferred_file = if has_linked_worktrees {
            None
        } else {
            file_state
                .selected()
                .and_then(|row| file_tree.get(row))
                .and_then(|row| row.file_index)
                .and_then(|index| files.get(index))
                .map(|file| file.path.clone())
        };

        Ok(Self {
            directory,
            base,
            mode: ChangeMode::Uncommitted,
            diff_view: DiffView::Hunks,
            diff_layout: DiffLayout::Split,
            line_wrap: false,
            expanded: false,
            initial_layout_applied: false,
            panel_layout: PanelLayout::SidebarLeft,
            focus,
            worktrees,
            files,
            file_tree,
            worktree_state,
            history_state,
            file_state,
            diff_state,
            show_help: false,
            delete_confirmation: None,
            status: None,
            worktree_filter: String::new(),
            file_filter: String::new(),
            search: None,
            worktree_panel: if has_linked_worktrees {
                WorktreePanel::Worktrees
            } else {
                WorktreePanel::History
            },
            commits,
            history_base_commit,
            selected_commit: (!has_linked_worktrees).then_some(0),
            history_preferred_file,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.delete_confirmation.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_worktree_removal(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.delete_confirmation = None;
                    self.set_info("Worktree cleanup cancelled");
                }
                _ => {}
            }
            return false;
        }

        if self.show_help {
            match key.code {
                KeyCode::Char('q') => return true,
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter => self.show_help = false,
                _ => {}
            }
            return false;
        }

        if self.search.is_some() {
            self.handle_search_key(key);
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc if self.focus != Focus::Diff => self.clear_filter(self.focus),
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('d') if self.focus == Focus::Worktrees => self.request_worktree_removal(),
            KeyCode::Char('h') if matches!(self.focus, Focus::Worktrees | Focus::Files) => {
                self.toggle_history()
            }
            KeyCode::Tab => self.toggle_mode(),
            KeyCode::Char('v') => self.toggle_diff_view(),
            KeyCode::Char('s') => self.diff_layout = self.diff_layout.toggle(),
            KeyCode::Char('w') => self.line_wrap = !self.line_wrap,
            KeyCode::Char(' ') => self.expanded = !self.expanded,
            KeyCode::Char('t') => {
                self.panel_layout = self.panel_layout.toggle();
                self.expanded = false;
            }
            KeyCode::Char('/') => self.begin_search(),
            KeyCode::Left | KeyCode::Char('h') => self.focus_left(),
            KeyCode::Right => self.focus_right(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::PageUp if self.focus == Focus::Diff => self.move_diff_by(-10),
            KeyCode::PageDown if self.focus == Focus::Diff => self.move_diff_by(10),
            KeyCode::Home if self.focus == Focus::Diff => self.select_diff_row(0),
            KeyCode::End if self.focus == Focus::Diff => {
                let count = self.diff_row_count();
                if count > 0 {
                    self.select_diff_row(count - 1);
                }
            }
            KeyCode::Enter if self.focus == Focus::Diff => self.toggle_selected_hunk(),
            KeyCode::Char('c') if self.focus == Focus::Diff => self.copy_selected_location(),
            _ => {}
        }
        false
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent, areas: [Rect; 3]) {
        if self.modal_open() || self.search.is_some() {
            return;
        }
        let position = Position::new(mouse.column, mouse.row);
        let show_worktrees = self.has_linked_worktrees() || self.history_active();

        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some(focus) = mouse_focus(position, areas, show_worktrees) else {
                    return;
                };
                self.focus = focus;
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -3
                } else {
                    3
                };
                match focus {
                    Focus::Worktrees => self.move_worktree(delta),
                    Focus::Files => self.move_file(delta),
                    Focus::Diff => self.move_diff_by(delta),
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_mouse_click(position, areas, show_worktrees);
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, position: Position, areas: [Rect; 3], show_worktrees: bool) {
        if show_worktrees && areas[0].contains(position) {
            let history = self.history_active();
            self.focus = Focus::Worktrees;
            if let Some(index) = self.list_index(position, areas[0], history) {
                if history {
                    self.select_commit(index);
                } else {
                    self.select_worktree(index);
                }
            }
        } else if areas[1].contains(position) {
            self.focus = Focus::Files;
            if let Some(row) = self.list_index(position, areas[1], false) {
                self.select_file_row(row);
            }
        } else if areas[2].contains(position) {
            let diff_rows_visible = self.expanded || self.focus == Focus::Diff;
            self.focus = Focus::Diff;
            if diff_rows_visible && let Some(row) = self.diff_row_at_position(position, areas[2]) {
                self.select_diff_row(row);
            }
        }
    }

    fn diff_row_at_position(&self, position: Position, area: Rect) -> Option<usize> {
        let file = self.selected_file()?;
        let effective_layout = match file.status {
            crate::model::FileStatus::Added
            | crate::model::FileStatus::Deleted
            | crate::model::FileStatus::Untracked => DiffLayout::Unified,
            crate::model::FileStatus::Modified
            | crate::model::FileStatus::Renamed
            | crate::model::FileStatus::Conflicted => self.diff_layout,
        };
        diff_row_at_position(
            file,
            effective_layout,
            self.line_wrap,
            self.diff_state.offset(),
            position,
            area,
        )
    }

    fn list_index(&self, position: Position, area: Rect, history: bool) -> Option<usize> {
        let content_top = area.y.saturating_add(1);
        if position.y < content_top {
            return None;
        }
        let row = usize::from(position.y - content_top);
        let item_height = if history || self.focus != Focus::Worktrees {
            1
        } else {
            2
        };
        Some(
            row / item_height
                + if history {
                    self.history_state.offset()
                } else if self.focus == Focus::Worktrees {
                    self.worktree_state.offset()
                } else {
                    self.file_state.offset()
                },
        )
    }

    fn select_worktree(&mut self, visible_position: usize) {
        let visible = self.visible_worktree_indices();
        let Some(index) = visible.get(visible_position).copied() else {
            return;
        };
        if Some(index) == self.worktree_state.selected() {
            return;
        }
        self.worktree_state.select(Some(index));
        self.reload_files(None);
    }

    fn select_commit(&mut self, visible_position: usize) {
        if visible_position > self.commits.len() {
            return;
        }
        let next = Some(visible_position);
        if self.selected_commit == next {
            return;
        }
        self.selected_commit = next;
        self.history_state.select(next);
        if visible_position == 0 {
            let preferred = self.history_preferred_file.clone();
            self.reload_files(preferred.as_deref());
        } else {
            self.reload_selected_commit();
        }
    }

    fn select_file_row(&mut self, visible_position: usize) {
        let visible = self.visible_file_rows();
        let Some(row) = visible.get(visible_position).copied() else {
            return;
        };
        if self.file_tree[row].is_directory() || Some(row) == self.file_state.selected() {
            return;
        }
        self.file_state.select(Some(row));
        if self.history_active() {
            self.history_preferred_file = self.selected_file().map(|file| file.path.clone());
        }
        self.diff_state
            .select(first_diff_row(&self.files, &self.file_tree, Some(row)));
    }

    pub fn apply_initial_layout(&mut self, terminal_width: u16, threshold: u16) {
        if self.initial_layout_applied {
            return;
        }
        self.expanded = terminal_width < threshold;
        self.diff_layout = if terminal_width < threshold {
            DiffLayout::Unified
        } else {
            DiffLayout::Split
        };
        self.initial_layout_applied = true;
    }

    pub fn selected_worktree(&self) -> Option<&Worktree> {
        self.worktree_state
            .selected()
            .and_then(|index| self.worktrees.get(index))
    }

    pub fn has_linked_worktrees(&self) -> bool {
        self.worktrees.iter().any(|worktree| !worktree.is_main)
    }

    pub fn history_active(&self) -> bool {
        self.worktree_panel == WorktreePanel::History
    }

    pub fn history_commit_selected(&self) -> bool {
        self.history_active() && self.selected_commit.is_some_and(|index| index > 0)
    }

    pub fn search_query(&self, focus: Focus) -> &str {
        match focus {
            Focus::Worktrees => &self.worktree_filter,
            Focus::Files => &self.file_filter,
            Focus::Diff => "",
        }
    }

    pub fn visible_worktree_indices(&self) -> Vec<usize> {
        self.worktrees
            .iter()
            .enumerate()
            .filter(|(_, worktree)| {
                fuzzy_match(
                    self.search_query(Focus::Worktrees),
                    &format!(
                        "{} {} {}",
                        worktree.path.display(),
                        worktree.branch,
                        worktree.head
                    ),
                )
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn visible_file_rows(&self) -> Vec<usize> {
        filtered_file_tree(
            &self.file_tree,
            &self.files,
            self.search_query(Focus::Files),
        )
    }

    pub fn file_tree_label(&self, row_index: usize, visible_rows: &[usize]) -> String {
        file_tree_label(&self.file_tree, row_index, visible_rows)
    }

    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.file_state
            .selected()
            .and_then(|row| self.file_tree.get(row))
            .and_then(|row| row.file_index)
            .and_then(|index| self.files.get(index))
    }

    pub fn diff_row_count(&self) -> usize {
        self.selected_file()
            .map(|file| {
                file.hunks
                    .iter()
                    .map(|hunk| 1 + usize::from(!hunk.collapsed) * hunk.rows.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    pub fn selected_file_index(&self) -> Option<usize> {
        self.file_state
            .selected()
            .and_then(|row| self.file_tree.get(row))
            .and_then(|row| row.file_index)
    }

    pub fn selected_hunk_index(&self) -> Option<usize> {
        let file = self.selected_file()?;
        let row = self.diff_state.selected()?;
        hunk_at_row(file, row)
    }

    pub fn diff_search_match_position(&self) -> Option<(usize, usize)> {
        let search = self
            .search
            .as_ref()
            .filter(|search| search.focus == Focus::Diff)?;
        if search.query.is_empty() {
            return None;
        }

        let matches = self.diff_matches(&search.query);
        if matches.is_empty() {
            return Some((0, 0));
        }

        let current = self
            .diff_state
            .selected()
            .and_then(|selected| matches.iter().position(|row| *row == selected))
            .map_or(0, |position| position + 1);
        Some((current, matches.len()))
    }

    pub fn modal_open(&self) -> bool {
        self.show_help || self.delete_confirmation.is_some()
    }

    fn move_up(&mut self) {
        match self.focus {
            Focus::Worktrees => self.move_worktree(-1),
            Focus::Files => self.move_file(-1),
            Focus::Diff => self.move_diff_by(-1),
        }
    }

    fn move_down(&mut self) {
        match self.focus {
            Focus::Worktrees => self.move_worktree(1),
            Focus::Files => self.move_file(1),
            Focus::Diff => self.move_diff_by(1),
        }
    }

    fn move_worktree(&mut self, delta: isize) {
        if self.history_active() {
            self.move_commit(delta);
            return;
        }
        let visible = self.visible_worktree_indices();
        let current = self
            .worktree_state
            .selected()
            .and_then(|selected| visible.iter().position(|index| *index == selected));
        let Some(next_position) = moved_selection(current, visible.len(), delta) else {
            return;
        };
        let next = visible[next_position];
        if Some(next) == self.worktree_state.selected() {
            return;
        }
        self.worktree_state.select(Some(next));
        self.reload_files(None);
    }

    fn move_file(&mut self, delta: isize) {
        let visible = self.visible_file_rows();
        let next = moved_visible_file_selection(
            self.file_state.selected(),
            &visible,
            &self.file_tree,
            delta,
        );
        if next == self.file_state.selected() {
            return;
        }
        self.file_state.select(next);
        if self.history_active() {
            self.history_preferred_file = self.selected_file().map(|file| file.path.clone());
        }
        self.diff_state.select(first_diff_row(
            &self.files,
            &self.file_tree,
            self.file_state.selected(),
        ));
    }

    fn move_diff_by(&mut self, delta: isize) {
        let next = moved_selection(self.diff_state.selected(), self.diff_row_count(), delta);
        self.diff_state.select(next);
    }

    fn select_diff_row(&mut self, row: usize) {
        self.diff_state
            .select((row < self.diff_row_count()).then_some(row));
    }

    fn toggle_selected_hunk(&mut self) {
        let Some(file_index) = self.selected_file_index() else {
            return;
        };
        let Some(selected_row) = self.diff_state.selected() else {
            return;
        };
        let Some(hunk_index) = hunk_at_row(&self.files[file_index], selected_row) else {
            return;
        };

        let header_row = self.files[file_index]
            .hunks
            .iter()
            .take(hunk_index)
            .map(|hunk| 1 + usize::from(!hunk.collapsed) * hunk.rows.len())
            .sum();
        let hunk = &mut self.files[file_index].hunks[hunk_index];
        hunk.collapsed = !hunk.collapsed;
        self.diff_state.select(Some(header_row));
    }

    fn toggle_mode(&mut self) {
        self.mode = self.mode.toggle();
        if self.history_commit_selected() {
            self.reload_selected_commit();
        } else {
            self.reload_files(None);
        }
    }

    fn toggle_diff_view(&mut self) {
        let selected_file = self.selected_file().map(|file| file.path.clone());
        self.diff_view = self.diff_view.toggle();
        if self.history_commit_selected() {
            self.reload_selected_commit();
        } else {
            self.reload_files(selected_file.as_deref());
        }
    }

    pub fn refresh(&mut self) {
        let selected_worktree = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone());
        let selected_file = self.selected_file().map(|file| file.path.clone());
        let diff_position = self.diff_position();
        let history_wip_selected = self.history_active() && self.selected_commit == Some(0);
        let selected_commit_hash = self
            .selected_commit
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| self.commits.get(index))
            .map(|commit| commit.hash.clone());

        match git::discover_worktrees(&self.directory) {
            Ok(worktrees) => {
                self.worktrees = worktrees;
                if !self.has_linked_worktrees()
                    && !self.history_active()
                    && self.focus == Focus::Worktrees
                {
                    self.focus = Focus::Files;
                    self.search = None;
                }
                let selected = selected_worktree
                    .as_deref()
                    .and_then(|path| worktree_index(&self.worktrees, path))
                    .or_else(|| {
                        self.worktrees
                            .iter()
                            .position(|worktree| worktree.is_current)
                    })
                    .or((!self.worktrees.is_empty()).then_some(0));
                self.worktree_state.select(selected);
                if self.history_active() {
                    if let Some(worktree_path) = self
                        .selected_worktree()
                        .map(|worktree| worktree.path.clone())
                    {
                        match git::commit_history(&worktree_path) {
                            Ok(commits) => {
                                self.commits = commits;
                                self.history_base_commit =
                                    git::history_base_commit(&worktree_path, &self.base).ok();
                            }
                            Err(error) => {
                                self.set_error(format!("Refresh failed: {error:#}"));
                                return;
                            }
                        }
                    }
                    self.selected_commit = history_selection_after_refresh(
                        history_wip_selected,
                        selected_commit_hash.as_deref(),
                        &self.commits,
                    );
                    if self.selected_commit == Some(0) {
                        self.reload_files_with_position(
                            selected_file.as_deref(),
                            diff_position.as_ref(),
                        );
                    }
                } else {
                    self.reload_files_with_position(
                        selected_file.as_deref(),
                        diff_position.as_ref(),
                    );
                }
            }
            Err(error) => self.set_error(format!("Refresh failed: {error:#}")),
        }
    }

    fn request_worktree_removal(&mut self) {
        let Some(worktree) = self.selected_worktree() else {
            return;
        };
        if worktree.is_current {
            self.set_error("Cannot remove the currently opened worktree");
            return;
        }
        if worktree.is_main {
            self.set_error("Cannot remove the main worktree");
            return;
        }
        if let Some(reason) = &worktree.locked_reason {
            let detail = if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            };
            self.set_error(format!("Cannot remove a locked worktree{detail}"));
            return;
        }
        if worktree.dirty {
            self.set_error(
                "Cannot remove a dirty worktree. Commit, stash, or discard its changes first"
                    .to_string(),
            );
            return;
        }

        self.delete_confirmation = Some(DeleteConfirmation {
            path: worktree.path.clone(),
            branch: worktree.branch.clone(),
            prune_only: worktree.is_missing(),
        });
        self.status = None;
    }

    fn confirm_worktree_removal(&mut self) {
        let Some(confirmation) = self.delete_confirmation.take() else {
            return;
        };
        let Some(worktree) = self
            .worktrees
            .iter()
            .find(|worktree| worktree.path == confirmation.path)
            .cloned()
        else {
            self.set_error("The selected worktree no longer exists");
            return;
        };

        match git::remove_worktree(&self.directory, &worktree) {
            Ok(()) => {
                let operation = if confirmation.prune_only {
                    "Pruned stale worktree metadata"
                } else {
                    "Removed worktree"
                };
                self.refresh();
                if self.status.is_none() {
                    self.set_info(format!("{operation}: {}", confirmation.path.display()));
                }
            }
            Err(error) => self.set_error(format!("Worktree cleanup failed: {error:#}")),
        }
    }

    fn reload_files(&mut self, preferred_file: Option<&Path>) {
        self.reload_files_with_position(preferred_file, None);
    }

    fn reload_files_with_position(
        &mut self,
        preferred_file: Option<&Path>,
        diff_position: Option<&DiffPosition>,
    ) {
        let Some(worktree) = self.selected_worktree() else {
            self.files.clear();
            self.file_tree.clear();
            self.file_state.select(None);
            self.diff_state.select(None);
            return;
        };
        let path = worktree.path.clone();
        if worktree.is_missing() {
            let reason = worktree
                .prunable_reason
                .as_deref()
                .filter(|reason| !reason.is_empty())
                .unwrap_or("working directory does not exist")
                .to_string();
            self.files.clear();
            self.file_tree.clear();
            self.file_state.select(None);
            self.diff_state.select(None);
            self.set_error(format!(
                "Unavailable worktree: {reason}. Press d in the Worktrees pane to clean it up"
            ));
            return;
        }

        match git::load_changes(&path, self.mode, self.diff_view, &self.base) {
            Ok(files) => {
                self.files = files;
                self.file_tree = build_file_tree(&self.files);
                let selected = preferred_file
                    .and_then(|path| {
                        self.file_tree.iter().position(|row| {
                            row.file_index
                                .and_then(|index| self.files.get(index))
                                .is_some_and(|file| file.path == path)
                        })
                    })
                    .or_else(|| first_file_row(&self.file_tree));
                self.file_state.select(selected);
                self.restore_diff_position(diff_position);
                self.status = None;
            }
            Err(error) => {
                self.files.clear();
                self.file_tree.clear();
                self.file_state.select(None);
                self.diff_state.select(None);
                self.set_error(format!("Could not load changes: {error:#}"));
            }
        }
    }

    fn diff_position(&self) -> Option<DiffPosition> {
        let file = self.selected_file()?;
        let selected_row = self.diff_state.selected().unwrap_or(0);
        let (hunk_index, row_in_hunk) = diff_row_position(file, selected_row)?;
        Some(DiffPosition {
            hunk_header: file.hunks[hunk_index].header.clone(),
            row_in_hunk,
            scroll_offset: self.diff_state.offset(),
            collapsed_hunks: file
                .hunks
                .iter()
                .filter(|hunk| hunk.collapsed)
                .map(|hunk| hunk.header.clone())
                .collect(),
        })
    }

    fn restore_diff_position(&mut self, position: Option<&DiffPosition>) {
        let Some(file_index) = self.selected_file_index() else {
            self.diff_state.select(None);
            return;
        };
        if let Some(position) = position {
            for hunk in &mut self.files[file_index].hunks {
                hunk.collapsed = position.collapsed_hunks.contains(&hunk.header);
            }
            let selected = diff_row_for_position(&self.files[file_index], position).or_else(|| {
                first_diff_row(&self.files, &self.file_tree, self.file_state.selected())
            });
            self.diff_state.select(selected);
            let max_offset = self.diff_row_count().saturating_sub(1);
            *self.diff_state.offset_mut() = position.scroll_offset.min(max_offset);
        } else {
            self.diff_state.select(first_diff_row(
                &self.files,
                &self.file_tree,
                self.file_state.selected(),
            ));
            *self.diff_state.offset_mut() = 0;
        }
    }

    fn set_info(&mut self, message: impl Into<String>) {
        self.status = Some(StatusMessage {
            kind: StatusKind::Info,
            text: message.into(),
        });
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.status = Some(StatusMessage {
            kind: StatusKind::Error,
            text: message.into(),
        });
    }

    fn begin_search(&mut self) {
        self.search = Some(SearchState {
            focus: self.focus,
            query: self.search_query(self.focus).to_string(),
        });
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let Some(search) = self.search.clone() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                let focus = search.focus;
                self.set_filter(focus, String::new());
                self.search = None;
            }
            KeyCode::Enter if search.focus == Focus::Diff => self.search_diff(1),
            KeyCode::Enter => self.search = None,
            KeyCode::Left => self.switch_search_panel(false),
            KeyCode::Right => self.switch_search_panel(true),
            KeyCode::Backspace => {
                let mut query = search.query;
                query.pop();
                self.search = Some(SearchState {
                    focus: search.focus,
                    query: query.clone(),
                });
                self.set_filter(search.focus, query);
                if search.focus == Focus::Diff {
                    self.search_diff_query(
                        &self
                            .search
                            .as_ref()
                            .map_or(String::new(), |search| search.query.clone()),
                        -1,
                    );
                }
            }
            KeyCode::Char(character) => {
                let mut query = search.query;
                query.push(character);
                self.search = Some(SearchState {
                    focus: search.focus,
                    query: query.clone(),
                });
                self.set_filter(search.focus, query);
                if search.focus == Focus::Diff {
                    self.search_diff_query(
                        &self
                            .search
                            .as_ref()
                            .map_or(String::new(), |search| search.query.clone()),
                        0,
                    );
                }
            }
            KeyCode::Up => self.search_diff(-1),
            KeyCode::Down => self.search_diff(1),
            _ => {}
        }
    }

    fn switch_search_panel(&mut self, right: bool) {
        let Some(current_focus) = self.search.as_ref().map(|search| search.focus) else {
            return;
        };
        let next_focus = match (current_focus, right) {
            (Focus::Worktrees, true) => Focus::Files,
            (Focus::Files, false) => Focus::Worktrees,
            (focus, _) => focus,
        };
        let query = self.search_query(next_focus).to_string();
        self.focus = next_focus;
        self.set_filter(next_focus, query);
        self.search = None;
    }

    fn set_filter(&mut self, focus: Focus, filter: String) {
        match focus {
            Focus::Worktrees => {
                self.worktree_filter = filter;
                let visible = self.visible_worktree_indices();
                let selected = self
                    .worktree_state
                    .selected()
                    .filter(|selected| visible.contains(selected))
                    .or_else(|| visible.first().copied());
                self.worktree_state.select(selected);
                self.reload_files(None);
            }
            Focus::Files => {
                self.file_filter = filter;
                let visible = self.visible_file_rows();
                let selected = self
                    .file_state
                    .selected()
                    .filter(|selected| visible.contains(selected))
                    .filter(|selected| self.file_tree[*selected].file_index.is_some())
                    .or_else(|| first_file_row_in(&self.file_tree, &visible));
                self.file_state.select(selected);
                self.diff_state
                    .select(first_diff_row(&self.files, &self.file_tree, selected));
            }
            Focus::Diff => self.search_diff(0),
        }
    }

    fn clear_filter(&mut self, focus: Focus) {
        if !self.search_query(focus).is_empty() {
            self.set_filter(focus, String::new());
        }
    }

    fn toggle_history(&mut self) {
        if self.history_active() {
            if !self.has_linked_worktrees() {
                return;
            }
            self.worktree_panel = WorktreePanel::Worktrees;
            self.selected_commit = None;
            let preferred = self.history_preferred_file.take();
            self.reload_files(preferred.as_deref());
            return;
        }
        let Some(worktree_path) = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
        else {
            return;
        };
        match git::commit_history(&worktree_path) {
            Ok(commits) => {
                self.history_preferred_file = self.selected_file().map(|file| file.path.clone());
                self.focus = Focus::Worktrees;
                self.commits = commits;
                self.history_base_commit =
                    git::history_base_commit(&worktree_path, &self.base).ok();
                self.selected_commit = Some(0);
                self.worktree_panel = WorktreePanel::History;
                let preferred = self.history_preferred_file.clone();
                self.reload_files(preferred.as_deref());
            }
            Err(error) => self.set_error(format!("Could not load commit history: {error:#}")),
        }
    }

    fn move_commit(&mut self, delta: isize) {
        let next = moved_selection(
            self.selected_commit.or(Some(0)),
            self.commits.len().saturating_add(1),
            delta,
        );
        if next == self.selected_commit {
            return;
        }
        self.selected_commit = next;
        self.history_state.select(next);
        if next == Some(0) {
            let preferred = self.history_preferred_file.clone();
            self.reload_files(preferred.as_deref());
        } else {
            self.reload_selected_commit();
        }
    }

    fn reload_selected_commit(&mut self) {
        let Some(commit_index) = self.selected_commit.and_then(|index| index.checked_sub(1)) else {
            let preferred = self.history_preferred_file.clone();
            self.reload_files(preferred.as_deref());
            return;
        };
        let Some(worktree) = self.selected_worktree() else {
            return;
        };
        match git::load_commit_changes(
            &worktree.path,
            &self.commits[commit_index],
            self.diff_view,
            self.mode,
            &self.base,
        ) {
            Ok(files) => {
                self.files = files;
                self.file_tree = build_file_tree(&self.files);
                let selected = self
                    .history_preferred_file
                    .as_deref()
                    .and_then(|path| {
                        self.file_tree.iter().position(|row| {
                            row.file_index
                                .and_then(|index| self.files.get(index))
                                .is_some_and(|file| file.path == path)
                        })
                    })
                    .or_else(|| first_file_row(&self.file_tree));
                self.file_state.select(selected);
                self.diff_state
                    .select(first_diff_row(&self.files, &self.file_tree, selected));
            }
            Err(error) => self.set_error(format!("Could not load commit: {error:#}")),
        }
    }

    fn search_diff(&mut self, direction: isize) {
        let Some(query) = self.search.as_ref().map(|search| search.query.clone()) else {
            return;
        };
        self.search_diff_query(&query, direction);
    }

    fn search_diff_query(&mut self, query: &str, direction: isize) {
        if query.is_empty() {
            return;
        }
        let matches = self.diff_matches(query);
        if matches.is_empty() {
            self.set_error(format!("No diff match for {query:?}"));
            return;
        }
        let current = self.diff_state.selected().unwrap_or(matches[0]);
        let next = if direction > 0 {
            matches
                .iter()
                .copied()
                .find(|row| *row > current)
                .unwrap_or(matches[0])
        } else if direction < 0 {
            matches
                .iter()
                .rev()
                .copied()
                .find(|row| *row < current)
                .unwrap_or(*matches.last().unwrap_or(&matches[0]))
        } else {
            matches[0]
        };
        self.diff_state.select(Some(next));
    }

    fn diff_matches(&self, query: &str) -> Vec<usize> {
        let Some(file) = self.selected_file() else {
            return Vec::new();
        };
        let query = query.to_lowercase();
        let mut matches = Vec::new();
        let mut row_index = 0;
        for hunk in &file.hunks {
            if hunk.header.to_lowercase().contains(&query) {
                matches.push(row_index);
            }
            row_index += 1;
            if !hunk.collapsed {
                for row in &hunk.rows {
                    if row
                        .old_text
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&query)
                        || row
                            .new_text
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&query)
                    {
                        matches.push(row_index);
                    }
                    row_index += 1;
                }
            }
        }
        matches
    }

    fn focus_left(&mut self) {
        self.focus = self
            .focus
            .left(self.has_linked_worktrees() || self.history_active());
    }

    fn focus_right(&mut self) {
        self.focus = self
            .focus
            .right(self.has_linked_worktrees() || self.history_active());
    }

    fn copy_selected_location(&mut self) {
        let Some(location) = self.selected_location() else {
            self.set_error("No source line is selected");
            return;
        };
        match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(location.clone())) {
            Ok(()) => self.set_info(format!("Copied {location}")),
            Err(error) => self.set_error(format!("Could not copy {location}: {error}")),
        }
    }

    fn selected_location(&self) -> Option<String> {
        let file = self.selected_file()?;
        let worktree = self.selected_worktree()?;
        let selected_row = self.diff_state.selected()?;
        let row = diff_row_at(file, selected_row)?;
        let line = row.new_number.or(row.old_number)?;
        Some(format!(
            "{}:{line}",
            worktree.path.join(&file.path).display()
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiffPosition {
    hunk_header: String,
    row_in_hunk: usize,
    scroll_offset: usize,
    collapsed_hunks: Vec<String>,
}

fn moved_selection(selected: Option<usize>, length: usize, delta: isize) -> Option<usize> {
    if length == 0 {
        return None;
    }
    let current = selected.unwrap_or(0);
    Some(current.saturating_add_signed(delta).min(length - 1))
}

fn worktree_index(worktrees: &[Worktree], path: &Path) -> Option<usize> {
    worktrees.iter().position(|worktree| worktree.path == path)
}

fn mouse_focus(position: Position, areas: [Rect; 3], show_worktrees: bool) -> Option<Focus> {
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

fn diff_row_at_position(
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

    let width = diff_content_width(area, layout);
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
            let height = diff_row_height(row, layout, line_wrap, width);
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

fn diff_row_height(
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

fn diff_content_width(area: Rect, layout: DiffLayout) -> usize {
    let available = area.width.saturating_sub(2 + 10 + 3 + 1);
    match layout {
        DiffLayout::Split => usize::from(available / 2).max(1),
        DiffLayout::Unified => usize::from(available).max(1),
    }
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    let mut lines = 0;
    for line in text.split('\n') {
        let mut current_width = 0;
        let mut line_count = 1;
        for character in line.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if current_width > 0 && current_width + character_width > width {
                line_count += 1;
                current_width = 0;
            }
            current_width += character_width;
        }
        lines += line_count;
    }
    lines.max(1)
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

fn history_selection_after_refresh(
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

fn first_diff_row(
    files: &[ChangedFile],
    tree: &[FileTreeRow],
    selected: Option<usize>,
) -> Option<usize> {
    selected
        .and_then(|row| tree.get(row))
        .and_then(|row| row.file_index)
        .and_then(|index| files.get(index))
        .filter(|file| !file.hunks.is_empty())
        .map(|_| 0)
}

fn first_file_row(tree: &[FileTreeRow]) -> Option<usize> {
    tree.iter().position(|row| !row.is_directory())
}

fn moved_visible_file_selection(
    selected: Option<usize>,
    visible: &[usize],
    tree: &[FileTreeRow],
    delta: isize,
) -> Option<usize> {
    let file_positions: Vec<usize> = visible
        .iter()
        .enumerate()
        .filter_map(|(position, row)| (!tree[*row].is_directory()).then_some(position))
        .collect();
    let current = selected
        .and_then(|selected| {
            file_positions
                .iter()
                .position(|position| visible[*position] == selected)
        })
        .or_else(|| (!file_positions.is_empty()).then_some(0));
    let next = moved_selection(current, file_positions.len(), delta)?;
    Some(visible[file_positions[next]])
}

fn first_file_row_in(tree: &[FileTreeRow], visible: &[usize]) -> Option<usize> {
    visible
        .iter()
        .copied()
        .find(|index| tree[*index].file_index.is_some())
}

fn fuzzy_match(query: &str, candidate: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut candidate = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|query_character| {
            candidate.any(|candidate_character| candidate_character == query_character)
        })
}

#[derive(Default)]
struct FileTreeDirectory {
    children: BTreeMap<OsString, FileTreeNode>,
}

enum FileTreeNode {
    Directory(FileTreeDirectory),
    File(usize),
}

fn build_file_tree(files: &[ChangedFile]) -> Vec<FileTreeRow> {
    let mut root = FileTreeDirectory::default();
    for (file_index, file) in files.iter().enumerate() {
        let components: Vec<OsString> = file
            .path
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_os_string()),
                _ => None,
            })
            .collect();
        insert_file_node(&mut root, &components, file_index);
    }

    let mut rows = Vec::new();
    flatten_file_tree(&root, Path::new(""), "", &mut rows);
    rows
}

fn insert_file_node(directory: &mut FileTreeDirectory, components: &[OsString], file_index: usize) {
    let Some((name, remainder)) = components.split_first() else {
        return;
    };
    if remainder.is_empty() {
        directory
            .children
            .insert(name.clone(), FileTreeNode::File(file_index));
        return;
    }

    let node = directory
        .children
        .entry(name.clone())
        .or_insert_with(|| FileTreeNode::Directory(FileTreeDirectory::default()));
    if let FileTreeNode::Directory(child) = node {
        insert_file_node(child, remainder, file_index);
    }
}

fn flatten_file_tree(
    directory: &FileTreeDirectory,
    path_prefix: &Path,
    label_prefix: &str,
    rows: &mut Vec<FileTreeRow>,
) {
    let child_count = directory.children.len();
    for (position, (name, node)) in directory.children.iter().enumerate() {
        let is_last = position + 1 == child_count;
        let connector = if is_last { "└── " } else { "├── " };
        let path = path_prefix.join(name);
        rows.push(FileTreeRow {
            label: format!("{label_prefix}{connector}{}", name.to_string_lossy()),
            path: path.clone(),
            file_index: match node {
                FileTreeNode::Directory(_) => None,
                FileTreeNode::File(index) => Some(*index),
            },
        });

        if let FileTreeNode::Directory(child) = node {
            let continuation = if is_last { "    " } else { "│   " };
            flatten_file_tree(child, &path, &format!("{label_prefix}{continuation}"), rows);
        }
    }
}

fn filtered_file_tree(tree: &[FileTreeRow], files: &[ChangedFile], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..tree.len()).collect();
    }

    let matching_files: Vec<usize> = tree
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.file_index.is_some_and(|file_index| {
                files
                    .get(file_index)
                    .is_some_and(|file| fuzzy_match(query, &file.path.display().to_string()))
            })
        })
        .map(|(row_index, _)| row_index)
        .collect();

    tree.iter()
        .enumerate()
        .filter(|(row_index, row)| {
            row.file_index
                .is_some_and(|_| matching_files.contains(row_index))
                || row.file_index.is_none()
                    && matching_files
                        .iter()
                        .any(|file_index| tree[*file_index].path.starts_with(&row.path))
        })
        .map(|(row_index, _)| row_index)
        .collect()
}

fn file_tree_label(tree: &[FileTreeRow], row_index: usize, visible_rows: &[usize]) -> String {
    let path = &tree[row_index].path;
    let depth = path.components().count();
    let mut label = String::new();
    for ancestor_depth in 1..depth {
        let ancestor =
            path.components()
                .take(ancestor_depth)
                .fold(PathBuf::new(), |mut path, component| {
                    path.push(component.as_os_str());
                    path
                });
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

fn hunk_at_row(file: &ChangedFile, row: usize) -> Option<usize> {
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

fn diff_row_at(file: &ChangedFile, row: usize) -> Option<&crate::model::DiffRow> {
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

fn diff_row_position(file: &ChangedFile, row: usize) -> Option<(usize, usize)> {
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

fn diff_row_for_position(file: &ChangedFile, position: &DiffPosition) -> Option<usize> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffHunk, DiffRow, DiffRowKind, FileStatus, HunkKind};
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    #[test]
    fn selection_stops_at_list_boundaries() {
        assert_eq!(moved_selection(Some(0), 3, -1), Some(0));
        assert_eq!(moved_selection(Some(0), 3, 1), Some(1));
        assert_eq!(moved_selection(Some(2), 3, 1), Some(2));
        assert_eq!(moved_selection(None, 0, 1), None);
    }

    #[test]
    fn fuzzy_matching_is_case_insensitive_subsequence_matching() {
        assert!(fuzzy_match("gm", "git-navigator-demo"));
        assert!(fuzzy_match("NAV", "git-navigator-demo"));
        assert!(!fuzzy_match("zz", "git-navigator-demo"));
        assert!(fuzzy_match("", "anything"));
    }

    #[test]
    fn slash_starts_live_search_for_worktrees_and_escape_clears_it() {
        let mut app = test_app();
        app.handle_key(key(KeyCode::Char('/')));
        assert_eq!(
            app.search,
            Some(SearchState {
                focus: Focus::Worktrees,
                query: String::new(),
            })
        );

        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.worktree_filter, "n");
        assert_eq!(
            app.search.as_ref().map(|search| search.query.as_str()),
            Some("n")
        );

        app.handle_key(key(KeyCode::Esc));
        assert!(app.search.is_none());
        assert!(app.worktree_filter.is_empty());
    }

    #[test]
    fn escape_clears_an_applied_search_without_reopening_search_mode() {
        let mut app = test_app();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('n')));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.search.is_none());
        assert_eq!(app.worktree_filter, "n");

        app.handle_key(key(KeyCode::Esc));
        assert!(app.worktree_filter.is_empty());
    }

    #[test]
    fn selected_location_prefers_new_line_and_falls_back_to_old_line() {
        let mut app = test_app();
        app.files = vec![ChangedFile {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            hunks: vec![DiffHunk {
                header: "@@".into(),
                kind: HunkKind::Unstaged,
                collapsed: false,
                rows: vec![
                    DiffRow {
                        old_number: Some(3),
                        new_number: Some(4),
                        old_text: Some("old".into()),
                        new_text: Some("new".into()),
                        kind: DiffRowKind::Modified,
                    },
                    DiffRow {
                        old_number: Some(5),
                        new_number: None,
                        old_text: Some("deleted".into()),
                        new_text: None,
                        kind: DiffRowKind::Deleted,
                    },
                ],
            }],
            binary: false,
        }];
        app.file_tree = vec![FileTreeRow {
            label: "└── main.rs".into(),
            path: PathBuf::from("src/main.rs"),
            file_index: Some(0),
        }];
        app.file_state.select(Some(0));
        app.diff_state.select(Some(1));
        app.worktrees = vec![Worktree {
            path: PathBuf::from("/repo/worktree"),
            branch: "main".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: true,
            is_main: true,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        }];
        app.worktree_state.select(Some(0));
        assert_eq!(
            app.selected_location(),
            Some("/repo/worktree/src/main.rs:4".into())
        );
        app.diff_state.select(Some(2));
        assert_eq!(
            app.selected_location(),
            Some("/repo/worktree/src/main.rs:5".into())
        );
    }

    #[test]
    fn slash_starts_contains_search_on_the_diff_panel() {
        let mut app = test_app();
        app.focus = Focus::Diff;
        app.handle_key(key(KeyCode::Char('/')));
        assert_eq!(
            app.search.as_ref().map(|search| search.focus),
            Some(Focus::Diff)
        );
    }

    #[test]
    fn diff_search_jumps_to_matching_rows_and_wraps() {
        let mut app = test_app();
        app.files = vec![ChangedFile {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 0,
            hunks: vec![DiffHunk {
                header: "@@".into(),
                kind: HunkKind::Unstaged,
                collapsed: false,
                rows: vec![
                    DiffRow {
                        old_number: Some(1),
                        new_number: Some(1),
                        old_text: Some("alpha".into()),
                        new_text: Some("alpha".into()),
                        kind: DiffRowKind::Context,
                    },
                    DiffRow {
                        old_number: Some(2),
                        new_number: Some(2),
                        old_text: Some("beta".into()),
                        new_text: Some("target one".into()),
                        kind: DiffRowKind::Modified,
                    },
                    DiffRow {
                        old_number: Some(3),
                        new_number: Some(3),
                        old_text: Some("target two".into()),
                        new_text: Some("gamma".into()),
                        kind: DiffRowKind::Modified,
                    },
                ],
            }],
            binary: false,
        }];
        app.file_tree = vec![FileTreeRow {
            label: "└── main.rs".into(),
            path: PathBuf::from("src/main.rs"),
            file_index: Some(0),
        }];
        app.file_state.select(Some(0));
        app.diff_state.select(Some(0));
        app.focus = Focus::Diff;
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('e')));
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.diff_state.selected(), Some(2));
        assert_eq!(app.diff_search_match_position(), Some((1, 2)));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.diff_state.selected(), Some(3));
        assert_eq!(app.diff_search_match_position(), Some((2, 2)));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.diff_state.selected(), Some(2));

        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.diff_state.selected(), Some(3));
    }

    #[test]
    fn search_arrow_keys_switch_panels_and_navigate_without_enter() {
        let mut app = test_app();
        app.worktrees = vec![
            Worktree {
                path: PathBuf::from("/repo/one"),
                branch: "one".into(),
                head: "11111111".into(),
                dirty: false,
                is_current: true,
                is_main: true,
                available: true,
                prunable_reason: None,
                locked_reason: None,
            },
            Worktree {
                path: PathBuf::from("/repo/two"),
                branch: "two".into(),
                head: "22222222".into(),
                dirty: false,
                is_current: false,
                is_main: false,
                available: true,
                prunable_reason: None,
                locked_reason: None,
            },
        ];
        app.files = vec![ChangedFile::empty(
            PathBuf::from("src/main.rs"),
            FileStatus::Modified,
        )];
        app.file_tree = vec![FileTreeRow {
            label: "└── main.rs".into(),
            path: PathBuf::from("src/main.rs"),
            file_index: Some(0),
        }];
        app.worktree_state.select(Some(0));
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.focus, Focus::Files);
        assert!(app.search.is_none());

        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('k')));
        app.handle_key(key(KeyCode::Char('l')));
        assert_eq!(
            app.search.as_ref().map(|search| search.query.as_str()),
            Some("hjkl")
        );
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.focus, Focus::Worktrees);
        assert!(app.search.is_none());

        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.worktree_state.selected(), Some(1));
    }

    #[test]
    fn filtered_file_tree_keeps_matching_ancestors() {
        let files = vec![
            ChangedFile::empty(PathBuf::from("src/git/parser.rs"), FileStatus::Modified),
            ChangedFile::empty(PathBuf::from("src/main.rs"), FileStatus::Modified),
            ChangedFile::empty(PathBuf::from("tests/test.rs"), FileStatus::Modified),
        ];
        let tree = build_file_tree(&files);
        let visible = filtered_file_tree(&tree, &files, "parser");
        let labels: Vec<String> = visible
            .iter()
            .map(|index| file_tree_label(&tree, *index, &visible))
            .collect();
        assert_eq!(
            labels,
            vec!["└── src", "    └── git", "        └── parser.rs"]
        );

        let all_rows = (0..tree.len()).collect::<Vec<_>>();
        assert_eq!(
            tree.iter()
                .map(|row| file_tree_label(
                    &tree,
                    tree.iter()
                        .position(|candidate| candidate.path == row.path)
                        .unwrap(),
                    &all_rows
                ))
                .collect::<Vec<_>>(),
            vec![
                "├── src",
                "│   ├── git",
                "│   │   └── parser.rs",
                "│   └── main.rs",
                "└── tests",
                "    └── test.rs",
            ]
        );
    }

    #[test]
    fn visible_file_navigation_skips_filtered_directories() {
        let tree = vec![
            FileTreeRow {
                label: "src".into(),
                path: PathBuf::from("src"),
                file_index: None,
            },
            FileTreeRow {
                label: "src/main.rs".into(),
                path: PathBuf::from("src/main.rs"),
                file_index: Some(0),
            },
            FileTreeRow {
                label: "tests/test.rs".into(),
                path: PathBuf::from("tests/test.rs"),
                file_index: Some(1),
            },
        ];
        let visible = vec![0, 1, 2];
        assert_eq!(
            moved_visible_file_selection(None, &visible, &tree, 1),
            Some(2)
        );
        assert_eq!(
            moved_visible_file_selection(Some(1), &visible, &tree, 1),
            Some(2)
        );
    }

    #[test]
    fn maps_visible_rows_to_hunks() {
        let mut file =
            ChangedFile::empty(PathBuf::from("file"), crate::model::FileStatus::Modified);
        file.hunks = vec![
            DiffHunk {
                header: "one".into(),
                kind: HunkKind::Staged,
                rows: vec![],
                collapsed: false,
            },
            DiffHunk {
                header: "two".into(),
                kind: HunkKind::Unstaged,
                rows: vec![],
                collapsed: false,
            },
        ];
        assert_eq!(hunk_at_row(&file, 0), Some(0));
        assert_eq!(hunk_at_row(&file, 1), Some(1));
        assert_eq!(hunk_at_row(&file, 2), None);
    }

    #[test]
    fn builds_a_sorted_folder_tree() {
        let files = vec![
            ChangedFile::empty(
                PathBuf::from("src/git/parser.rs"),
                crate::model::FileStatus::Modified,
            ),
            ChangedFile::empty(
                PathBuf::from("AGENTS.md"),
                crate::model::FileStatus::Modified,
            ),
            ChangedFile::empty(
                PathBuf::from("src/main.rs"),
                crate::model::FileStatus::Modified,
            ),
        ];

        let tree = build_file_tree(&files);
        assert_eq!(tree[0].label, "├── AGENTS.md");
        assert_eq!(tree[1].label, "└── src");
        assert!(tree[1].is_directory());
        assert_eq!(tree[2].label, "    ├── git");
        assert_eq!(tree[3].label, "    │   └── parser.rs");
        assert_eq!(tree[4].label, "    └── main.rs");
    }

    #[test]
    fn file_navigation_skips_directories() {
        let tree = vec![
            FileTreeRow {
                label: "src".into(),
                path: PathBuf::from("src"),
                file_index: None,
            },
            FileTreeRow {
                label: "src/a.rs".into(),
                path: PathBuf::from("src/a.rs"),
                file_index: Some(0),
            },
            FileTreeRow {
                label: "tests".into(),
                path: PathBuf::from("tests"),
                file_index: None,
            },
            FileTreeRow {
                label: "tests/a.rs".into(),
                path: PathBuf::from("tests/a.rs"),
                file_index: Some(1),
            },
        ];

        assert_eq!(first_file_row(&tree), Some(1));
        assert_eq!(
            moved_visible_file_selection(Some(1), &[0, 1, 2, 3], &tree, 1),
            Some(3)
        );
        assert_eq!(
            moved_visible_file_selection(Some(3), &[0, 1, 2, 3], &tree, -1),
            Some(1)
        );
        assert_eq!(
            moved_visible_file_selection(Some(1), &[0, 1, 2, 3], &tree, -1),
            Some(1)
        );
        assert_eq!(
            moved_visible_file_selection(Some(3), &[0, 1, 2, 3], &tree, 1),
            Some(3)
        );
    }

    #[test]
    fn missing_worktrees_are_identified_as_unavailable() {
        let worktree = Worktree {
            path: PathBuf::from("/missing"),
            branch: "feature".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: false,
            is_main: false,
            available: false,
            prunable_reason: Some("gitdir file points to non-existent location".into()),
            locked_reason: None,
        };
        assert!(worktree.is_missing());
    }

    #[test]
    fn restores_diff_position_and_collapsed_hunks() {
        let mut file = ChangedFile::empty(
            PathBuf::from("src/main.rs"),
            crate::model::FileStatus::Modified,
        );
        file.hunks = vec![
            DiffHunk {
                header: "@@ -1,2 +1,2 @@".into(),
                kind: HunkKind::Unstaged,
                rows: vec![
                    crate::model::DiffRow {
                        old_number: Some(1),
                        new_number: Some(1),
                        old_text: Some("one".into()),
                        new_text: Some("one".into()),
                        kind: crate::model::DiffRowKind::Context,
                    },
                    crate::model::DiffRow {
                        old_number: Some(2),
                        new_number: Some(2),
                        old_text: Some("old".into()),
                        new_text: Some("new".into()),
                        kind: crate::model::DiffRowKind::Modified,
                    },
                ],
                collapsed: true,
            },
            DiffHunk {
                header: "@@ -10 +10 @@".into(),
                kind: HunkKind::Unstaged,
                rows: vec![crate::model::DiffRow {
                    old_number: Some(10),
                    new_number: Some(10),
                    old_text: Some("before".into()),
                    new_text: Some("after".into()),
                    kind: crate::model::DiffRowKind::Modified,
                }],
                collapsed: false,
            },
        ];
        let position = DiffPosition {
            hunk_header: "@@ -10 +10 @@".into(),
            row_in_hunk: 1,
            scroll_offset: 1,
            collapsed_hunks: vec!["@@ -1,2 +1,2 @@".into()],
        };

        for hunk in &mut file.hunks {
            hunk.collapsed = position.collapsed_hunks.contains(&hunk.header);
        }
        assert_eq!(diff_row_for_position(&file, &position), Some(2));
        assert!(file.hunks[0].collapsed);
        assert!(!file.hunks[1].collapsed);
    }

    #[test]
    fn expanded_mode_follows_horizontal_focus_navigation() {
        let mut app = test_app();
        app.worktrees.push(Worktree {
            path: PathBuf::from("/repo/agent"),
            branch: "agent".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: false,
            is_main: false,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        });
        app.worktree_state.select(Some(0));
        assert!(!app.expanded);

        app.handle_key(key(KeyCode::Char(' ')));
        assert!(app.expanded);
        assert_eq!(app.focus, Focus::Worktrees);

        app.handle_key(key(KeyCode::Right));
        assert!(app.expanded);
        assert_eq!(app.focus, Focus::Files);

        app.handle_key(key(KeyCode::Right));
        assert!(app.expanded);
        assert_eq!(app.focus, Focus::Diff);

        app.handle_key(key(KeyCode::Char(' ')));
        assert!(!app.expanded);
        assert_eq!(app.focus, Focus::Diff);
    }

    #[test]
    fn panel_layout_cycles_through_all_layouts() {
        assert_eq!(PanelLayout::Columns.toggle(), PanelLayout::SidebarLeft);
        assert_eq!(PanelLayout::SidebarLeft.toggle(), PanelLayout::SidebarTop);
        assert_eq!(PanelLayout::SidebarTop.toggle(), PanelLayout::Columns);
    }

    #[test]
    fn panel_layout_cycle_includes_all_layouts_without_linked_worktrees() {
        let mut app = test_app();
        app.panel_layout = PanelLayout::Columns;
        app.worktree_panel = WorktreePanel::History;

        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.panel_layout, PanelLayout::SidebarLeft);

        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.panel_layout, PanelLayout::SidebarTop);

        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.panel_layout, PanelLayout::Columns);
    }

    #[test]
    fn history_mode_navigation_reaches_the_history_panel_without_linked_worktrees() {
        let mut app = test_app();
        app.worktree_panel = WorktreePanel::History;
        app.focus = Focus::Files;

        app.handle_key(key(KeyCode::Left));

        assert_eq!(app.focus, Focus::Worktrees);

        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn h_toggles_history_from_worktrees_and_files() {
        let mut app = test_app();
        app.worktrees.push(Worktree {
            path: PathBuf::from("/repo/agent"),
            branch: "agent".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: false,
            is_main: false,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        });
        let repository = tempfile::tempdir().expect("temporary repository should be created");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repository.path())
            .status()
            .expect("git init should run");
        std::fs::write(repository.path().join("file"), "content").expect("file should be written");
        std::process::Command::new("git")
            .args(["add", "file"])
            .current_dir(repository.path())
            .status()
            .expect("git add should run");
        std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ])
            .current_dir(repository.path())
            .status()
            .expect("git commit should run");
        app.worktree_state.select(Some(0));
        app.worktrees[0].path = repository.path().to_path_buf();
        app.handle_key(key(KeyCode::Char('h')));
        assert!(app.history_active());
        assert_eq!(app.focus, Focus::Worktrees);

        app.handle_key(key(KeyCode::Char('h')));
        assert!(!app.history_active());
        app.focus = Focus::Files;
        app.handle_key(key(KeyCode::Char('h')));
        assert!(app.history_active());
    }

    #[test]
    fn tab_changes_comparison_mode_while_history_is_active() {
        let mut app = test_app();
        app.worktree_panel = WorktreePanel::History;
        app.mode = ChangeMode::Uncommitted;

        app.handle_key(key(KeyCode::Tab));

        assert_eq!(app.mode, ChangeMode::Branch);
        assert!(app.history_active());
    }

    #[test]
    fn wip_is_the_first_history_entry_and_uses_live_changes() {
        let mut app = test_app();
        app.worktree_panel = WorktreePanel::History;
        app.commits = vec![Commit {
            hash: "1234567890abcdef".into(),
            short_hash: "12345678".into(),
            subject: "commit".into(),
        }];
        app.selected_commit = Some(1);

        app.move_commit(-1);

        assert_eq!(app.selected_commit, Some(0));
        assert!(!app.history_commit_selected());
    }

    #[test]
    fn history_commit_reload_prefers_the_remembered_file() {
        let mut app = test_app();
        app.history_preferred_file = Some(PathBuf::from("src/app.rs"));
        app.files = vec![
            ChangedFile::empty(PathBuf::from("README.md"), FileStatus::Modified),
            ChangedFile::empty(PathBuf::from("src/app.rs"), FileStatus::Modified),
        ];
        app.file_tree = build_file_tree(&app.files);
        let selected = app
            .history_preferred_file
            .as_deref()
            .and_then(|path| {
                app.file_tree.iter().position(|row| {
                    row.file_index
                        .and_then(|index| app.files.get(index))
                        .is_some_and(|file| file.path == path)
                })
            })
            .or_else(|| first_file_row(&app.file_tree));

        assert_eq!(
            selected.and_then(|row| app.file_tree[row].file_index),
            Some(1)
        );
    }

    #[test]
    fn history_refresh_preserves_wip_or_selected_commit_hash() {
        let commits = vec![
            Commit {
                hash: "new".into(),
                short_hash: "new".into(),
                subject: "new commit".into(),
            },
            Commit {
                hash: "selected".into(),
                short_hash: "selected".into(),
                subject: "selected commit".into(),
            },
        ];

        assert_eq!(
            history_selection_after_refresh(true, None, &commits),
            Some(0)
        );
        assert_eq!(
            history_selection_after_refresh(false, Some("selected"), &commits),
            Some(2)
        );
        assert_eq!(
            history_selection_after_refresh(false, Some("rewritten"), &commits),
            Some(0)
        );
    }

    #[test]
    fn mouse_scroll_moves_the_focused_diff() {
        let mut app = test_app();
        app.files = vec![ChangedFile {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            additions: 2,
            deletions: 2,
            hunks: vec![DiffHunk {
                header: "@@".into(),
                kind: HunkKind::Combined,
                collapsed: false,
                rows: vec![
                    DiffRow {
                        old_number: Some(1),
                        new_number: Some(1),
                        old_text: Some("one".into()),
                        new_text: Some("ONE".into()),
                        kind: DiffRowKind::Modified,
                    },
                    DiffRow {
                        old_number: Some(2),
                        new_number: Some(2),
                        old_text: Some("two".into()),
                        new_text: Some("TWO".into()),
                        kind: DiffRowKind::Modified,
                    },
                    DiffRow {
                        old_number: Some(3),
                        new_number: Some(3),
                        old_text: Some("three".into()),
                        new_text: Some("THREE".into()),
                        kind: DiffRowKind::Modified,
                    },
                ],
            }],
            binary: false,
        }];
        app.file_tree = vec![FileTreeRow {
            label: "└── main.rs".into(),
            path: PathBuf::from("src/main.rs"),
            file_index: Some(0),
        }];
        app.file_state.select(Some(0));
        app.diff_state.select(Some(0));
        app.focus = Focus::Diff;
        let areas = [
            Rect::new(0, 0, 10, 10),
            Rect::new(10, 0, 10, 10),
            Rect::new(20, 0, 40, 10),
        ];

        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 25,
                row: 5,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
            areas,
        );

        assert_eq!(app.focus, Focus::Diff);
        assert_eq!(app.diff_state.selected(), Some(3));
    }

    #[test]
    fn changing_layout_from_expanded_mode_minimizes_the_panel() {
        let mut app = test_app();
        app.worktrees.push(Worktree {
            path: PathBuf::from("/repo/agent"),
            branch: "agent".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: false,
            is_main: false,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        });
        app.expanded = true;
        app.panel_layout = PanelLayout::Columns;

        app.handle_key(key(KeyCode::Char('t')));

        assert!(!app.expanded);
        assert_eq!(app.panel_layout, PanelLayout::SidebarLeft);
        assert_eq!(app.focus, Focus::Worktrees);
    }

    #[test]
    fn narrow_initial_layout_starts_expanded_once() {
        let mut app = test_app();
        app.initial_layout_applied = false;

        app.apply_initial_layout(119, 120);
        assert!(app.expanded);
        assert_eq!(app.diff_layout, crate::model::DiffLayout::Unified);
        assert!(app.initial_layout_applied);

        app.expanded = false;
        app.apply_initial_layout(80, 120);
        assert!(!app.expanded, "manual minimize must remain authoritative");
    }

    #[test]
    fn wide_initial_layout_starts_minimized() {
        let mut app = test_app();
        app.initial_layout_applied = false;
        app.expanded = true;

        app.apply_initial_layout(120, 120);
        assert!(!app.expanded);
        assert_eq!(app.diff_layout, crate::model::DiffLayout::Split);
    }

    #[test]
    fn a_single_worktree_can_start_in_history_mode() {
        let mut app = test_app();
        app.worktrees.push(Worktree {
            path: PathBuf::from("/repo"),
            branch: "main".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: true,
            is_main: true,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        });

        let has_linked_worktrees = app.worktrees.iter().any(|worktree| !worktree.is_main);
        assert!(!has_linked_worktrees);
        app.worktree_panel = if has_linked_worktrees {
            WorktreePanel::Worktrees
        } else {
            WorktreePanel::History
        };
        app.selected_commit = (!has_linked_worktrees).then_some(0);

        assert!(app.history_active());
        assert_eq!(app.selected_commit, Some(0));
    }

    fn test_app() -> App {
        App {
            directory: PathBuf::from("/repo"),
            base: "main".into(),
            mode: ChangeMode::Uncommitted,
            diff_view: crate::model::DiffView::Hunks,
            diff_layout: crate::model::DiffLayout::Split,
            line_wrap: false,
            expanded: false,
            initial_layout_applied: true,
            panel_layout: PanelLayout::Columns,
            focus: Focus::Worktrees,
            worktrees: vec![],
            files: vec![],
            file_tree: vec![],
            worktree_state: ListState::default(),
            history_state: ListState::default(),
            file_state: ListState::default(),
            diff_state: TableState::default(),
            show_help: false,
            delete_confirmation: None,
            status: None,
            worktree_filter: String::new(),
            file_filter: String::new(),
            search: None,
            worktree_panel: WorktreePanel::Worktrees,
            commits: Vec::new(),
            history_base_commit: None,
            selected_commit: None,
            history_preferred_file: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }
}
