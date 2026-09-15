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

Directories in the tree are visual grouping rows and cannot be selected. `Up` and `Down` jump directly between changed files. The diff pane occupies 60 percent of the terminal and is scrollable. Press `s` to switch between a side-by-side Split layout and a single-column Unified layout where deleted lines appear in red above added lines in green. Added, untracked, and deleted files always use the full-width Unified layout because only one side of the comparison exists. This automatic layout does not change the user's preference for modified files. Press `v` to switch between a concise view containing Git diff hunks with surrounding context and a full-file view containing every line of the changed file. Long lines do not wrap by default; press `w` to toggle wrapping. All layouts include old and new line numbers.

Press `Space` to expand the focused panel across most of the content area. The other two panels remain visible as narrow rails showing one dot per logical item and an arrow beside the selected item. While expanded, `Left` and `Right` switch to the adjacent panel and keep the newly focused panel expanded. Press `Space` again to restore the three-column layout. Terminals narrower than 120 columns start in expanded mode with Unified diff layout. Wider terminals start with Split diff layout. This is decided once at startup, so manual `Space` and `s` toggles are not overridden afterward.

Press `t` to toggle between the default Columns layout and a Stacked layout. In Stacked layout, Worktrees appears above Files in a 25 percent sidebar and Diff uses the remaining 75 percent of the terminal width. Expanded mode is independent of this preference and keeps using compact rails for inactive panels.

## Worktrees

Discover and list all Git worktrees associated with the current repository. Each entry should show enough context to identify it, including its directory, current branch, and dirty state. Selecting a worktree updates the file and diff panes.

The app is intended to make worktrees created by developers or AI agents quick to inspect without changing directories.

Missing or disconnected worktrees are marked `MISSING` instead of producing a Git diff error. With the Worktrees pane focused, press `d` to clean up the selected worktree after confirming the action. For a worktree whose `.git` link is missing, cleanup identifies the exact administrative entry whose recorded path matches the selected worktree, then removes only that stale Git metadata. Any remaining directory and files are left untouched. For an existing clean linked worktree, cleanup removes its directory and Git metadata but leaves its branch intact. The current worktree, main worktree, locked worktrees, and dirty worktrees are protected from removal.

## Automatic refresh

Git Navigator watches the selected worktree and the repository's Git metadata for changes. File creation, modification, deletion, staging, commits, and worktree metadata changes trigger an automatic refresh after a 500 millisecond debounce. Refreshes are limited to at most once per second, with a two-second maximum delay during continuous activity. A reconciliation refresh runs every 30 seconds in case the operating system coalesces or misses an event.

Only the selected worktree is watched recursively. Switching worktrees updates the watcher and refreshes that worktree immediately. Automatic refresh preserves the selected worktree, selected file, diff position, scroll offset, and collapsed hunks when the corresponding content still exists. Press `r` at any time for an immediate manual refresh.

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
- Files containing both additions and deletions are orange in the file list

The base branch defaults to `main` and can be changed with `--base <branch>`. A local branch is preferred when both a local branch and `origin/<branch>` exist.

## Navigation

- `Left` and `Right` move focus between the worktree, file, and diff panes
- `Up` and `Down` change the selection in the focused pane or scroll the diff
- `h`, `j`, `k`, and `l` provide equivalent navigation
- `Page Up`, `Page Down`, `Home`, and `End` move through large diffs
- `Tab` switches between Uncommitted and Branch modes
- `Space` expands the focused panel or restores the three-column layout
- `t` toggles between Columns and Stacked panel layouts
- `v` switches the diff pane between Hunks and Full File views
- `s` switches the diff panel between Split and Unified layouts
- `w` toggles wrapping of long lines in the diff pane
- `Enter` expands or collapses the selected diff hunk
- `r` refreshes worktrees and Git state
- `d` cleans up the selected non-current worktree after confirmation
- `?` opens keyboard help
- `q` quits

Git inspection is read-only. Worktree cleanup is the only modifying action and always requires confirmation.
