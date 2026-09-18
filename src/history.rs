use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::{Result, bail};

use crate::model::Commit;

use super::process::{git_allow_failure, git_text};

pub fn commit_history(worktree: &Path) -> Result<Vec<Commit>> {
    let output = git_text(
        worktree,
        &[
            "log",
            "--all",
            "--graph",
            "--format=%x01%H%x00%P%x00%h%x00%s",
            "--max-count=100",
        ],
    )?;
    Ok(parse_commit_history(&output))
}

pub fn head_hash(worktree: &Path) -> Result<String> {
    Ok(git_text(worktree, &["rev-parse", "HEAD"])?
        .trim()
        .to_string())
}

pub fn base_tip_hashes(worktree: &Path, base: &str) -> (Option<String>, Option<String>) {
    (
        resolve_ref_hash(worktree, base),
        resolve_ref_hash(worktree, &format!("origin/{base}")),
    )
}

pub fn branch_tips(worktree: &Path) -> Result<HashMap<String, Vec<String>>> {
    let output = git_text(
        worktree,
        &[
            "for-each-ref",
            "--format=%(objectname)\t%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let mut tips = HashMap::<String, Vec<String>>::new();
    for line in output.lines() {
        let Some((hash, reference)) = line.split_once('\t') else {
            continue;
        };
        let Some(name) = reference
            .strip_prefix("refs/heads/")
            .or_else(|| reference.strip_prefix("refs/remotes/"))
        else {
            continue;
        };
        if name.ends_with("/HEAD") {
            continue;
        }
        tips.entry((*hash).to_string())
            .or_default()
            .push((*name).to_string());
    }
    Ok(tips)
}

pub fn history_range_commits_from_base(
    worktree: &Path,
    target: &str,
    comparison_base: &str,
) -> Result<HashSet<String>> {
    let output = git_text(worktree, &["rev-list", target, "--not", comparison_base])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub fn is_ancestor(worktree: &Path, target: &str, base: &str) -> Result<bool> {
    Ok(
        git_allow_failure(worktree, &["merge-base", "--is-ancestor", target, base])?
            .status
            .success(),
    )
}

pub fn history_branch_ref(worktree: &Path, target: &str, base: &str) -> Result<Option<String>> {
    let base_ref = resolve_base_ref(worktree, base)?;
    let base_refs = [
        format!("refs/heads/{base_ref}"),
        format!("refs/remotes/{base_ref}"),
        format!("refs/remotes/origin/{base}"),
    ];
    let output = git_text(
        worktree,
        &[
            "for-each-ref",
            "--contains",
            target,
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let references: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|reference| {
            !reference.is_empty() && !base_refs.iter().any(|base_ref| base_ref == reference)
        })
        .collect();
    Ok(references
        .iter()
        .find(|reference| reference.starts_with("refs/heads/"))
        .or_else(|| references.first())
        .map(|reference| reference.to_string()))
}

pub fn history_comparison_base(
    worktree: &Path,
    target: &str,
    base: &str,
    comparison_ref: Option<&str>,
) -> Result<String> {
    let base_ref = resolve_base_ref(worktree, base)?;
    let Some(comparison_ref) = comparison_ref else {
        let output = git_text(worktree, &["merge-base", target, &base_ref])?;
        return Ok(output.trim().to_string());
    };
    let merges = git_text(
        worktree,
        &[
            "log",
            "--first-parent",
            "--merges",
            "--format=%H%x00%P",
            &base_ref,
        ],
    )?;

    for line in merges.lines() {
        let Some((_merge, parents)) = line.split_once('\0') else {
            continue;
        };
        let mut parents = parents.split_whitespace();
        let Some(first_parent) = parents.next() else {
            continue;
        };
        for second_parent in parents {
            let ancestor = git_allow_failure(
                worktree,
                &["merge-base", "--is-ancestor", comparison_ref, second_parent],
            )?;
            if ancestor.status.success() {
                return Ok(first_parent.to_string());
            }
        }
    }

    let output = git_text(worktree, &["merge-base", comparison_ref, &base_ref])?;
    Ok(output.trim().to_string())
}

pub(crate) fn resolve_base_ref(worktree: &Path, base: &str) -> Result<String> {
    let candidates = if base.contains('/') {
        vec![base.to_string()]
    } else {
        vec![base.to_string(), format!("origin/{base}")]
    };

    for candidate in candidates {
        let revision = format!("{candidate}^{{commit}}");
        let output = git_allow_failure(worktree, &["rev-parse", "--verify", "--quiet", &revision])?;
        if output.status.success() {
            return Ok(candidate);
        }
    }

    bail!(
        "base branch '{base}' was not found in {}; pass --base <branch> to select another base",
        worktree.display()
    )
}

fn parse_commit_history(output: &str) -> Vec<Commit> {
    let mut pending_graph = Vec::new();
    let mut commits = Vec::new();

    for line in output.lines() {
        let Some((graph, metadata)) = line.split_once('\u{1}') else {
            pending_graph.push(graph_text(line));
            continue;
        };
        let mut fields = metadata.split('\0');
        let Some(hash) = fields.next() else {
            continue;
        };
        let Some(parents) = fields.next() else {
            continue;
        };
        let Some(short_hash) = fields.next() else {
            continue;
        };
        let Some(subject) = fields.next() else {
            continue;
        };

        pending_graph.push(graph_text(graph));
        commits.push(Commit {
            hash: hash.to_string(),
            parents: parents.split_whitespace().map(ToOwned::to_owned).collect(),
            short_hash: short_hash.to_string(),
            subject: subject.to_string(),
            graph: std::mem::take(&mut pending_graph),
        });
    }

    commits
}

fn graph_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '|' => '│',
            '/' => '╱',
            '\\' => '╲',
            '-' => '─',
            '*' => '●',
            other => other,
        })
        .collect()
}

fn resolve_ref_hash(worktree: &Path, reference: &str) -> Option<String> {
    git_allow_failure(
        worktree,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{commit}}"),
        ],
    )
    .ok()
    .filter(|output| output.status.success())
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{
        process::git_text,
        test_support::{TestRepository, run_git},
    };
    use std::fs;

    #[test]
    fn parses_git_graph_lines_into_commit_rows() {
        let output = "* \u{1}head\u{0}base\u{0}head\u{0}head commit\n| * \u{1}side\u{0}head\u{0}side\u{0}side commit\n|/  \n* \u{1}base\u{0}\u{0}base\u{0}base commit\n";
        let commits = parse_commit_history(output);

        assert_eq!(commits.len(), 3);
        assert_eq!(commits[0].graph, vec!["● "]);
        assert_eq!(commits[0].parents, vec!["base"]);
        assert_eq!(commits[1].graph, vec!["│ ● "]);
        assert_eq!(commits[1].parents, vec!["head"]);
        assert_eq!(commits[2].graph, vec!["│╱  ", "● "]);
        assert!(commits[2].parents.is_empty());
    }

    #[test]
    fn history_range_commits_excludes_commits_already_on_base() {
        let repository = TestRepository::new();
        let base = git_text(repository.path(), &["rev-parse", "main"])
            .expect("base commit should resolve")
            .trim()
            .to_string();
        assert!(
            history_range_commits_from_base(
                repository.path(),
                "HEAD",
                &history_comparison_base(repository.path(), "HEAD", "main", None)
                    .expect("comparison base should resolve"),
            )
            .expect("main history range should load")
            .is_empty()
        );
        run_git(repository.path(), &["switch", "-c", "feature"]);
        fs::write(repository.path().join("feature.txt"), "feature\n")
            .expect("feature file should be writable");
        run_git(repository.path(), &["add", "feature.txt"]);
        run_git(repository.path(), &["commit", "-m", "feature commit"]);
        let head = git_text(repository.path(), &["rev-parse", "HEAD"])
            .expect("head commit should resolve")
            .trim()
            .to_string();

        let comparison_base = history_comparison_base(repository.path(), &head, "main", None)
            .expect("comparison base should resolve");
        let range = history_range_commits_from_base(repository.path(), &head, &comparison_base)
            .expect("history range should load");
        assert!(range.contains(&head));
        assert!(!range.contains(&base));
    }

    #[test]
    fn uses_the_branch_divergence_after_the_branch_is_merged() {
        let repository = TestRepository::new();
        run_git(repository.path(), &["switch", "-c", "feature"]);
        fs::write(repository.path().join("feature.txt"), "feature\n")
            .expect("feature file should be writable");
        run_git(repository.path(), &["add", "feature.txt"]);
        run_git(repository.path(), &["commit", "-m", "feature commit"]);
        let feature_commit = git_text(repository.path(), &["rev-parse", "HEAD"])
            .expect("feature commit should resolve")
            .trim()
            .to_string();
        run_git(repository.path(), &["switch", "main"]);
        run_git(
            repository.path(),
            &["merge", "--no-ff", "feature", "-m", "merge feature"],
        );

        let comparison_base =
            history_comparison_base(repository.path(), &feature_commit, "main", Some("feature"))
                .expect("comparison base should resolve");
        let range =
            history_range_commits_from_base(repository.path(), &feature_commit, &comparison_base)
                .expect("merged branch range should load");
        assert!(range.contains(&feature_commit));
    }

    #[test]
    fn groups_local_and_remote_branch_tips_by_commit() {
        let repository = TestRepository::new();
        run_git(repository.path(), &["branch", "feature"]);
        run_git(
            repository.path(),
            &["update-ref", "refs/remotes/origin/feature", "HEAD"],
        );

        let tips = branch_tips(repository.path()).expect("branch tips should load");
        let head = git_text(repository.path(), &["rev-parse", "HEAD"])
            .expect("head should resolve")
            .trim()
            .to_string();

        assert_eq!(
            tips.get(&head),
            Some(&vec![
                "feature".into(),
                "main".into(),
                "origin/feature".into()
            ])
        );
    }
}
