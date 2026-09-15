use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result};
use arboard::Clipboard;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent},
    widgets::{ListState, TableState},
};

use crate::{
    git,
    model::{ChangeMode, ChangedFile, DiffLayout, DiffView, FileTreeRow, Worktree},
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

impl Focus {
    fn left(self) -> Self {
        match self {
            Self::Worktrees => Self::Worktrees,
            Self::Files => Self::Worktrees,
            Self::Diff => Self::Files,
        }
    }

    fn right(self) -> Self {
        match self {
            Self::Worktrees => Self::Files,
            Self::Files => Self::Diff,
            Self::Diff => Self::Diff,
        }
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
    pub file_state: ListState,
    pub diff_state: TableState,
    pub show_help: bool,
    pub delete_confirmation: Option<DeleteConfirmation>,
    pub status: Option<StatusMessage>,
    pub worktree_filter: String,
    pub file_filter: String,
    pub search: Option<SearchState>,
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

        let mut worktree_state = ListState::default();
        worktree_state.select(Some(selected_worktree));
        let file_tree = build_file_tree(&files);
        let mut file_state = ListState::default();
        file_state.select(first_file_row(&file_tree));
        let mut diff_state = TableState::default();
        diff_state.select(first_diff_row(&files, &file_tree, file_state.selected()));

        Ok(Self {
            directory,
            base,
            mode: ChangeMode::Uncommitted,
            diff_view: DiffView::Hunks,
            diff_layout: DiffLayout::Split,
            line_wrap: false,
            expanded: false,
            initial_layout_applied: false,
            panel_layout: PanelLayout::Columns,
            focus: Focus::Worktrees,
            worktrees,
            files,
            file_tree,
            worktree_state,
            file_state,
            diff_state,
            show_help: false,
            delete_confirmation: None,
            status: None,
            worktree_filter: String::new(),
            file_filter: String::new(),
            search: None,
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
            KeyCode::Tab => self.toggle_mode(),
            KeyCode::Char('v') => self.toggle_diff_view(),
            KeyCode::Char('s') => self.diff_layout = self.diff_layout.toggle(),
            KeyCode::Char('w') => self.line_wrap = !self.line_wrap,
            KeyCode::Char(' ') => self.expanded = !self.expanded,
            KeyCode::Char('t') => {
                self.panel_layout = self.panel_layout.toggle();
                self.expanded = false;
            }
            KeyCode::Char('/') if self.focus != Focus::Diff => self.begin_search(),
            KeyCode::Left | KeyCode::Char('h') => self.focus = self.focus.left(),
            KeyCode::Right | KeyCode::Char('l') => self.focus = self.focus.right(),
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
        self.reload_files(None);
    }

    fn toggle_diff_view(&mut self) {
        let selected_file = self.selected_file().map(|file| file.path.clone());
        self.diff_view = self.diff_view.toggle();
        self.reload_files(selected_file.as_deref());
    }

    pub fn refresh(&mut self) {
        let selected_worktree = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone());
        let selected_file = self.selected_file().map(|file| file.path.clone());
        let diff_position = self.diff_position();

        match git::discover_worktrees(&self.directory) {
            Ok(worktrees) => {
                self.worktrees = worktrees;
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
                self.reload_files_with_position(selected_file.as_deref(), diff_position.as_ref());
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
            }
            KeyCode::Char(character) => {
                let mut query = search.query;
                query.push(character);
                self.search = Some(SearchState {
                    focus: search.focus,
                    query: query.clone(),
                });
                self.set_filter(search.focus, query);
            }
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
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
            Focus::Diff => {}
        }
    }

    fn clear_filter(&mut self, focus: Focus) {
        if !self.search_query(focus).is_empty() {
            self.set_filter(focus, String::new());
        }
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
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

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
    fn slash_is_not_search_on_the_diff_panel() {
        let mut app = test_app();
        app.focus = Focus::Diff;
        app.handle_key(key(KeyCode::Char('/')));
        assert!(app.search.is_none());
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
    fn changing_layout_from_expanded_mode_minimizes_the_panel() {
        let mut app = test_app();
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
            file_state: ListState::default(),
            diff_state: TableState::default(),
            show_help: false,
            delete_confirmation: None,
            status: None,
            worktree_filter: String::new(),
            file_filter: String::new(),
            search: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }
}
