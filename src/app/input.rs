use super::{
    App, Focus, SearchState, diff_row_at, diff_row_at_position, first_diff_row, history_list_index,
    mouse_focus,
};
use crate::diff_geometry::effective_layout;
use crate::editor;
use arboard::Clipboard;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::{Position, Rect},
};

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.view.delete_confirmation.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_worktree_removal(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.view.delete_confirmation = None;
                    self.set_info("Worktree cleanup cancelled");
                }
                _ => {}
            }
            return false;
        }

        if self.view.show_help {
            match key.code {
                KeyCode::Char('q') => return true,
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter => self.view.show_help = false,
                _ => {}
            }
            return false;
        }

        if self.view.search.is_some() {
            self.handle_search_key(key);
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc if self.view.focus != Focus::Diff => self.clear_filter(self.view.focus),
            KeyCode::Char('?') => self.view.show_help = true,
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('f') => self.toggle_follow_changes(),
            KeyCode::Char('o') => self.open_selected_worktree(),
            KeyCode::Char('d') if self.view.focus == Focus::Worktrees => {
                self.request_worktree_removal()
            }
            KeyCode::Char('h') if matches!(self.view.focus, Focus::Worktrees | Focus::Files) => {
                self.toggle_history()
            }
            KeyCode::Tab => self.toggle_mode(),
            KeyCode::Char('v') => self.toggle_diff_view(),
            KeyCode::Char('s') => self.view.diff_layout = self.view.diff_layout.toggle(),
            KeyCode::Char('w') => self.view.line_wrap = !self.view.line_wrap,
            KeyCode::Char(' ') => self.view.expanded = !self.view.expanded,
            KeyCode::Char('t') => {
                self.view.panel_layout = self.view.panel_layout.toggle();
                self.view.expanded = false;
            }
            KeyCode::Char('/') => self.begin_search(),
            KeyCode::Left | KeyCode::Char('h') => self.focus_left(),
            KeyCode::Right => self.focus_right(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::PageUp if self.view.focus == Focus::Diff => self.move_diff_by(-10),
            KeyCode::PageDown if self.view.focus == Focus::Diff => self.move_diff_by(10),
            KeyCode::Home if self.view.focus == Focus::Diff => self.select_diff_row(0),
            KeyCode::End if self.view.focus == Focus::Diff => {
                let count = self.diff_row_count();
                if count > 0 {
                    self.select_diff_row(count - 1);
                }
            }
            KeyCode::Enter if self.view.focus == Focus::Diff => self.toggle_selected_hunk(),
            KeyCode::Char('c') if self.view.focus == Focus::Diff => self.copy_selected_location(),
            _ => {}
        }
        false
    }

    fn toggle_follow_changes(&mut self) {
        self.view.follow_changes = !self.view.follow_changes;
        self.view.status = None;
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent, areas: [Rect; 3]) {
        if self.modal_open() || self.view.search.is_some() {
            return;
        }
        let position = Position::new(mouse.column, mouse.row);
        let show_worktrees = self.has_linked_worktrees() || self.history_active();

        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some(focus) = mouse_focus(position, areas, show_worktrees) else {
                    return;
                };
                self.view.focus = focus;
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
            self.view.focus = Focus::Worktrees;
            if let Some(index) = self.list_index(position, areas[0], history) {
                if history {
                    self.select_commit(index);
                } else {
                    self.select_worktree(index);
                }
            }
        } else if areas[1].contains(position) {
            self.view.focus = Focus::Files;
            if let Some(row) = self.list_index(position, areas[1], false) {
                self.select_file_row(row);
            }
        } else if areas[2].contains(position) {
            let diff_rows_visible = self.view.expanded || self.view.focus == Focus::Diff;
            self.view.focus = Focus::Diff;
            if diff_rows_visible && let Some(row) = self.diff_row_at_position(position, areas[2]) {
                self.select_diff_row(row);
            }
        }
    }

    fn diff_row_at_position(&self, position: Position, area: Rect) -> Option<usize> {
        let file = self.selected_file()?;
        let effective_layout = effective_layout(file, self.view.diff_layout);
        diff_row_at_position(
            file,
            effective_layout,
            self.view.line_wrap,
            self.changes.diff_state.offset(),
            position,
            area,
        )
    }

    pub(crate) fn list_index(
        &self,
        position: Position,
        area: Rect,
        history: bool,
    ) -> Option<usize> {
        let content_top = area.y.saturating_add(1);
        if position.y < content_top {
            return None;
        }
        let row = usize::from(position.y - content_top);
        if history {
            return history_list_index(self, row);
        }

        let item_height = if self.view.focus == Focus::Worktrees {
            2
        } else {
            1
        };
        Some(
            row / item_height
                + if self.view.focus == Focus::Worktrees {
                    self.repository.worktree_state.offset()
                } else {
                    self.changes.file_state.offset()
                },
        )
    }

    fn begin_search(&mut self) {
        self.view.search = Some(SearchState {
            focus: self.view.focus,
            query: self.search_query(self.view.focus).to_string(),
        });
    }

    fn open_selected_worktree(&mut self) {
        let Some(path) = self
            .selected_worktree()
            .map(|worktree| worktree.path.clone())
        else {
            self.set_error("No worktree is selected");
            return;
        };

        match editor::open(&path) {
            Ok(()) => self.set_info(format!("Opened {} in the default editor", path.display())),
            Err(error) => self.set_error(format!(
                "Could not open {} in the default editor: {error}",
                path.display()
            )),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let Some(search) = self.view.search.clone() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                let focus = search.focus;
                self.set_filter(focus, String::new());
                self.view.search = None;
            }
            KeyCode::Enter if search.focus == Focus::Diff => self.search_diff(1),
            KeyCode::Enter => self.view.search = None,
            KeyCode::Left => self.switch_search_panel(false),
            KeyCode::Right => self.switch_search_panel(true),
            KeyCode::Backspace => {
                let mut query = search.query;
                query.pop();
                self.view.search = Some(SearchState {
                    focus: search.focus,
                    query: query.clone(),
                });
                self.set_filter(search.focus, query);
                if search.focus == Focus::Diff {
                    let query = self
                        .view
                        .search
                        .as_ref()
                        .map_or(String::new(), |search| search.query.clone());
                    self.search_diff_query(&query, -1);
                }
            }
            KeyCode::Char(character) => {
                let mut query = search.query;
                query.push(character);
                self.view.search = Some(SearchState {
                    focus: search.focus,
                    query: query.clone(),
                });
                self.set_filter(search.focus, query);
                if search.focus == Focus::Diff {
                    let query = self
                        .view
                        .search
                        .as_ref()
                        .map_or(String::new(), |search| search.query.clone());
                    self.search_diff_query(&query, 0);
                }
            }
            KeyCode::Up => self.search_diff(-1),
            KeyCode::Down => self.search_diff(1),
            _ => {}
        }
    }

    fn switch_search_panel(&mut self, right: bool) {
        let Some(current_focus) = self.view.search.as_ref().map(|search| search.focus) else {
            return;
        };
        let next_focus = match (current_focus, right) {
            (Focus::Worktrees, true) => Focus::Files,
            (Focus::Files, false) => Focus::Worktrees,
            (focus, _) => focus,
        };
        let query = self.search_query(next_focus).to_string();
        self.view.focus = next_focus;
        self.set_filter(next_focus, query);
        self.view.search = None;
    }

    fn set_filter(&mut self, focus: Focus, filter: String) {
        match focus {
            Focus::Worktrees => {
                self.view.worktree_filter = filter;
                let visible = self.visible_worktree_indices();
                let selected = self
                    .repository
                    .worktree_state
                    .selected()
                    .filter(|selected| visible.contains(selected))
                    .or_else(|| visible.first().copied());
                self.repository.worktree_state.select(selected);
                self.reload_files(None);
            }
            Focus::Files => {
                self.view.file_filter = filter;
                let visible = self.visible_file_rows();
                let selected = self
                    .changes
                    .file_state
                    .selected()
                    .filter(|selected| visible.contains(selected))
                    .filter(|selected| {
                        self.changes
                            .file_tree_row(*selected)
                            .is_some_and(|row| !row.is_directory())
                    })
                    .or_else(|| self.changes.first_file_row_in(&visible));
                let selected_path = selected
                    .and_then(|row| self.changes.file_tree_row(row))
                    .map(|row| row.path.clone());
                self.select_file_path(selected_path);
                self.changes
                    .diff_state
                    .select(first_diff_row(&self.changes, selected));
            }
            Focus::Diff => self.search_diff(0),
        }
    }

    fn clear_filter(&mut self, focus: Focus) {
        if !self.search_query(focus).is_empty() {
            self.set_filter(focus, String::new());
        }
    }

    fn focus_left(&mut self) {
        self.view.focus = self
            .view
            .focus
            .left(self.has_linked_worktrees() || self.history_active());
    }

    fn focus_right(&mut self) {
        self.view.focus = self
            .view
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

    pub(crate) fn selected_location(&self) -> Option<String> {
        let file = self.selected_file()?;
        let worktree = self.selected_worktree()?;
        let selected_row = self.changes.diff_state.selected()?;
        let row = diff_row_at(file, selected_row)?;
        let line = row.new_number.or(row.old_number)?;
        Some(format!(
            "{}:{line}",
            worktree.path.join(&file.path).display()
        ))
    }
}
