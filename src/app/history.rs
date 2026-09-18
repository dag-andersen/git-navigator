use std::collections::HashMap;

use crate::{
    app::App,
    model::{Commit, HistoryRow},
};

pub(crate) fn display_rows(app: &App) -> Vec<HistoryRow> {
    display_rows_for(
        &app.commits,
        &app.branch_tips,
        app.history_head_hash.as_deref(),
    )
}

fn display_rows_for(
    commits: &[Commit],
    branch_tips: &HashMap<String, Vec<String>>,
    head_hash: Option<&str>,
) -> Vec<HistoryRow> {
    let mut rows = Vec::new();
    for (index, commit) in commits.iter().enumerate() {
        rows.extend(
            commit
                .graph
                .iter()
                .take(commit.graph.len().saturating_sub(1))
                .cloned()
                .map(HistoryRow::Graph),
        );
        if head_hash == Some(commit.hash.as_str()) {
            rows.push(HistoryRow::Wip {
                graph: commit.graph.last().cloned().unwrap_or_else(|| "●".into()),
            });
        }
        if let Some(names) = branch_tips.get(&commit.hash) {
            let connected = commits[..index]
                .iter()
                .any(|newer| newer.parents.iter().any(|parent| parent == &commit.hash));
            rows.push(HistoryRow::BranchLabel {
                graph: commit.graph.last().cloned().unwrap_or_else(|| "●".into()),
                names: names.clone(),
                connected,
            });
        }
        rows.push(HistoryRow::Commit { index });
    }
    if !rows.iter().any(|row| matches!(row, HistoryRow::Wip { .. })) {
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
    match display_rows(app).get(visible_row + app.history_state.offset())? {
        HistoryRow::Wip { .. } => Some(0),
        HistoryRow::Commit { index } => Some(index + 1),
        HistoryRow::Graph(_) | HistoryRow::BranchLabel { .. } => None,
    }
}

pub(crate) fn selectable_indices(app: &App) -> Vec<usize> {
    display_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            HistoryRow::Wip { .. } => Some(0),
            HistoryRow::Commit { index } => Some(index + 1),
            HistoryRow::Graph(_) | HistoryRow::BranchLabel { .. } => None,
        })
        .collect()
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
            display_rows_for(&commits, &branch_tips, Some("tip")),
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
                HistoryRow::Commit { index: 0 },
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
            display_rows_for(&commits, &branch_tips, None)
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
}
