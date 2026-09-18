mod diff;
mod files;
pub(crate) mod history;
mod input;
mod navigation;
mod search;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    git,
    model::{ChangeMode, ChangedFile, Commit, DiffLayout, DiffView, FileTreeRow, Worktree},
};
use anyhow::{Context, Result};
use ratatui::widgets::{ListState, TableState};

#[cfg(test)]
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind},
    layout::{Position, Rect},
};

use self::{
    diff::{
        PositionState as DiffPosition, first_row as first_diff_row, hunk_at_row,
        row_at as diff_row_at, row_at_position as diff_row_at_position,
        row_for_position as diff_row_for_position, row_position as diff_row_position,
    },
    files::{
        build_tree as build_file_tree, filtered_tree as filtered_file_tree, first_file_row,
        first_file_row_in, moved_visible_file_selection, tree_label as file_tree_label,
    },
    history::{
        list_index as history_list_index,
        selection_after_refresh as history_selection_after_refresh,
    },
    navigation::{mouse_focus, moved_selection, worktree_index},
    search::{diff_matches, fuzzy_match},
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
    pub history_range_commits: HashSet<String>,
    pub local_base_hash: Option<String>,
    pub remote_base_hash: Option<String>,
    pub branch_tips: HashMap<String, Vec<String>>,
    pub history_head_hash: Option<String>,
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
        let (local_base_hash, remote_base_hash) = if has_linked_worktrees {
            (None, None)
        } else {
            git::base_tip_hashes(&worktrees[selected_worktree].path, &base)
        };
        let branch_tips = git::branch_tips(&worktrees[selected_worktree].path)?;
        let history_head_hash = git::head_hash(&worktrees[selected_worktree].path).ok();
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
            history_range_commits: HashSet::new(),
            local_base_hash,
            remote_base_hash,
            branch_tips,
            history_head_hash,
            selected_commit: (!has_linked_worktrees).then_some(0),
            history_preferred_file,
        })
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
        self.history_state
            .select(crate::ui::history_visual_index(self, next));
        if let Err(error) = self.update_history_range() {
            self.set_error(format!("Could not update history range: {error:#}"));
            return;
        }
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

    pub fn prepare_render(&mut self, history: bool, selected_commit: Option<&str>) -> Result<()> {
        if let Some(hash) = selected_commit.filter(|_| !history) {
            anyhow::bail!("--commit requires --history when rendering: {hash}");
        }

        if history {
            let worktree_path = self
                .selected_worktree()
                .map(|worktree| worktree.path.clone())
                .context("no worktree is selected")?;
            self.commits = git::commit_history(&worktree_path)?;
            self.branch_tips = git::branch_tips(&worktree_path)?;
            self.history_head_hash = git::head_hash(&worktree_path).ok();
            (self.local_base_hash, self.remote_base_hash) =
                git::base_tip_hashes(&worktree_path, &self.base);
            self.worktree_panel = WorktreePanel::History;
            self.focus = Focus::Worktrees;
            self.selected_commit = match selected_commit {
                Some(hash) => Some(
                    self.commits
                        .iter()
                        .position(|commit| commit.hash == hash || commit.short_hash == hash)
                        .map(|index| index + 1)
                        .with_context(|| {
                            format!("commit '{hash}' was not found in rendered history")
                        })?,
                ),
                None => Some(0),
            };
            self.history_state
                .select(crate::ui::history_visual_index(self, self.selected_commit));
            self.update_history_range()?;
            if self.history_commit_selected() {
                self.reload_selected_commit();
            } else {
                self.reload_files(None);
            }
        } else {
            self.reload_files(None);
        }

        if let Some(status) = self.status.take()
            && status.kind == StatusKind::Error
        {
            anyhow::bail!("{}", status.text);
        }
        Ok(())
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

        let matches = diff_matches(self.selected_file(), &search.query);
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
        if let Err(error) = self.update_history_range() {
            self.set_error(format!("Could not update history range: {error:#}"));
        }
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
                                self.branch_tips =
                                    git::branch_tips(&worktree_path).unwrap_or_default();
                                self.history_head_hash = git::head_hash(&worktree_path).ok();
                                let (local_base_hash, remote_base_hash) =
                                    git::base_tip_hashes(&worktree_path, &self.base);
                                self.local_base_hash = local_base_hash;
                                self.remote_base_hash = remote_base_hash;
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
                    if let Err(error) = self.update_history_range() {
                        self.set_error(format!("Could not update history range: {error:#}"));
                        return;
                    }
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
                self.branch_tips = git::branch_tips(&worktree_path).unwrap_or_default();
                self.history_head_hash = git::head_hash(&worktree_path).ok();
                let (local_base_hash, remote_base_hash) =
                    git::base_tip_hashes(&worktree_path, &self.base);
                self.local_base_hash = local_base_hash;
                self.remote_base_hash = remote_base_hash;
                self.selected_commit = Some(0);
                self.worktree_panel = WorktreePanel::History;
                if let Err(error) = self.update_history_range() {
                    self.set_error(format!("Could not update history range: {error:#}"));
                    return;
                }
                let preferred = self.history_preferred_file.clone();
                self.reload_files(preferred.as_deref());
            }
            Err(error) => self.set_error(format!("Could not load commit history: {error:#}")),
        }
    }

    fn move_commit(&mut self, delta: isize) {
        let selectable = history::selectable_indices(self);
        let current = self
            .selected_commit
            .and_then(|selected| selectable.iter().position(|index| *index == selected));
        let Some(next_position) = moved_selection(current, selectable.len(), delta) else {
            return;
        };
        let next = selectable[next_position];
        if Some(next) == self.selected_commit {
            return;
        }
        self.selected_commit = Some(next);
        self.history_state
            .select(crate::ui::history_visual_index(self, Some(next)));
        if let Err(error) = self.update_history_range() {
            self.set_error(format!("Could not update history range: {error:#}"));
            return;
        }
        if next == 0 {
            let preferred = self.history_preferred_file.clone();
            self.reload_files(preferred.as_deref());
        } else {
            self.reload_selected_commit();
        }
    }

    fn reload_selected_commit(&mut self) {
        if let Err(error) = self.load_selected_commit_files() {
            self.set_error(format!("Could not load commit: {error:#}"));
        }
    }

    fn update_history_range(&mut self) -> Result<()> {
        self.history_range_commits.clear();
        if self.mode != ChangeMode::Branch || !self.history_active() {
            return Ok(());
        }

        let worktree_path = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
            .context("no worktree is selected")?;
        let target = match self.selected_commit {
            Some(0) => "HEAD".to_string(),
            Some(index) => self
                .commits
                .get(index - 1)
                .map(|commit| commit.hash.clone())
                .context("selected history commit is not available")?,
            None => return Ok(()),
        };
        self.history_range_commits =
            git::history_range_commits(&worktree_path, &target, &self.base)?;
        Ok(())
    }

    fn load_selected_commit_files(&mut self) -> Result<()> {
        let Some(commit_index) = self.selected_commit.and_then(|index| index.checked_sub(1)) else {
            anyhow::bail!("no historical commit is selected");
        };
        let worktree_path = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
            .context("no worktree is selected")?;
        let files = git::load_commit_changes(
            &worktree_path,
            &self.commits[commit_index],
            self.diff_view,
            self.mode,
            &self.base,
        )?;
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
        Ok(())
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
        let matches = diff_matches(self.selected_file(), query);
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
            parents: vec![],
            short_hash: "12345678".into(),
            subject: "commit".into(),
            graph: vec!["●".into()],
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
                parents: vec![],
                short_hash: "new".into(),
                subject: "new commit".into(),
                graph: vec!["●".into()],
            },
            Commit {
                hash: "selected".into(),
                parents: vec![],
                short_hash: "selected".into(),
                subject: "selected commit".into(),
                graph: vec!["●".into()],
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
    fn history_clicks_skip_graph_continuation_rows() {
        let mut app = test_app();
        app.worktree_panel = WorktreePanel::History;
        app.commits = vec![
            Commit {
                hash: "first".into(),
                parents: vec![],
                short_hash: "first".into(),
                subject: "first".into(),
                graph: vec!["●".into()],
            },
            Commit {
                hash: "second".into(),
                parents: vec![],
                short_hash: "second".into(),
                subject: "second".into(),
                graph: vec!["│ ●".into(), "│╱".into()],
            },
        ];
        app.history_state.select(Some(0));

        let area = Rect::new(0, 0, 50, 10);
        assert_eq!(app.list_index(Position::new(1, 1), area, true), Some(0));
        assert_eq!(app.list_index(Position::new(1, 2), area, true), Some(1));
        assert_eq!(app.list_index(Position::new(1, 3), area, true), None);
        assert_eq!(app.list_index(Position::new(1, 4), area, true), Some(2));
        assert_eq!(app.list_index(Position::new(1, 6), area, true), None);
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
            history_range_commits: HashSet::new(),
            local_base_hash: None,
            remote_base_hash: None,
            branch_tips: HashMap::new(),
            history_head_hash: None,
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
