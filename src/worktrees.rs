use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::model::Worktree;

use super::process::{ensure_git_success, git, git_command, git_text};

pub fn discover_worktrees(directory: &Path) -> Result<Vec<Worktree>> {
    let current_root = git_text(directory, &["rev-parse", "--show-toplevel"])?;
    let current_root = canonicalize_or_original(Path::new(current_root.trim()));
    let output = git_text(directory, &["worktree", "list", "--porcelain"])?;
    let mut worktrees = Vec::new();
    let mut record = WorktreeRecord::default();

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(path) = record.path.take() {
                let canonical_path = canonicalize_or_original(&path);
                let available = path.is_dir() && record.prunable_reason.is_none();
                let dirty = available && is_dirty(&path).unwrap_or(false);
                worktrees.push(Worktree {
                    is_current: canonical_path == current_root,
                    is_main: worktrees.is_empty(),
                    path,
                    branch: record
                        .branch
                        .take()
                        .unwrap_or_else(|| "detached HEAD".to_string()),
                    head: record.head.take().unwrap_or_default(),
                    dirty,
                    available,
                    prunable_reason: record.prunable_reason.take(),
                    locked_reason: record.locked_reason.take(),
                });
            }
            record = WorktreeRecord::default();
            continue;
        }

        if let Some(value) = line.strip_prefix("worktree ") {
            record.path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            record.head = Some(value.chars().take(8).collect());
        } else if let Some(value) = line.strip_prefix("branch ") {
            record.branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_string(),
            );
        } else if let Some(value) = line.strip_prefix("prunable") {
            record.prunable_reason = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("locked") {
            record.locked_reason = Some(value.trim().to_string());
        }
    }

    if worktrees.is_empty() {
        bail!("{} does not belong to a Git worktree", directory.display());
    }
    Ok(worktrees)
}

pub fn remove_worktree(repository: &Path, worktree: &Worktree) -> Result<()> {
    if worktree.is_current {
        bail!("cannot remove the worktree currently opened by git-navigator");
    }
    if worktree.is_main {
        bail!("Git does not allow removing the main worktree");
    }
    if let Some(reason) = &worktree.locked_reason {
        if reason.is_empty() {
            bail!("the selected worktree is locked");
        }
        bail!("the selected worktree is locked: {reason}");
    }

    if worktree.is_missing() {
        remove_missing_worktree_metadata(repository, worktree)?;
        return Ok(());
    }
    if worktree.dirty {
        bail!("the selected worktree contains uncommitted or untracked changes");
    }

    let output = git_command(repository)
        .args([OsStr::new("worktree"), OsStr::new("remove")])
        .arg(&worktree.path)
        .output()
        .with_context(|| format!("failed to remove worktree {}", worktree.path.display()))?;
    ensure_git_success(output, repository, "git worktree remove")?;
    Ok(())
}

fn remove_missing_worktree_metadata(repository: &Path, worktree: &Worktree) -> Result<()> {
    if worktree.prunable_reason.is_none() {
        bail!("Git has not marked the selected worktree as prunable");
    }
    let worktree_git_file = worktree.path.join(".git");
    match fs::symlink_metadata(&worktree_git_file) {
        Ok(_) => bail!(
            "refusing stale cleanup because {} still exists",
            worktree_git_file.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "could not inspect worktree Git link {}",
                    worktree_git_file.display()
                )
            });
        }
    }

    let common_dir = git_text(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let metadata_root = PathBuf::from(common_dir.trim()).join("worktrees");
    let mut matches = Vec::new();

    let entries = match fs::read_dir(&metadata_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "could not find Git metadata registered for {}",
            worktree.path.display()
        ),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("could not read {}", metadata_root.display()));
        }
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("could not read {}", metadata_root.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("could not inspect {}", entry.path().display()))?
            .is_dir()
        {
            continue;
        }
        let gitdir_file = entry.path().join("gitdir");
        let Ok(gitdir) = fs::read_to_string(&gitdir_file) else {
            continue;
        };
        let gitdir = Path::new(gitdir.trim());
        if gitdir.file_name() == Some(OsStr::new(".git"))
            && gitdir.parent() == Some(worktree.path.as_path())
        {
            matches.push(entry.path());
        }
    }

    let metadata = match matches.as_slice() {
        [metadata] => metadata,
        [] => bail!(
            "could not find Git metadata registered for {}",
            worktree.path.display()
        ),
        _ => bail!(
            "multiple Git metadata entries refer to {}; refusing cleanup",
            worktree.path.display()
        ),
    };
    fs::remove_dir_all(metadata)
        .with_context(|| format!("could not remove stale metadata at {}", metadata.display()))?;
    Ok(())
}

#[derive(Default)]
struct WorktreeRecord {
    path: Option<PathBuf>,
    branch: Option<String>,
    head: Option<String>,
    prunable_reason: Option<String>,
    locked_reason: Option<String>,
}

fn is_dirty(path: &Path) -> Result<bool> {
    Ok(!git(
        path,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?
    .stdout
    .is_empty())
}

fn canonicalize_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::git::test_support::{TestRepository, path_text, run_git};

    #[test]
    fn discovers_linked_worktrees() {
        let repository = TestRepository::new();
        let linked = repository.root.path().join("linked-worktree");
        run_git(
            repository.path(),
            &["worktree", "add", "-b", "topic", path_text(&linked), "HEAD"],
        );

        let worktrees = discover_worktrees(repository.path()).expect("worktrees should load");
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees.iter().any(|worktree| worktree.is_current));
        assert!(worktrees.iter().any(|worktree| worktree.branch == "topic"));
    }

    #[test]
    fn discovers_and_removes_a_missing_worktree_registration() {
        let repository = TestRepository::new();
        let linked = repository.root.path().join("missing-worktree");
        run_git(
            repository.path(),
            &["worktree", "add", "-b", "stale", path_text(&linked), "HEAD"],
        );
        fs::remove_file(linked.join(".git")).expect("worktree Git link should be removable");

        let worktrees = discover_worktrees(repository.path()).expect("worktrees should load");
        let stale = worktrees
            .iter()
            .find(|worktree| worktree.branch == "stale")
            .expect("missing worktree should remain listed");
        assert!(stale.is_missing());
        assert!(stale.prunable_reason.is_some());

        remove_worktree(repository.path(), stale).expect("stale metadata should be removed");
        let refreshed = discover_worktrees(repository.path()).expect("worktrees should refresh");
        assert!(refreshed.iter().all(|worktree| worktree.branch != "stale"));
        assert!(linked.is_dir(), "orphaned directory must be preserved");
        assert!(
            linked.join("tracked.txt").is_file(),
            "orphaned files must be preserved"
        );
        let branch = git_text(repository.path(), &["branch", "--list", "stale"])
            .expect("branch list should load");
        assert!(
            !branch.trim().is_empty(),
            "cleanup must preserve the branch"
        );
    }

    #[test]
    fn stale_cleanup_refuses_a_path_not_present_in_metadata() {
        let repository = TestRepository::new();
        let unregistered = Worktree {
            path: repository.root.path().join("not-registered"),
            branch: "stale".into(),
            head: "12345678".into(),
            dirty: false,
            is_current: false,
            is_main: false,
            available: false,
            prunable_reason: Some("missing".into()),
            locked_reason: None,
        };

        let error = remove_worktree(repository.path(), &unregistered)
            .expect_err("an unregistered path must not be removed");
        assert!(
            error
                .to_string()
                .contains("could not find Git metadata registered")
        );
    }
}
