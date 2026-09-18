mod diff;
pub(crate) mod files;
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
    model::{
        ChangeMode, ChangedFile, Commit, DiffLayout, DiffRowKind, DiffView, FileTreeRow,
        HistorySelection, Worktree,
    },
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
        first_file_row_in, moved_visible_file_selection,
    },
    history::{
        list_index as history_list_index,
        selection_after_refresh as history_selection_after_refresh,
        visual_index as history_visual_index,
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

#[derive(Debug)]
pub struct RepositoryState {
    pub directory: PathBuf,
    pub base: String,
    pub worktrees: Vec<Worktree>,
    pub worktree_state: ListState,
}

#[derive(Debug)]
pub struct ChangeState {
    pub mode: ChangeMode,
    pub diff_view: DiffView,
    files: Vec<ChangedFile>,
    file_tree: Vec<FileTreeRow>,
    file_lookup: files::FileLookup,
    pub file_state: ListState,
    pub diff_state: TableState,
    selected_file_path: Option<PathBuf>,
}

impl ChangeState {
    pub(crate) fn new(mode: ChangeMode, diff_view: DiffView) -> Self {
        Self {
            mode,
            diff_view,
            files: Vec::new(),
            file_tree: Vec::new(),
            file_lookup: files::FileLookup::default(),
            file_state: ListState::default(),
            diff_state: TableState::default(),
            selected_file_path: None,
        }
    }

    pub(crate) fn install_files(&mut self, files: Vec<ChangedFile>) {
        self.files = files;
        self.file_tree = build_file_tree(&self.files);
        self.file_lookup = files::FileLookup::build(&self.files, &self.file_tree);
    }

    pub(crate) fn clear_files(&mut self) {
        self.install_files(Vec::new());
        self.selected_file_path = None;
        self.file_state.select(None);
        self.diff_state.select(None);
    }

    pub(crate) fn files(&self) -> &[ChangedFile] {
        &self.files
    }

    fn file_mut(&mut self, index: usize) -> Option<&mut ChangedFile> {
        self.files.get_mut(index)
    }

    pub(crate) fn file_tree(&self) -> &[FileTreeRow] {
        &self.file_tree
    }

    pub(crate) fn file_tree_row(&self, row: usize) -> Option<&FileTreeRow> {
        self.file_tree.get(row)
    }

    pub(crate) fn file_for_tree_row(&self, row: usize) -> Option<&ChangedFile> {
        self.file_tree_row(row)
            .and_then(|row| self.file_lookup.file_index(&row.path))
            .and_then(|index| self.files.get(index))
    }

    pub(crate) fn file_index(&self, path: &Path) -> Option<usize> {
        self.file_lookup.file_index(path)
    }

    pub(crate) fn tree_index(&self, path: &Path) -> Option<usize> {
        self.file_lookup.tree_index(path)
    }

    pub(crate) fn first_file_row(&self) -> Option<usize> {
        first_file_row(&self.file_tree)
    }

    pub(crate) fn first_file_row_in(&self, visible: &[usize]) -> Option<usize> {
        first_file_row_in(&self.file_tree, visible)
    }

    pub(crate) fn visible_file_rows(&self, query: &str) -> Vec<usize> {
        filtered_file_tree(&self.file_tree, &self.files, &self.file_lookup, query)
    }

    pub(crate) fn selected_file(&self) -> Option<&ChangedFile> {
        self.selected_file_path
            .as_deref()
            .and_then(|path| self.file_index(path))
            .and_then(|index| self.files.get(index))
    }

    pub(crate) fn selected_file_index(&self) -> Option<usize> {
        self.selected_file_path
            .as_deref()
            .and_then(|path| self.file_index(path))
    }

    pub(crate) fn select_file_path(&mut self, path: Option<PathBuf>) {
        self.selected_file_path = path;
        let row = self
            .selected_file_path
            .as_deref()
            .and_then(|path| self.tree_index(path));
        self.file_state.select(row);
    }
}

#[derive(Debug)]
pub struct HistoryState {
    pub list_state: ListState,
    pub worktree_panel: WorktreePanel,
    pub commits: Vec<Commit>,
    pub range_commits: HashSet<String>,
    pub local_base_hash: Option<String>,
    pub remote_base_hash: Option<String>,
    pub branch_tips: HashMap<String, Vec<String>>,
    pub head_hash: Option<String>,
    pub comparison_base: Option<String>,
    pub selection: Option<HistorySelection>,
    pub preferred_file: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ViewState {
    pub diff_layout: DiffLayout,
    pub line_wrap: bool,
    pub expanded: bool,
    pub initial_layout_applied: bool,
    pub panel_layout: PanelLayout,
    pub focus: Focus,
    pub show_help: bool,
    pub delete_confirmation: Option<DeleteConfirmation>,
    pub status: Option<StatusMessage>,
    pub worktree_filter: String,
    pub file_filter: String,
    pub search: Option<SearchState>,
    pub follow_changes: bool,
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
    pub repository: RepositoryState,
    pub changes: ChangeState,
    pub history: HistoryState,
    pub view: ViewState,
}

#[derive(Clone, Debug)]
struct RepositorySnapshot {
    worktree_path: Option<PathBuf>,
    file_path: Option<PathBuf>,
    diff_position: Option<DiffPosition>,
    history_selection: Option<HistorySelection>,
}

impl App {
    fn repository_snapshot(&self) -> RepositorySnapshot {
        RepositorySnapshot {
            worktree_path: self
                .selected_worktree()
                .map(|worktree| worktree.path.clone()),
            file_path: self.selected_file().map(|file| file.path.clone()),
            diff_position: self.diff_position(),
            history_selection: self.history.selection.clone(),
        }
    }

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
        let mut changes = ChangeState::new(ChangeMode::Uncommitted, DiffView::Hunks);
        changes.install_files(files);
        let mut file_state = ListState::default();
        file_state.select(changes.first_file_row());
        let mut diff_state = TableState::default();
        diff_state.select(first_diff_row(&changes, file_state.selected()));
        let focus = Focus::Files;
        let history_preferred_file = if has_linked_worktrees {
            None
        } else {
            file_state
                .selected()
                .and_then(|row| changes.file_for_tree_row(row))
                .map(|file| file.path.clone())
        };
        let selected_file_path = file_state
            .selected()
            .and_then(|row| changes.file_tree_row(row))
            .filter(|row| !row.is_directory())
            .map(|row| row.path.clone());
        changes.file_state = file_state;
        changes.diff_state = diff_state;
        changes.select_file_path(selected_file_path);

        Ok(Self {
            repository: RepositoryState {
                directory,
                base,
                worktrees,
                worktree_state,
            },
            changes,
            history: HistoryState {
                list_state: history_state,
                worktree_panel: if has_linked_worktrees {
                    WorktreePanel::Worktrees
                } else {
                    WorktreePanel::History
                },
                commits,
                range_commits: HashSet::new(),
                local_base_hash,
                remote_base_hash,
                branch_tips,
                head_hash: history_head_hash,
                comparison_base: None,
                selection: (!has_linked_worktrees).then_some(HistorySelection::Wip),
                preferred_file: history_preferred_file,
            },
            view: ViewState {
                diff_layout: DiffLayout::Split,
                line_wrap: false,
                expanded: false,
                initial_layout_applied: false,
                panel_layout: PanelLayout::SidebarLeft,
                focus,
                show_help: false,
                delete_confirmation: None,
                status: None,
                worktree_filter: String::new(),
                file_filter: String::new(),
                search: None,
                follow_changes: false,
            },
        })
    }

    fn select_worktree(&mut self, visible_position: usize) {
        let visible = self.visible_worktree_indices();
        let Some(index) = visible.get(visible_position).copied() else {
            return;
        };
        if Some(index) == self.repository.worktree_state.selected() {
            return;
        }
        self.repository.worktree_state.select(Some(index));
        self.reload_files(None);
    }

    fn select_commit(&mut self, visible_position: usize) {
        if visible_position > self.history.commits.len() {
            return;
        }
        let next = if visible_position == 0 {
            HistorySelection::Wip
        } else {
            let Some(commit) = self.history.commits.get(visible_position - 1) else {
                return;
            };
            HistorySelection::Commit {
                hash: commit.hash.clone(),
            }
        };
        let next = Some(next);
        if self.history.selection == next {
            return;
        }
        self.history.selection = next;
        self.history
            .list_state
            .select(history_visual_index(self, self.history.selection.as_ref()));
        if let Err(error) = self.update_history_range() {
            self.set_error(format!("Could not update history range: {error:#}"));
            return;
        }
        if matches!(self.history.selection, Some(HistorySelection::Wip)) {
            let preferred = self.history.preferred_file.clone();
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
        if self
            .changes
            .file_tree_row(row)
            .is_none_or(FileTreeRow::is_directory)
            || Some(row) == self.changes.file_state.selected()
        {
            return;
        }
        self.select_file_path(self.changes.file_tree_row(row).map(|row| row.path.clone()));
        if self.history_active() {
            self.history.preferred_file = self.selected_file().map(|file| file.path.clone());
        }
        self.changes
            .diff_state
            .select(first_diff_row(&self.changes, Some(row)));
    }

    pub fn apply_initial_layout(&mut self, terminal_width: u16, threshold: u16) {
        if self.view.initial_layout_applied {
            return;
        }
        self.view.expanded = terminal_width < threshold;
        self.view.diff_layout = if terminal_width < threshold {
            DiffLayout::Unified
        } else {
            DiffLayout::Split
        };
        self.view.initial_layout_applied = true;
    }

    pub fn selected_worktree(&self) -> Option<&Worktree> {
        self.repository
            .worktree_state
            .selected()
            .and_then(|index| self.repository.worktrees.get(index))
    }

    pub fn has_linked_worktrees(&self) -> bool {
        self.repository
            .worktrees
            .iter()
            .any(|worktree| !worktree.is_main)
    }

    pub fn history_active(&self) -> bool {
        self.history.worktree_panel == WorktreePanel::History
    }

    pub fn history_commit_selected(&self) -> bool {
        self.history_active()
            && matches!(
                self.history.selection,
                Some(HistorySelection::Commit { .. })
            )
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
            self.history.commits = git::commit_history(&worktree_path)?;
            self.history.branch_tips = git::branch_tips(&worktree_path)?;
            self.history.head_hash = git::head_hash(&worktree_path).ok();
            (self.history.local_base_hash, self.history.remote_base_hash) =
                git::base_tip_hashes(&worktree_path, &self.repository.base);
            self.history.worktree_panel = WorktreePanel::History;
            self.view.focus = Focus::Worktrees;
            self.history.selection = match selected_commit {
                Some(hash) => {
                    let commit = self
                        .history
                        .commits
                        .iter()
                        .find(|commit| commit.hash == hash || commit.short_hash == hash)
                        .with_context(|| {
                            format!("commit '{hash}' was not found in rendered history")
                        })?;
                    Some(HistorySelection::Commit {
                        hash: commit.hash.clone(),
                    })
                }
                None => Some(HistorySelection::Wip),
            };
            self.history
                .list_state
                .select(history_visual_index(self, self.history.selection.as_ref()));
            self.update_history_range()?;
            if self.history_commit_selected() {
                self.reload_selected_commit();
            } else {
                self.reload_files(None);
            }
        } else {
            self.reload_files(None);
        }

        if let Some(status) = self.view.status.take()
            && status.kind == StatusKind::Error
        {
            anyhow::bail!("{}", status.text);
        }
        Ok(())
    }

    pub fn search_query(&self, focus: Focus) -> &str {
        match focus {
            Focus::Worktrees => &self.view.worktree_filter,
            Focus::Files => &self.view.file_filter,
            Focus::Diff => "",
        }
    }

    pub fn visible_worktree_indices(&self) -> Vec<usize> {
        self.repository
            .worktrees
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
        self.visible_file_rows_with_lookup()
    }

    fn visible_file_rows_with_lookup(&self) -> Vec<usize> {
        self.changes
            .visible_file_rows(self.search_query(Focus::Files))
    }

    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.changes.selected_file()
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
        self.changes.selected_file_index()
    }

    fn select_file_path(&mut self, path: Option<PathBuf>) {
        self.changes.select_file_path(path);
    }

    pub fn selected_hunk_index(&self) -> Option<usize> {
        let file = self.selected_file()?;
        let row = self.changes.diff_state.selected()?;
        hunk_at_row(file, row)
    }

    pub fn diff_search_match_position(&self) -> Option<(usize, usize)> {
        let search = self
            .view
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
            .changes
            .diff_state
            .selected()
            .and_then(|selected| matches.iter().position(|row| *row == selected))
            .map_or(0, |position| position + 1);
        Some((current, matches.len()))
    }

    pub fn modal_open(&self) -> bool {
        self.view.show_help || self.view.delete_confirmation.is_some()
    }

    fn move_up(&mut self) {
        match self.view.focus {
            Focus::Worktrees => self.move_worktree(-1),
            Focus::Files => self.move_file(-1),
            Focus::Diff => self.move_diff_by(-1),
        }
    }

    fn move_down(&mut self) {
        match self.view.focus {
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
            .repository
            .worktree_state
            .selected()
            .and_then(|selected| visible.iter().position(|index| *index == selected));
        let Some(next_position) = moved_selection(current, visible.len(), delta) else {
            return;
        };
        let next = visible[next_position];
        if Some(next) == self.repository.worktree_state.selected() {
            return;
        }
        self.repository.worktree_state.select(Some(next));
        self.reload_files(None);
    }

    fn move_file(&mut self, delta: isize) {
        let visible = self.visible_file_rows();
        let next = moved_visible_file_selection(
            self.changes.file_state.selected(),
            &visible,
            self.changes.file_tree(),
            delta,
        );
        if next == self.changes.file_state.selected() {
            return;
        }
        let next_path = next
            .and_then(|row| self.changes.file_tree_row(row))
            .map(|row| row.path.clone());
        self.select_file_path(next_path);
        if self.history_active() {
            self.history.preferred_file = self.selected_file().map(|file| file.path.clone());
        }
        self.changes.diff_state.select(first_diff_row(
            &self.changes,
            self.changes.file_state.selected(),
        ));
    }

    fn move_diff_by(&mut self, delta: isize) {
        let next = moved_selection(
            self.changes.diff_state.selected(),
            self.diff_row_count(),
            delta,
        );
        self.changes.diff_state.select(next);
    }

    fn select_diff_row(&mut self, row: usize) {
        self.changes
            .diff_state
            .select((row < self.diff_row_count()).then_some(row));
    }

    fn toggle_selected_hunk(&mut self) {
        let Some(file_index) = self.selected_file_index() else {
            return;
        };
        let Some(selected_row) = self.changes.diff_state.selected() else {
            return;
        };
        let Some(hunk_index) = self
            .changes
            .files()
            .get(file_index)
            .and_then(|file| hunk_at_row(file, selected_row))
        else {
            return;
        };

        let header_row = self.changes.files()[file_index]
            .hunks
            .iter()
            .take(hunk_index)
            .map(|hunk| 1 + usize::from(!hunk.collapsed) * hunk.rows.len())
            .sum();
        let Some(hunk) = self
            .changes
            .file_mut(file_index)
            .and_then(|file| file.hunks.get_mut(hunk_index))
        else {
            return;
        };
        hunk.collapsed = !hunk.collapsed;
        self.changes.diff_state.select(Some(header_row));
    }

    fn toggle_mode(&mut self) {
        self.changes.mode = self.changes.mode.toggle();
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
        self.changes.diff_view = self.changes.diff_view.toggle();
        if self.history_commit_selected() {
            self.reload_selected_commit();
        } else {
            self.reload_files(selected_file.as_deref());
        }
    }

    pub fn refresh(&mut self) {
        self.refresh_with_paths(None);
    }

    pub fn refresh_following(&mut self, paths: &[PathBuf]) {
        self.refresh_with_paths(Some(paths));
    }

    fn refresh_with_paths(&mut self, changed_paths: Option<&[PathBuf]>) {
        let snapshot = self.repository_snapshot();

        match git::discover_worktrees(&self.repository.directory) {
            Ok(worktrees) => {
                self.repository.worktrees = worktrees;
                if !self.has_linked_worktrees()
                    && !self.history_active()
                    && self.view.focus == Focus::Worktrees
                {
                    self.view.focus = Focus::Files;
                    self.view.search = None;
                }
                let selected = snapshot
                    .worktree_path
                    .as_deref()
                    .and_then(|path| worktree_index(&self.repository.worktrees, path))
                    .or_else(|| {
                        self.repository
                            .worktrees
                            .iter()
                            .position(|worktree| worktree.is_current)
                    })
                    .or((!self.repository.worktrees.is_empty()).then_some(0));
                self.repository.worktree_state.select(selected);
                if self.history_active() {
                    if let Some(worktree_path) = self
                        .selected_worktree()
                        .map(|worktree| worktree.path.clone())
                    {
                        let branch_tips = git::branch_tips(&worktree_path).unwrap_or_default();
                        let history_head_hash = git::head_hash(&worktree_path).ok();
                        let history_changed = self.history.commits.is_empty()
                            || self.history.branch_tips != branch_tips
                            || self.history.head_hash != history_head_hash;
                        if history_changed {
                            match git::commit_history(&worktree_path) {
                                Ok(commits) => self.history.commits = commits,
                                Err(error) => {
                                    self.set_error(format!("Refresh failed: {error:#}"));
                                    return;
                                }
                            }
                        }
                        self.history.branch_tips = branch_tips;
                        self.history.head_hash = history_head_hash;
                        let (local_base_hash, remote_base_hash) =
                            git::base_tip_hashes(&worktree_path, &self.repository.base);
                        self.history.local_base_hash = local_base_hash;
                        self.history.remote_base_hash = remote_base_hash;
                    }
                    self.history.selection = history_selection_after_refresh(
                        snapshot.history_selection.as_ref(),
                        &self.history.commits,
                    );
                    if let Err(error) = self.update_history_range() {
                        self.set_error(format!("Could not update history range: {error:#}"));
                        return;
                    }
                    if self.history.selection == Some(HistorySelection::Wip) {
                        self.reload_files_with_position(
                            snapshot.file_path.as_deref(),
                            snapshot.diff_position.as_ref(),
                        );
                    }
                } else {
                    self.reload_files_with_position(
                        snapshot.file_path.as_deref(),
                        snapshot.diff_position.as_ref(),
                    );
                }
                if let Some(paths) = changed_paths {
                    self.follow_changed_paths(paths);
                }
            }
            Err(error) => self.set_error(format!("Refresh failed: {error:#}")),
        }
    }

    fn follow_changed_paths(&mut self, changed_paths: &[PathBuf]) {
        if self.history_commit_selected() {
            return;
        }
        let Some(worktree_path) = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
        else {
            return;
        };
        let selected_path = changed_paths.iter().rev().find_map(|path| {
            let relative = path.strip_prefix(&worktree_path).ok()?;
            self.changes
                .file_index(relative)
                .and_then(|index| self.changes.files().get(index))
                .map(|file| file.path.clone())
        });
        let Some(selected_path) = selected_path else {
            return;
        };
        self.select_file_path(Some(selected_path));
        self.select_latest_diff_row();
    }

    fn select_latest_diff_row(&mut self) {
        let Some(file) = self.selected_file() else {
            self.changes.diff_state.select(None);
            return;
        };
        let mut row = None;
        let mut offset = 0;
        for hunk in &file.hunks {
            row = Some(offset);
            for diff_row in &hunk.rows {
                offset += 1;
                if matches!(
                    diff_row.kind,
                    DiffRowKind::Added | DiffRowKind::Deleted | DiffRowKind::Modified
                ) {
                    row = Some(offset);
                }
            }
            offset += 1;
        }
        self.changes.diff_state.select(row);
        *self.changes.diff_state.offset_mut() = row.unwrap_or(0);
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

        self.view.delete_confirmation = Some(DeleteConfirmation {
            path: worktree.path.clone(),
            branch: worktree.branch.clone(),
            prune_only: worktree.is_missing(),
        });
        self.view.status = None;
    }

    fn confirm_worktree_removal(&mut self) {
        let Some(confirmation) = self.view.delete_confirmation.take() else {
            return;
        };
        let Some(worktree) = self
            .repository
            .worktrees
            .iter()
            .find(|worktree| worktree.path == confirmation.path)
            .cloned()
        else {
            self.set_error("The selected worktree no longer exists");
            return;
        };

        match git::remove_worktree(&self.repository.directory, &worktree) {
            Ok(()) => {
                let operation = if confirmation.prune_only {
                    "Pruned stale worktree metadata"
                } else {
                    "Removed worktree"
                };
                self.refresh();
                if self.view.status.is_none() {
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
            self.changes.clear_files();
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
            self.changes.clear_files();
            self.set_error(format!(
                "Unavailable worktree: {reason}. Press d in the Worktrees pane to clean it up"
            ));
            return;
        }

        match git::load_changes(
            &path,
            self.changes.mode,
            self.changes.diff_view,
            &self.repository.base,
        ) {
            Ok(files) => {
                self.changes.install_files(files);
                let selected_path = preferred_file
                    .filter(|path| self.changes.file_index(path).is_some())
                    .map(Path::to_path_buf)
                    .or_else(|| {
                        self.changes
                            .first_file_row()
                            .and_then(|row| self.changes.file_tree_row(row))
                            .map(|row| row.path.clone())
                    });
                self.select_file_path(selected_path);
                self.restore_diff_position(diff_position);
                self.view.status = None;
            }
            Err(error) => {
                self.changes.clear_files();
                self.set_error(format!("Could not load changes: {error:#}"));
            }
        }
    }

    fn diff_position(&self) -> Option<DiffPosition> {
        let file = self.selected_file()?;
        let selected_row = self.changes.diff_state.selected().unwrap_or(0);
        let (hunk_index, row_in_hunk) = diff_row_position(file, selected_row)?;
        Some(DiffPosition {
            hunk_id: file.hunks[hunk_index].id.clone(),
            row_in_hunk,
            scroll_offset: self.changes.diff_state.offset(),
            collapsed_hunks: file
                .hunks
                .iter()
                .filter(|hunk| hunk.collapsed)
                .map(|hunk| hunk.id.clone())
                .collect(),
        })
    }

    fn restore_diff_position(&mut self, position: Option<&DiffPosition>) {
        let Some(file_index) = self.selected_file_index() else {
            self.changes.diff_state.select(None);
            return;
        };
        if let Some(position) = position {
            for hunk in &mut self.changes.file_mut(file_index).unwrap().hunks {
                hunk.collapsed = position.collapsed_hunks.contains(&hunk.id);
            }
            let selected =
                diff_row_for_position(self.changes.files().get(file_index).unwrap(), position)
                    .or_else(|| first_diff_row(&self.changes, self.changes.file_state.selected()));
            self.changes.diff_state.select(selected);
            let max_offset = self.diff_row_count().saturating_sub(1);
            *self.changes.diff_state.offset_mut() = position.scroll_offset.min(max_offset);
        } else {
            self.changes.diff_state.select(first_diff_row(
                &self.changes,
                self.changes.file_state.selected(),
            ));
            *self.changes.diff_state.offset_mut() = 0;
        }
    }

    fn set_info(&mut self, message: impl Into<String>) {
        self.view.status = Some(StatusMessage {
            kind: StatusKind::Info,
            text: message.into(),
        });
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.view.status = Some(StatusMessage {
            kind: StatusKind::Error,
            text: message.into(),
        });
    }

    fn toggle_history(&mut self) {
        if self.history_active() {
            if !self.has_linked_worktrees() {
                return;
            }
            self.history.worktree_panel = WorktreePanel::Worktrees;
            self.history.selection = None;
            let preferred = self.history.preferred_file.take();
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
                self.history.preferred_file = self.selected_file().map(|file| file.path.clone());
                self.view.focus = Focus::Worktrees;
                self.history.commits = commits;
                self.history.branch_tips = git::branch_tips(&worktree_path).unwrap_or_default();
                self.history.head_hash = git::head_hash(&worktree_path).ok();
                let (local_base_hash, remote_base_hash) =
                    git::base_tip_hashes(&worktree_path, &self.repository.base);
                self.history.local_base_hash = local_base_hash;
                self.history.remote_base_hash = remote_base_hash;
                self.history.selection = Some(HistorySelection::Wip);
                self.history.worktree_panel = WorktreePanel::History;
                if let Err(error) = self.update_history_range() {
                    self.set_error(format!("Could not update history range: {error:#}"));
                    return;
                }
                let preferred = self.history.preferred_file.clone();
                self.reload_files(preferred.as_deref());
            }
            Err(error) => self.set_error(format!("Could not load commit history: {error:#}")),
        }
    }

    fn move_commit(&mut self, delta: isize) {
        let selectable = history::selectable_selections(self);
        let current = self.history.selection.as_ref().and_then(|selected| {
            selectable
                .iter()
                .position(|selection| selection == selected)
        });
        let Some(next_position) = moved_selection(current, selectable.len(), delta) else {
            return;
        };
        let next = selectable[next_position].clone();
        if Some(next.clone()) == self.history.selection {
            return;
        }
        self.history.selection = Some(next.clone());
        self.history
            .list_state
            .select(history_visual_index(self, self.history.selection.as_ref()));
        if let Err(error) = self.update_history_range() {
            self.set_error(format!("Could not update history range: {error:#}"));
            return;
        }
        if next == HistorySelection::Wip {
            let preferred = self.history.preferred_file.clone();
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
        self.history.range_commits.clear();
        self.history.comparison_base = None;
        if self.changes.mode != ChangeMode::Branch || !self.history_active() {
            return Ok(());
        }

        let worktree_path = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
            .context("no worktree is selected")?;
        let target = match &self.history.selection {
            Some(HistorySelection::Wip) => "HEAD".to_string(),
            Some(HistorySelection::Commit { hash }) => hash.clone(),
            None => return Ok(()),
        };
        let comparison_ref =
            if matches!(
                self.history.selection,
                Some(HistorySelection::Commit { .. })
            ) && git::is_ancestor(&worktree_path, &target, &self.repository.base)?
            {
                git::history_branch_ref(&worktree_path, &target, &self.repository.base)?
            } else {
                None
            };
        let comparison_base = git::history_comparison_base(
            &worktree_path,
            &target,
            &self.repository.base,
            comparison_ref.as_deref(),
        )?;
        self.history.range_commits =
            git::history_range_commits_from_base(&worktree_path, &target, &comparison_base)?;
        self.history.comparison_base = Some(comparison_base);
        Ok(())
    }

    fn load_selected_commit_files(&mut self) -> Result<()> {
        let Some(HistorySelection::Commit { hash }) = &self.history.selection else {
            anyhow::bail!("no historical commit is selected");
        };
        let worktree_path = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
            .context("no worktree is selected")?;
        let commit = self
            .history
            .commits
            .iter()
            .find(|commit| commit.hash == *hash)
            .context("selected history commit is not available")?;
        let files = git::load_commit_changes(
            &worktree_path,
            commit,
            self.changes.diff_view,
            self.changes.mode,
            &self.repository.base,
            self.history.comparison_base.as_deref(),
        )?;
        self.changes.install_files(files);
        let selected_path = self
            .history
            .preferred_file
            .as_deref()
            .filter(|path| self.changes.file_index(path).is_some())
            .map(Path::to_path_buf)
            .or_else(|| {
                self.changes
                    .first_file_row()
                    .and_then(|row| self.changes.file_tree_row(row))
                    .map(|row| row.path.clone())
            });
        self.select_file_path(selected_path);
        self.changes.diff_state.select(first_diff_row(
            &self.changes,
            self.changes.file_state.selected(),
        ));
        Ok(())
    }

    fn search_diff(&mut self, direction: isize) {
        let Some(query) = self.view.search.as_ref().map(|search| search.query.clone()) else {
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
        let current = self.changes.diff_state.selected().unwrap_or(matches[0]);
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
        self.changes.diff_state.select(Some(next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffHunk, DiffRow, DiffRowKind, FileStatus, FileTreeRowKind, HunkKind};
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
            app.view.search,
            Some(SearchState {
                focus: Focus::Worktrees,
                query: String::new(),
            })
        );

        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.view.worktree_filter, "n");
        assert_eq!(
            app.view.search.as_ref().map(|search| search.query.as_str()),
            Some("n")
        );

        app.handle_key(key(KeyCode::Esc));
        assert!(app.view.search.is_none());
        assert!(app.view.worktree_filter.is_empty());
    }

    #[test]
    fn escape_clears_an_applied_search_without_reopening_search_mode() {
        let mut app = test_app();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('n')));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.view.search.is_none());
        assert_eq!(app.view.worktree_filter, "n");

        app.handle_key(key(KeyCode::Esc));
        assert!(app.view.worktree_filter.is_empty());
    }

    #[test]
    fn selected_location_prefers_new_line_and_falls_back_to_old_line() {
        let mut app = test_app();
        app.changes.install_files(vec![ChangedFile {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            hunks: vec![DiffHunk {
                id: crate::model::HunkId::synthetic("test-hunk-1"),
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
        }]);
        app.changes
            .select_file_path(Some(PathBuf::from("src/main.rs")));
        app.changes.file_state.select(Some(0));
        app.changes.diff_state.select(Some(1));
        app.repository.worktrees = vec![Worktree {
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
        app.repository.worktree_state.select(Some(0));
        assert_eq!(
            app.selected_location(),
            Some("/repo/worktree/src/main.rs:4".into())
        );
        app.changes.diff_state.select(Some(2));
        assert_eq!(
            app.selected_location(),
            Some("/repo/worktree/src/main.rs:5".into())
        );
    }

    #[test]
    fn follow_toggle_selects_latest_changed_file_and_diff_row() {
        let mut app = test_app();
        app.repository.worktrees = vec![Worktree {
            path: PathBuf::from("/repo"),
            branch: "main".into(),
            head: "12345678".into(),
            dirty: true,
            is_current: true,
            is_main: true,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        }];
        app.repository.worktree_state.select(Some(0));
        let mut first = ChangedFile::empty(PathBuf::from("first.txt"), FileStatus::Modified);
        first.hunks.push(DiffHunk {
            id: crate::model::HunkId::synthetic("first"),
            header: "@@".into(),
            kind: HunkKind::Unstaged,
            collapsed: false,
            rows: vec![DiffRow {
                old_number: Some(1),
                new_number: Some(1),
                old_text: Some("old".into()),
                new_text: Some("new".into()),
                kind: DiffRowKind::Modified,
            }],
        });
        let mut second = ChangedFile::empty(PathBuf::from("src/second.txt"), FileStatus::Untracked);
        second.hunks.push(DiffHunk {
            id: crate::model::HunkId::synthetic("second"),
            header: "@@".into(),
            kind: HunkKind::Untracked,
            collapsed: false,
            rows: vec![DiffRow {
                old_number: None,
                new_number: Some(1),
                old_text: None,
                new_text: Some("new file".into()),
                kind: DiffRowKind::Added,
            }],
        });
        app.changes.install_files(vec![first, second]);
        app.select_file_path(Some(PathBuf::from("first.txt")));
        app.view.follow_changes = true;

        app.follow_changed_paths(&[PathBuf::from("/repo/src/second.txt")]);

        assert_eq!(
            app.selected_file().map(|file| file.path.as_path()),
            Some(Path::new("src/second.txt"))
        );
        assert_eq!(app.changes.diff_state.selected(), Some(1));
    }

    #[test]
    fn follow_toggle_is_off_by_default_and_can_be_changed_with_f() {
        let mut app = test_app();
        assert!(!app.view.follow_changes);
        app.handle_key(key(KeyCode::Char('f')));
        assert!(app.view.follow_changes);
        assert!(app.view.status.is_none());
        app.handle_key(key(KeyCode::Char('f')));
        assert!(!app.view.follow_changes);
        assert!(app.view.status.is_none());
    }

    #[test]
    fn slash_starts_contains_search_on_the_diff_panel() {
        let mut app = test_app();
        app.view.focus = Focus::Diff;
        app.handle_key(key(KeyCode::Char('/')));
        assert_eq!(
            app.view.search.as_ref().map(|search| search.focus),
            Some(Focus::Diff)
        );
    }

    #[test]
    fn diff_search_jumps_to_matching_rows_and_wraps() {
        let mut app = test_app();
        app.changes.install_files(vec![ChangedFile {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 0,
            hunks: vec![DiffHunk {
                id: crate::model::HunkId::synthetic("test-hunk-2"),
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
        }]);
        app.changes
            .select_file_path(Some(PathBuf::from("src/main.rs")));
        app.changes.file_state.select(Some(0));
        app.changes.diff_state.select(Some(0));
        app.view.focus = Focus::Diff;
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('e')));
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.changes.diff_state.selected(), Some(2));
        assert_eq!(app.diff_search_match_position(), Some((1, 2)));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.changes.diff_state.selected(), Some(3));
        assert_eq!(app.diff_search_match_position(), Some((2, 2)));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.changes.diff_state.selected(), Some(2));

        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.changes.diff_state.selected(), Some(3));
    }

    #[test]
    fn search_arrow_keys_switch_panels_and_navigate_without_enter() {
        let mut app = test_app();
        app.repository.worktrees = vec![
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
        app.changes.install_files(vec![ChangedFile::empty(
            PathBuf::from("src/main.rs"),
            FileStatus::Modified,
        )]);
        app.changes
            .select_file_path(Some(PathBuf::from("src/main.rs")));
        app.repository.worktree_state.select(Some(0));
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.view.focus, Focus::Files);
        assert!(app.view.search.is_none());

        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('k')));
        app.handle_key(key(KeyCode::Char('l')));
        assert_eq!(
            app.view.search.as_ref().map(|search| search.query.as_str()),
            Some("hjkl")
        );
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.view.focus, Focus::Worktrees);
        assert!(app.view.search.is_none());

        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.repository.worktree_state.selected(), Some(1));
    }

    #[test]
    fn filtered_file_tree_keeps_matching_ancestors() {
        let files = vec![
            ChangedFile::empty(PathBuf::from("src/git/parser.rs"), FileStatus::Modified),
            ChangedFile::empty(PathBuf::from("src/main.rs"), FileStatus::Modified),
            ChangedFile::empty(PathBuf::from("tests/test.rs"), FileStatus::Modified),
        ];
        let mut changes = ChangeState::new(ChangeMode::Uncommitted, DiffView::Hunks);
        changes.install_files(files);
        let visible = changes.visible_file_rows("parser");
        let labels: Vec<String> = visible
            .iter()
            .map(|index| crate::ui::files::tree_label(changes.file_tree(), *index, &visible))
            .collect();
        assert_eq!(
            labels,
            vec!["└── src", "    └── git", "        └── parser.rs"]
        );

        let all_rows = (0..changes.file_tree().len()).collect::<Vec<_>>();
        assert_eq!(
            changes
                .file_tree()
                .iter()
                .map(|row| crate::ui::files::tree_label(
                    changes.file_tree(),
                    changes
                        .file_tree()
                        .iter()
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
                path: PathBuf::from("src"),
                kind: FileTreeRowKind::Directory,
            },
            FileTreeRow {
                path: PathBuf::from("src/main.rs"),
                kind: FileTreeRowKind::File,
            },
            FileTreeRow {
                path: PathBuf::from("tests/test.rs"),
                kind: FileTreeRowKind::File,
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
    fn installing_and_clearing_files_rebuilds_lookup_and_resets_selection() {
        let mut app = test_app();
        let old_path = PathBuf::from("old.rs");
        let new_path = PathBuf::from("src/new.rs");

        app.changes.install_files(vec![ChangedFile::empty(
            old_path.clone(),
            FileStatus::Modified,
        )]);
        assert_eq!(app.changes.file_index(&old_path), Some(0));

        app.changes.install_files(vec![ChangedFile::empty(
            new_path.clone(),
            FileStatus::Modified,
        )]);
        assert_eq!(app.changes.file_index(&old_path), None);
        assert_eq!(app.changes.file_index(&new_path), Some(0));
        assert_eq!(app.changes.tree_index(&new_path), Some(1));

        app.changes.select_file_path(Some(new_path));
        app.changes.file_state.select(Some(1));
        app.changes.diff_state.select(Some(2));
        app.changes.clear_files();

        assert!(app.changes.files().is_empty());
        assert!(app.changes.file_tree().is_empty());
        assert_eq!(app.changes.file_index(Path::new("old.rs")), None);
        assert_eq!(app.changes.tree_index(Path::new("src/new.rs")), None);
        assert!(app.changes.selected_file().is_none());
        assert_eq!(app.changes.file_state.selected(), None);
        assert_eq!(app.changes.diff_state.selected(), None);
    }

    #[test]
    fn maps_visible_rows_to_hunks() {
        let mut file =
            ChangedFile::empty(PathBuf::from("file"), crate::model::FileStatus::Modified);
        file.hunks = vec![
            DiffHunk {
                id: crate::model::HunkId::synthetic("test-hunk-3"),
                header: "one".into(),
                kind: HunkKind::Staged,
                rows: vec![],
                collapsed: false,
            },
            DiffHunk {
                id: crate::model::HunkId::synthetic("test-hunk-4"),
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
        let visible = (0..tree.len()).collect::<Vec<_>>();
        assert_eq!(
            crate::ui::files::tree_label(&tree, 0, &visible),
            "├── AGENTS.md"
        );
        assert_eq!(crate::ui::files::tree_label(&tree, 1, &visible), "└── src");
        assert!(tree[1].is_directory());
        assert_eq!(
            crate::ui::files::tree_label(&tree, 2, &visible),
            "    ├── git"
        );
        assert_eq!(
            crate::ui::files::tree_label(&tree, 3, &visible),
            "    │   └── parser.rs"
        );
        assert_eq!(
            crate::ui::files::tree_label(&tree, 4, &visible),
            "    └── main.rs"
        );
    }

    #[test]
    fn file_navigation_skips_directories() {
        let tree = vec![
            FileTreeRow {
                path: PathBuf::from("src"),
                kind: FileTreeRowKind::Directory,
            },
            FileTreeRow {
                path: PathBuf::from("src/a.rs"),
                kind: FileTreeRowKind::File,
            },
            FileTreeRow {
                path: PathBuf::from("tests"),
                kind: FileTreeRowKind::Directory,
            },
            FileTreeRow {
                path: PathBuf::from("tests/a.rs"),
                kind: FileTreeRowKind::File,
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
                id: crate::model::HunkId::synthetic("test-hunk-5"),
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
                id: crate::model::HunkId::synthetic("test-hunk-6"),
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
            hunk_id: crate::model::HunkId::synthetic("test-hunk-6"),
            row_in_hunk: 1,
            scroll_offset: 1,
            collapsed_hunks: vec![crate::model::HunkId::synthetic("test-hunk-5")],
        };

        for hunk in &mut file.hunks {
            hunk.collapsed = position.collapsed_hunks.contains(&hunk.id);
        }
        assert_eq!(diff_row_for_position(&file, &position), Some(2));
        assert!(file.hunks[0].collapsed);
        assert!(!file.hunks[1].collapsed);
    }

    #[test]
    fn expanded_mode_follows_horizontal_focus_navigation() {
        let mut app = test_app();
        app.repository.worktrees.push(Worktree {
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
        app.repository.worktree_state.select(Some(0));
        assert!(!app.view.expanded);

        app.handle_key(key(KeyCode::Char(' ')));
        assert!(app.view.expanded);
        assert_eq!(app.view.focus, Focus::Worktrees);

        app.handle_key(key(KeyCode::Right));
        assert!(app.view.expanded);
        assert_eq!(app.view.focus, Focus::Files);

        app.handle_key(key(KeyCode::Right));
        assert!(app.view.expanded);
        assert_eq!(app.view.focus, Focus::Diff);

        app.handle_key(key(KeyCode::Char(' ')));
        assert!(!app.view.expanded);
        assert_eq!(app.view.focus, Focus::Diff);
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
        app.view.panel_layout = PanelLayout::Columns;
        app.history.worktree_panel = WorktreePanel::History;

        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.view.panel_layout, PanelLayout::SidebarLeft);

        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.view.panel_layout, PanelLayout::SidebarTop);

        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.view.panel_layout, PanelLayout::Columns);
    }

    #[test]
    fn history_mode_navigation_reaches_the_history_panel_without_linked_worktrees() {
        let mut app = test_app();
        app.history.worktree_panel = WorktreePanel::History;
        app.view.focus = Focus::Files;

        app.handle_key(key(KeyCode::Left));

        assert_eq!(app.view.focus, Focus::Worktrees);

        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.view.focus, Focus::Files);
    }

    #[test]
    fn h_toggles_history_from_worktrees_and_files() {
        let mut app = test_app();
        app.repository.worktrees.push(Worktree {
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
        app.repository.worktree_state.select(Some(0));
        app.repository.worktrees[0].path = repository.path().to_path_buf();
        app.handle_key(key(KeyCode::Char('h')));
        assert!(app.history_active());
        assert_eq!(app.view.focus, Focus::Worktrees);

        app.handle_key(key(KeyCode::Char('h')));
        assert!(!app.history_active());
        app.view.focus = Focus::Files;
        app.handle_key(key(KeyCode::Char('h')));
        assert!(app.history_active());
    }

    #[test]
    fn tab_changes_comparison_mode_while_history_is_active() {
        let mut app = test_app();
        app.history.worktree_panel = WorktreePanel::History;
        app.changes.mode = ChangeMode::Uncommitted;

        app.handle_key(key(KeyCode::Tab));

        assert_eq!(app.changes.mode, ChangeMode::Branch);
        assert!(app.history_active());
    }

    #[test]
    fn wip_is_the_first_history_entry_and_uses_live_changes() {
        let mut app = test_app();
        app.history.worktree_panel = WorktreePanel::History;
        app.history.commits = vec![Commit {
            hash: "1234567890abcdef".into(),
            parents: vec![],
            short_hash: "12345678".into(),
            subject: "commit".into(),
            graph: vec!["●".into()],
        }];
        app.history.selection = Some(HistorySelection::Commit {
            hash: "1234567890abcdef".into(),
        });

        app.move_commit(-1);

        assert_eq!(app.history.selection, Some(HistorySelection::Wip));
        assert!(!app.history_commit_selected());
    }

    #[test]
    fn history_commit_reload_prefers_the_remembered_file() {
        let mut app = test_app();
        app.history.preferred_file = Some(PathBuf::from("src/app.rs"));
        app.changes.install_files(vec![
            ChangedFile::empty(PathBuf::from("README.md"), FileStatus::Modified),
            ChangedFile::empty(PathBuf::from("src/app.rs"), FileStatus::Modified),
        ]);
        let selected = app
            .history
            .preferred_file
            .as_deref()
            .and_then(|path| {
                app.changes
                    .file_tree
                    .iter()
                    .position(|row| !row.is_directory() && row.path == path)
            })
            .or_else(|| app.changes.first_file_row());

        assert_eq!(
            selected.map(|row| app.changes.file_tree_row(row).unwrap().path.clone()),
            Some(PathBuf::from("src/app.rs"))
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
            history_selection_after_refresh(Some(&HistorySelection::Wip), &commits),
            Some(HistorySelection::Wip)
        );
        assert_eq!(
            history_selection_after_refresh(
                Some(&HistorySelection::Commit {
                    hash: "selected".into(),
                }),
                &commits,
            ),
            Some(HistorySelection::Commit {
                hash: "selected".into(),
            })
        );
        assert_eq!(
            history_selection_after_refresh(
                Some(&HistorySelection::Commit {
                    hash: "rewritten".into(),
                }),
                &commits,
            ),
            Some(HistorySelection::Wip)
        );
    }

    #[test]
    fn history_clicks_skip_graph_continuation_rows() {
        let mut app = test_app();
        app.history.worktree_panel = WorktreePanel::History;
        app.history.commits = vec![
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
        app.history.list_state.select(Some(0));

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
        app.changes.install_files(vec![ChangedFile {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            additions: 2,
            deletions: 2,
            hunks: vec![DiffHunk {
                id: crate::model::HunkId::synthetic("test-hunk-7"),
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
        }]);
        app.changes
            .select_file_path(Some(PathBuf::from("src/main.rs")));
        app.changes.file_state.select(Some(0));
        app.changes.diff_state.select(Some(0));
        app.view.focus = Focus::Diff;
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

        assert_eq!(app.view.focus, Focus::Diff);
        assert_eq!(app.changes.diff_state.selected(), Some(3));
    }

    #[test]
    fn changing_layout_from_expanded_mode_minimizes_the_panel() {
        let mut app = test_app();
        app.repository.worktrees.push(Worktree {
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
        app.view.expanded = true;
        app.view.panel_layout = PanelLayout::Columns;

        app.handle_key(key(KeyCode::Char('t')));

        assert!(!app.view.expanded);
        assert_eq!(app.view.panel_layout, PanelLayout::SidebarLeft);
        assert_eq!(app.view.focus, Focus::Worktrees);
    }

    #[test]
    fn narrow_initial_layout_starts_expanded_once() {
        let mut app = test_app();
        app.view.initial_layout_applied = false;

        app.apply_initial_layout(119, 120);
        assert!(app.view.expanded);
        assert_eq!(app.view.diff_layout, crate::model::DiffLayout::Unified);
        assert!(app.view.initial_layout_applied);

        app.view.expanded = false;
        app.apply_initial_layout(80, 120);
        assert!(
            !app.view.expanded,
            "manual minimize must remain authoritative"
        );
    }

    #[test]
    fn wide_initial_layout_starts_minimized() {
        let mut app = test_app();
        app.view.initial_layout_applied = false;
        app.view.expanded = true;

        app.apply_initial_layout(120, 120);
        assert!(!app.view.expanded);
        assert_eq!(app.view.diff_layout, crate::model::DiffLayout::Split);
    }

    #[test]
    fn a_single_worktree_can_start_in_history_mode() {
        let mut app = test_app();
        app.repository.worktrees.push(Worktree {
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

        let has_linked_worktrees = app
            .repository
            .worktrees
            .iter()
            .any(|worktree| !worktree.is_main);
        assert!(!has_linked_worktrees);
        app.history.worktree_panel = if has_linked_worktrees {
            WorktreePanel::Worktrees
        } else {
            WorktreePanel::History
        };
        app.history.selection = (!has_linked_worktrees).then_some(HistorySelection::Wip);

        assert!(app.history_active());
        assert_eq!(app.history.selection, Some(HistorySelection::Wip));
    }

    fn test_app() -> App {
        App {
            repository: RepositoryState {
                directory: PathBuf::from("/repo"),
                base: "main".into(),
                worktrees: vec![],
                worktree_state: ListState::default(),
            },
            changes: ChangeState::new(ChangeMode::Uncommitted, crate::model::DiffView::Hunks),
            history: HistoryState {
                list_state: ListState::default(),
                worktree_panel: WorktreePanel::Worktrees,
                commits: Vec::new(),
                range_commits: HashSet::new(),
                local_base_hash: None,
                remote_base_hash: None,
                branch_tips: HashMap::new(),
                head_hash: None,
                comparison_base: None,
                selection: None,
                preferred_file: None,
            },
            view: ViewState {
                diff_layout: crate::model::DiffLayout::Split,
                line_wrap: false,
                expanded: false,
                initial_layout_applied: true,
                panel_layout: PanelLayout::Columns,
                focus: Focus::Worktrees,
                show_help: false,
                delete_confirmation: None,
                status: None,
                worktree_filter: String::new(),
                file_filter: String::new(),
                search: None,
                follow_changes: false,
            },
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
