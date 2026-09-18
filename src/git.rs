#[path = "changes.rs"]
mod changes;
#[path = "history.rs"]
mod history;
#[path = "process.rs"]
mod process;
#[path = "worktrees.rs"]
mod worktrees;

pub use changes::{load_changes, load_commit_changes};
pub use history::{base_tip_hashes, branch_tips, commit_history, head_hash, history_range_commits};
pub use process::common_git_dir;
pub use worktrees::{discover_worktrees, remove_worktree};

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use tempfile::TempDir;

    pub(crate) struct TestRepository {
        pub(crate) root: TempDir,
        repository: PathBuf,
    }

    impl TestRepository {
        pub(crate) fn new() -> Self {
            let root = tempfile::tempdir().expect("temporary directory should be created");
            let repository = root.path().join("repository");
            fs::create_dir(&repository).expect("repository directory should be created");
            run_git(&repository, &["init", "-b", "main"]);
            run_git(&repository, &["config", "user.name", "Git Navigator Tests"]);
            run_git(
                &repository,
                &["config", "user.email", "git-navigator@example.invalid"],
            );
            fs::write(repository.join("tracked.txt"), "original\n")
                .expect("tracked file should be writable");
            run_git(&repository, &["add", "tracked.txt"]);
            run_git(&repository, &["commit", "-m", "initial"]);
            Self { root, repository }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.repository
        }
    }

    pub(crate) fn run_git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub(crate) fn path_text(path: &Path) -> &str {
        path.to_str().expect("temporary path should be UTF-8")
    }
}
