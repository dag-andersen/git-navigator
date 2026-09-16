# git-navigator

`git-navigator` is a fast, read-only terminal interface for exploring Git worktrees, changed files, commit history, and diffs without repeatedly changing directories or running separate Git commands.

It is written in Rust with [Ratatui](https://ratatui.rs/).

## Features

- Discover every worktree associated with a repository
- Switch between worktrees and inspect their changes immediately
- Show changed files as a navigable folder tree
- Inspect staged, unstaged, untracked, committed, added, deleted, renamed, and conflicted files
- Compare the working tree with `HEAD` or with the branch merge base
- Browse recent commit history with a live WIP entry
- Inspect either a single commit or the complete change from the configured base branch to that commit
- View diffs in Split or Unified layouts
- Switch between concise Git hunks and full-file diffs
- Search Worktrees and Files with live fuzzy filtering
- Search Diff contents with case-insensitive substring matching
- Highlight every Diff search match and show the current match position
- Copy an absolute `path:line` location from the selected Diff row
- Automatically refresh when files, commits, or worktrees change
- Collapse and expand individual Diff hunks
- Wrap long Diff lines when needed
- Expand any panel while retaining compact navigation rails
- Cycle through multiple panel arrangements
- Safely clean up eligible worktrees after confirmation

## Installation

### Install from GitHub

Install Rust with [rustup](https://rustup.rs/) if it is not already available, then install `git-navigator`:

```shell
cargo install --git https://github.com/dag-andersen/git-navigator
```

Cargo normally installs the binary in `$HOME/.cargo/bin`. Make sure that directory is on your `PATH`.

### Install from source

```shell
git clone https://github.com/dag-andersen/git-navigator.git
cd git-navigator
cargo install --path .
```

Git must also be installed and available on `PATH`.

## Usage

Pass a directory anywhere inside the repository you want to inspect:

```shell
git-navigator .
```

The comparison base defaults to `main`. It can be changed with the base option when a repository uses another primary branch.

## Interface

The application is organized around three panels:

- **Worktrees or History** - select a worktree or browse its recent commits
- **Files** - navigate changed files in a folder tree
- **Diff** - inspect the selected file and line-level changes

Selecting an item updates the panels to its right. Directory rows in the Files tree are visual grouping rows and are skipped during keyboard navigation.

### Panel layouts

Press `t` to cycle through the available layouts.

#### Columns

```text
┌────────────┬────────────┬────────────────────────┐
│ Worktrees  │ Files      │ Diff                   │
└────────────┴────────────┴────────────────────────┘
```

#### SidebarLeft

```text
┌────────────┬─────────────────────────────────────┐
│ Worktrees  │                                     │
├────────────┤ Diff                                │
│ Files      │                                     │
└────────────┴─────────────────────────────────────┘
```

#### SidebarTop

```text
┌───────────────────────┬──────────────────────────┐
│ Worktrees             │ Files                    │
├───────────────────────┴──────────────────────────┤
│ Diff                                                │
└────────────────────────────────────────────────────┘
```

SidebarTop gives 25 percent of the height to Worktrees and Files and 75 percent to Diff.

If the repository has no linked worktrees, git-navigator opens directly in History mode with the History panel visible as the first column. The history belongs to the repository's primary worktree and starts with the live WIP entry. Layout cycling keeps the History panel visible in this mode and still cycles through all three layouts.

### Expanded panels

Press `Space` to expand the focused panel. Inactive panels remain visible as compact rails. Left and Right move between panels while keeping the focused panel expanded.

Pressing `t` while expanded cycles to the next layout and returns to the normal multi-panel view.

Terminals narrower than 120 columns initially use an expanded panel and Unified Diff. Wider terminals initially use the SidebarLeft layout and Split Diff. This automatic choice is applied only at startup.

## Worktrees

The Worktrees panel lists all worktrees registered with the repository. Each entry includes:

- Directory name
- Branch name
- Short commit hash
- Current-worktree marker
- Dirty state
- Missing or locked state

Selecting a worktree loads that worktree's Files and Diff.

The panel includes a scrollbar when all worktrees do not fit in the available height.

### Worktree cleanup

With Worktrees focused, press `d` to request cleanup of the selected worktree. Cleanup always requires confirmation.

The following worktrees are protected from removal:

- The currently opened worktree
- The primary worktree
- Dirty worktrees
- Locked worktrees

For an existing clean linked worktree, cleanup removes its directory and Git metadata but leaves its branch intact. For a missing worktree, cleanup removes only the matching stale Git administrative entry.

## Commit History

Press `h` while Worktrees or Files is focused to replace the Worktrees panel with Commit History. Press `h` again to return to Worktrees when linked worktrees exist. Repositories without linked worktrees open directly in History mode, so their first column is already the commit history rather than a collapsed or hidden Worktrees panel. History remains active in that case.

History contains up to 100 commits and starts with a virtual WIP entry:

```text
WIP       Uncommitted changes
736a5c4   Improve diff search and worktree layouts
9b42ffd   Add search and copy navigation features
```

The WIP entry shows the current live worktree changes. Moving down to a commit shows the Files and Diff belonging to that exact commit. Moving back to WIP restores the live view.

Git Navigator remembers the preferred file while moving between WIP and commits. If that file exists in a selected commit, it remains selected. If it does not exist, the first changed file is selected temporarily, while the preferred path remains remembered for later commits.

History refreshes through the normal automatic-refresh mechanism. WIP remains selected when new commits appear. Historical commit selection is preserved by full commit hash, so inserting a newer commit does not move the user to a different commit. If rewritten history removes the selected commit, selection falls back to WIP.

### Historical comparison modes

Press `Tab` while a historical commit is selected to switch between:

- **Commit** - compare the selected commit with its parent
- **Commit Range** - compare the merge base of the configured base branch and the selected commit with the selected commit

For a root commit, Commit mode compares against Git's empty tree.

When WIP is selected, `Tab` retains the normal Uncommitted and Branch behavior.

## Change modes

Press `Tab` to switch comparison mode.

### Uncommitted

Shows all local changes that have not been committed:

- Staged changes between `HEAD` and the index
- Unstaged changes between the index and working tree
- Untracked files as newly added files

Hunks are marked as staged or unstaged. A file can contain both kinds.

### Branch

Shows everything changed since the current branch diverged from the configured base branch. This includes committed, staged, unstaged, and untracked changes.

The base branch defaults to `main`. A local branch is preferred when both a local branch and a matching remote branch exist.

## Files

Changed paths are rendered as a folder tree. Directory rows cannot be selected, and Up or Down moves directly between files.

File colors communicate change shape:

- Green for additions
- Red for deletions
- Yellow for modified files in Uncommitted mode
- Orange for files with additions and deletions in Branch mode

The panel also displays file status and addition/deletion counts.

## Diff views

### Split and Unified

Press `s` to switch between:

- **Split** - before and after content shown side by side
- **Unified** - deleted lines followed by added lines in one full-width column

Added, deleted, and untracked files automatically use Unified view because only one side of the comparison exists. This does not change the saved preference used for modified files.

### Hunks and Full File

Press `v` to switch between:

- **Hunks** - changed regions with Git context lines
- **Full File** - the complete changed file

Press `Enter` to collapse or expand the selected hunk. Press `w` to toggle wrapping of long lines.

### Copy file location

With Diff focused, press `c` to copy the selected source location to the system clipboard:

```text
/absolute/path/to/worktree/src/app.rs:44
```

New-file line numbers are preferred. Deleted rows fall back to their old-file line number.

## Search

Press `/` in the focused panel to start searching. The active query appears in the panel title immediately.

### Worktrees and Files

Worktrees and Files use case-insensitive fuzzy subsequence matching.

- Worktrees search path, branch, and commit hash
- Files search full file paths
- Matching files retain all ancestor directories in the rendered tree
- Up and Down navigate filtered items
- Enter keeps the filter and exits search mode
- Esc clears the filter
- Left and Right exit search mode and move between panels

Applied filters remain visible in panel titles:

```text
Files (2/14) /parser/
```

### Diff

Diff search uses case-insensitive substring matching rather than fuzzy matching.

- Every matching substring receives a highlighted background
- The title shows the current and total match count
- Enter moves to the next match
- Up and Down move between matches
- Navigation wraps at the first and last match
- Esc clears the search

Example:

```text
Diff [HUNKS, SPLIT] - src/app.rs /error/ (3/12)
```

## Automatic refresh

Git Navigator watches the selected worktree and shared Git metadata.

- Relevant events are debounced for 500 milliseconds
- Refreshes are limited to at most once per second during activity
- Continuous activity is refreshed within two seconds
- A reconciliation refresh runs every 30 seconds
- Press `r` for an immediate refresh

Refresh attempts to preserve:

- Selected worktree
- Selected file
- Diff position and scroll offset
- Collapsed hunks
- WIP selection
- Historical commit selection by full commit hash

## Keyboard reference

| Key | Action |
| --- | --- |
| `Left`, `Right` | Move focus between panels |
| `Up`, `Down` | Navigate the focused panel |
| `j`, `k` | Move down or up outside search input |
| `Right` | Move focus right |
| `h` | Toggle History from Worktrees or Files, or move left from Diff |
| `Page Up`, `Page Down` | Move through the Diff by ten rows |
| `Home`, `End` | Jump to the beginning or end of the Diff |
| `Tab` | Switch comparison mode |
| `Space` | Expand or restore the focused panel |
| `t` | Cycle panel layout |
| `/` | Search the focused panel |
| `Enter` | Fold a Diff hunk, apply a list search, or move to the next Diff search match |
| `v` | Toggle Hunks and Full File views |
| `s` | Toggle Split and Unified Diff layouts |
| `w` | Toggle Diff line wrapping |
| `c` | Copy the selected absolute `path:line` from Diff |
| `r` | Refresh Git state |
| `d` | Clean up the selected eligible worktree after confirmation |
| `Esc` | Cancel search, clear an applied list filter, or close a dialog |
| `?` | Open keyboard help |
| `q` | Quit |

## Safety

Git inspection is read-only. Worktree cleanup is the only operation that changes repository or filesystem state, and it always requires explicit confirmation inside the application.
