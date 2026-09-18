use crate::app::App;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PanelProjection {
    pub(crate) show_worktrees: bool,
    pub(crate) worktree_rows: Vec<usize>,
    pub(crate) file_rows: Vec<usize>,
    pub(crate) selected_worktree: Option<usize>,
    pub(crate) selected_file: Option<usize>,
    pub(crate) selected_hunk: Option<usize>,
}

pub(crate) fn panels(app: &App) -> PanelProjection {
    let worktree_rows = app.visible_worktree_indices();
    let file_rows = app.visible_file_rows();
    PanelProjection {
        show_worktrees: app.has_linked_worktrees() || app.history_active(),
        selected_worktree: app
            .repository
            .worktree_state
            .selected()
            .and_then(|selected| worktree_rows.iter().position(|index| *index == selected)),
        selected_file: app
            .changes
            .file_state
            .selected()
            .and_then(|selected| file_rows.iter().position(|index| *index == selected)),
        selected_hunk: app.selected_hunk_index(),
        worktree_rows,
        file_rows,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use ratatui::widgets::{ListState, TableState};

    use super::*;
    use crate::{
        app::{
            ChangeState, Focus, HistoryState, PanelLayout, RepositoryState, ViewState,
            WorktreePanel,
        },
        model::{
            ChangeMode, ChangedFile, Commit, DiffHunk, DiffRow, DiffRowKind, DiffView, FileStatus,
            HunkId, HunkKind, Worktree,
        },
    };

    #[test]
    fn derives_visible_rows_and_selection_positions() {
        let files = vec![
            ChangedFile::empty(PathBuf::from("tests/test.rs"), FileStatus::Modified),
            ChangedFile {
                path: PathBuf::from("src/main.rs"),
                old_path: None,
                status: FileStatus::Modified,
                additions: 1,
                deletions: 0,
                hunks: vec![DiffHunk {
                    id: HunkId::synthetic("main-hunk"),
                    header: "@@".into(),
                    kind: HunkKind::Unstaged,
                    rows: vec![DiffRow {
                        old_number: Some(1),
                        new_number: Some(1),
                        old_text: Some("old".into()),
                        new_text: Some("new".into()),
                        kind: DiffRowKind::Modified,
                    }],
                    collapsed: false,
                }],
                binary: false,
            },
        ];
        let file_tree = crate::app::files::build_tree(&files);
        let file_lookup = crate::app::files::FileLookup::build(&files, &file_tree);
        let selected_file_row = file_lookup
            .tree_indices
            .get(PathBuf::from("src/main.rs").as_path())
            .copied()
            .expect("selected file should be in the tree");

        let mut worktree_state = ListState::default();
        worktree_state.select(Some(1));
        let mut file_state = ListState::default();
        file_state.select(Some(selected_file_row));
        let mut diff_state = TableState::default();
        diff_state.select(Some(0));
        let app = App {
            repository: RepositoryState {
                directory: PathBuf::from("/repo"),
                base: "main".into(),
                worktrees: vec![
                    worktree("/repo", "main", true),
                    worktree("/repo/agent", "agent", false),
                ],
                worktree_state,
            },
            changes: ChangeState {
                mode: ChangeMode::Uncommitted,
                diff_view: DiffView::Hunks,
                files,
                file_tree,
                file_lookup,
                file_state,
                diff_state,
                selected_file_path: Some(PathBuf::from("src/main.rs")),
            },
            history: HistoryState {
                list_state: ListState::default(),
                worktree_panel: WorktreePanel::Worktrees,
                commits: Vec::<Commit>::new(),
                range_commits: HashSet::new(),
                local_base_hash: None,
                remote_base_hash: None,
                branch_tips: Default::default(),
                head_hash: None,
                selection: None,
                preferred_file: None,
            },
            view: ViewState {
                diff_layout: crate::model::DiffLayout::Split,
                line_wrap: false,
                expanded: false,
                initial_layout_applied: true,
                panel_layout: PanelLayout::Columns,
                focus: Focus::Files,
                show_help: false,
                delete_confirmation: None,
                status: None,
                worktree_filter: "agent".into(),
                file_filter: "main".into(),
                search: None,
            },
        };

        let projection = panels(&app);
        assert!(projection.show_worktrees);
        assert_eq!(projection.worktree_rows, vec![1]);
        assert_eq!(projection.selected_worktree, Some(0));
        assert_eq!(
            projection
                .file_rows
                .iter()
                .map(|index| app.changes.file_tree[*index].path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("src"), PathBuf::from("src/main.rs")]
        );
        assert_eq!(projection.selected_file, Some(1));
        assert_eq!(projection.selected_hunk, Some(0));
    }

    fn worktree(path: &str, branch: &str, is_main: bool) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            branch: branch.into(),
            head: "12345678".into(),
            dirty: false,
            is_current: is_main,
            is_main,
            available: true,
            prunable_reason: None,
            locked_reason: None,
        }
    }
}
