# git-navigator

A read-only Rust TUI built with Ratatui for inspecting changes across the Git worktrees of a repository.

## Installation

### Install from GitHub

Install Rust with [rustup](https://rustup.rs/) if it is not already available, then install the latest version of `git-navigator`:

```shell
cargo install --git https://github.com/dag-andersen/git-navigator
```

The `git-navigator` binary is installed into Cargo's binary directory, which is usually `$HOME/.cargo/bin`. Make sure that directory is on your `PATH`.

### Install from source

```shell
git clone https://github.com/dag-andersen/git-navigator.git
cd git-navigator
cargo install --path .
```

Git must also be installed and available on `PATH`.

## Usage

The first argument is a directory anywhere inside the repository to inspect:

```shell
git-navigator .
```

The base branch defaults to `main`. Override it when needed:

```shell
git-navigator /path/to/repository --base trunk
```

## Layout

Use a three-pane, file-explorer-style layout. The changed-files pane renders the changed paths as a folder tree:

```text
| Worktrees       | Changed files       | Split diff                    |
| current: main   | ├── AGENTS.md       | Before          | After       |
| agent: feature  | ├── src             | removed line    | added line  |
| agent: tests    | │   ├── git.rs      | context         | context     |
|                 | │   └── main.rs     |                 |             |
|                 | └── tests           |                 |             |
|                 |     └── git.rs      |                 |             |
```

Directories in the tree are visual grouping rows and cannot be selected. `Up` and `Down` jump directly between changed files. The diff pane is scrollable and displays each file as a side-by-side diff, with the old version on the left and the new version on the right. Press `v` to switch between a concise view containing Git diff hunks with surrounding context and a full-file view containing every line of the changed file. Both views include old and new line numbers.

## Worktrees

Discover and list all Git worktrees associated with the current repository. Each entry should show enough context to identify it, including its directory, current branch, and dirty state. Selecting a worktree updates the file and diff panes.

The app is intended to make worktrees created by developers or AI agents quick to inspect without changing directories.

## Change modes

Press `Tab` to switch between two modes.

### Uncommitted

Show all local changes that have not been committed:

- Staged changes between `HEAD` and the index
- Unstaged changes between the index and the working tree
- Untracked files as newly added files

In the Hunks view, hunks receive a staged or unstaged badge. Staged changed lines also use a distinct foreground color so they remain easy to recognize. A file may contain both staged and unstaged hunks. The Full File view combines the index and working tree into the final local version and emphasizes the resulting additions and deletions rather than their staging provenance.

### Branch

Show everything changed since the current branch diverged from `main`. Find the merge base of `HEAD` and the base branch, then compare that commit with the current working tree. This includes committed, staged, unstaged, and untracked changes.

In this mode, line provenance is not important:

- Added lines are green
- Deleted lines are red
- Files containing both additions and deletions are blue in the file list

The base branch defaults to `main` and can be changed with `--base <branch>`. A local branch is preferred when both a local branch and `origin/<branch>` exist.

## Navigation

- `Left` and `Right` move focus between the worktree, file, and diff panes
- `Up` and `Down` change the selection in the focused pane or scroll the diff
- `h`, `j`, `k`, and `l` provide equivalent navigation
- `Page Up`, `Page Down`, `Home`, and `End` move through large diffs
- `Tab` switches between Uncommitted and Branch modes
- `v` switches the diff pane between Hunks and Full File views
- `Enter` expands or collapses the selected diff hunk
- `r` refreshes worktrees and Git state
- `?` opens keyboard help
- `q` quits

The initial version is read-only and does not stage, discard, commit, or otherwise modify repository state.
