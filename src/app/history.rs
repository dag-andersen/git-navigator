use std::collections::{HashMap, HashSet};

use crate::{
    app::App,
    model::{Commit, HistorySelection},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoryRow {
    Wip {
        graph: String,
    },
    Graph(String),
    BranchLabel {
        graph: String,
        names: Vec<String>,
        connected: bool,
    },
    Commit {
        hash: String,
    },
}

pub(crate) fn display_rows(app: &App) -> Vec<HistoryRow> {
    display_rows_for(
        &app.history.commits,
        &app.history.branch_tips,
        app.history.head_hash.as_deref(),
        history_has_wip(app),
    )
}

fn display_rows_for(
    commits: &[Commit],
    branch_tips: &HashMap<String, Vec<String>>,
    head_hash: Option<&str>,
    has_wip: bool,
) -> Vec<HistoryRow> {
    let mut rows = Vec::new();
    let mut newer_parents = HashSet::new();
    for commit in commits {
        rows.extend(
            commit
                .graph
                .iter()
                .take(commit.graph.len().saturating_sub(1))
                .cloned()
                .map(HistoryRow::Graph),
        );
        if has_wip && head_hash == Some(commit.hash.as_str()) {
            rows.push(HistoryRow::Wip {
                graph: commit.graph.last().cloned().unwrap_or_else(|| "●".into()),
            });
        }
        if let Some(names) = branch_tips.get(&commit.hash) {
            rows.push(HistoryRow::BranchLabel {
                graph: commit.graph.last().cloned().unwrap_or_else(|| "●".into()),
                names: names.clone(),
                connected: newer_parents.contains(commit.hash.as_str()),
            });
        }
        rows.push(HistoryRow::Commit {
            hash: commit.hash.clone(),
        });
        newer_parents.extend(commit.parents.iter().map(String::as_str));
    }
    if has_wip && !rows.iter().any(|row| matches!(row, HistoryRow::Wip { .. })) {
        rows.insert(
            0,
            HistoryRow::Wip {
                graph: "●".into()
            },
        );
    }
    rows
}

pub(crate) fn selection_after_refresh(
    selection: Option<&HistorySelection>,
    commits: &[Commit],
    head_hash: Option<&str>,
    has_wip: bool,
) -> Option<HistorySelection> {
    match selection {
        Some(HistorySelection::Wip) if has_wip => Some(HistorySelection::Wip),
        Some(HistorySelection::Commit { hash })
            if commits.iter().any(|commit| commit.hash == *hash) =>
        {
            Some(HistorySelection::Commit { hash: hash.clone() })
        }
        _ => default_selection(commits, head_hash, has_wip),
    }
}

pub(crate) fn list_index(app: &App, visible_row: usize) -> Option<usize> {
    match display_rows(app).get(visible_row + app.history.list_state.offset())? {
        HistoryRow::Wip { .. } => Some(0),
        HistoryRow::Commit { hash } => app
            .history
            .commits
            .iter()
            .position(|commit| commit.hash == *hash)
            .map(|index| index + usize::from(history_has_wip(app))),
        HistoryRow::Graph(_) | HistoryRow::BranchLabel { .. } => None,
    }
}

pub(crate) fn default_selection(
    commits: &[Commit],
    head_hash: Option<&str>,
    has_wip: bool,
) -> Option<HistorySelection> {
    if has_wip {
        return Some(HistorySelection::Wip);
    }
    head_hash
        .filter(|hash| commits.iter().any(|commit| commit.hash == *hash))
        .map(|hash| HistorySelection::Commit {
            hash: hash.to_string(),
        })
        .or_else(|| {
            commits.first().map(|commit| HistorySelection::Commit {
                hash: commit.hash.clone(),
            })
        })
}

pub(crate) fn history_has_wip(app: &App) -> bool {
    app.selected_worktree()
        .is_some_and(|worktree| worktree.dirty)
}

pub(crate) fn selectable_selections(app: &App) -> Vec<HistorySelection> {
    display_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            HistoryRow::Wip { .. } => Some(HistorySelection::Wip),
            HistoryRow::Commit { hash } => Some(HistorySelection::Commit { hash }),
            HistoryRow::Graph(_) | HistoryRow::BranchLabel { .. } => None,
        })
        .collect()
}

pub(crate) fn visual_index(app: &App, selection: Option<&HistorySelection>) -> Option<usize> {
    let target = selection?;
    display_rows(app).iter().position(|row| match row {
        HistoryRow::Wip { .. } => matches!(target, HistorySelection::Wip),
        HistoryRow::Commit { hash } => {
            matches!(target, HistorySelection::Commit { hash: selected } if selected == hash)
        }
        HistoryRow::Graph(_) | HistoryRow::BranchLabel { .. } => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_branch_labels_without_making_them_selectable() {
        let commits = vec![Commit {
            hash: "tip".into(),
            parents: vec![],
            short_hash: "tip".into(),
            subject: "tip".into(),
            graph: vec!["│╲  ".into(), "● ".into()],
        }];
        let branch_tips = HashMap::from([(String::from("tip"), vec![String::from("feature")])]);

        assert_eq!(
            display_rows_for(&commits, &branch_tips, Some("tip"), true),
            vec![
                HistoryRow::Graph("│╲  ".into()),
                HistoryRow::Wip {
                    graph: "● ".into()
                },
                HistoryRow::BranchLabel {
                    graph: "● ".into(),
                    names: vec!["feature".into()],
                    connected: false,
                },
                HistoryRow::Commit { hash: "tip".into() },
            ]
        );
    }

    #[test]
    fn connects_branch_labels_when_a_newer_commit_has_the_tip_as_parent() {
        let commits = vec![
            Commit {
                hash: "child".into(),
                parents: vec!["tip".into()],
                short_hash: "child".into(),
                subject: "child".into(),
                graph: vec!["● ".into()],
            },
            Commit {
                hash: "tip".into(),
                parents: vec!["base".into()],
                short_hash: "tip".into(),
                subject: "tip".into(),
                graph: vec!["│  ".into(), "● ".into()],
            },
        ];
        let branch_tips = HashMap::from([(String::from("tip"), vec![String::from("feature")])]);

        assert!(
            display_rows_for(&commits, &branch_tips, None, false)
                .iter()
                .any(|row| {
                    matches!(
                        row,
                        HistoryRow::BranchLabel {
                            connected: true,
                            ..
                        }
                    )
                })
        );
    }

    #[test]
    fn omits_wip_when_the_worktree_is_clean() {
        let commits = vec![Commit {
            hash: "tip".into(),
            parents: vec![],
            short_hash: "tip".into(),
            subject: "tip".into(),
            graph: vec!["● ".into()],
        }];

        let rows = display_rows_for(&commits, &HashMap::new(), Some("tip"), false);

        assert!(!rows.iter().any(|row| matches!(row, HistoryRow::Wip { .. })));
        assert_eq!(
            default_selection(&commits, Some("tip"), false),
            Some(HistorySelection::Commit { hash: "tip".into() })
        );
    }
}
