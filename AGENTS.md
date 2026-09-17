# git-navigator development instructions

This repository builds the `git-navigator` binary used from the global Cargo
bin directory. After completing every feature request, bugfix, behavior change,
or other implementation change in this repository, rebuild and install the
updated binary before reporting completion.

## Required verification and installation

Run these commands from the repository root after making changes:

```shell
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo install --path . --force
codesign --force --sign - "$HOME/.cargo/bin/git-navigator"
git-navigator --help
```

The install command must update the global binary at:

```text
$HOME/.cargo/bin/git-navigator
```

The ad-hoc code-signing step is required on macOS so the installed binary can
be launched normally. If any verification, build, installation, signing, or
smoke-test command fails, report the failure and do not claim that the global
binary was updated.

## Repository safeguards

- Do not commit, push, or create pull requests unless explicitly requested.
- When integrating a topic branch into `main`, never use a fast-forward merge. Use
  either a regular merge commit with `git merge --no-ff <branch>` or a squash
  merge with `git merge --squash <branch>` followed by `git commit`. If the user
  does not specify which style they want, ask before merging.
- Do not modify unrelated files.
- Preserve unrelated untracked files, especially `test.txt`.
- Do not remove or clean worktrees, generated files, or untracked files without
  explicit confirmation.
- Use a regular hyphen instead of an em dash in prose, comments, and commits.

## Multi-agent Git and worktree workflow

Use one worktree and one feature branch per agent. Worktrees share Git history
and branch references, but each worktree has its own checked-out files.

### Worktree ownership

- Reserve the primary worktree for integration and keep local `main` checked out
  there.
- Never check out local `main` in an agent worktree. Git normally prevents this,
  and that protection must not be bypassed.
- Each agent must work in its own linked worktree on its own branch.
- Do not edit files, switch branches, or run cleanup commands in another
  agent's worktree.
- Do not run merges into `main` concurrently. Branch pointers are shared across
  worktrees, so integration must be serialized.

Recommended layout:

```text
project/                  # integration worktree, local main
project-agent-a/          # agent/a/feature-name
project-agent-b/          # agent/b/feature-name
project-agent-c/          # agent/c/feature-name
```

### Starting an agent branch

Before creating a new agent worktree, update the integration worktree from the
remote using a fast-forward-only pull:

```shell
git switch main
git pull --ff-only
git worktree add -b agent/<name>/<feature> ../project-<name> main
```

If the branch already exists, inspect it before adding or reusing its worktree:

```shell
git worktree list
git status --short --branch
git log --oneline --decorate -10
```

Never create a feature branch from a stale local `main` when a newer remote
`main` is available.

### Agent branch rules

- Agents may commit changes only to their assigned feature branch.
- Agents must not commit directly to `main`.
- Agents must not merge, rebase, reset, or force-push another agent's branch
  without explicit authorization from the user. An explicit request to merge a
  completed feature into `main` authorizes the integration owner to perform the
  merge workflow below.
- Agents must not force-push any branch unless explicitly confirmed.
- Keep commits focused and do not include unrelated changes.
- Before handoff, run the repository's required verification commands.
- When explicitly moving uncommitted changes between the integration worktree
  and a feature worktree, preserve them in the destination before restoring or
  removing them from the source. No additional permission is required for that
  transfer.
- In a feature branch or feature worktree, no additional permission is required
  to delete uncommitted files created as part of the feature or to remove the
  feature worktree during authorized cleanup. Never delete committed files or
  another agent's files without explicit authorization.

When a feature is ready, report:

```text
Branch: agent/<name>/<feature>
Worktree: /absolute/path/to/worktree
Commit: <commit>
Tests and linting: passed or failed
Conflicts with current main: known or none
```

### Keeping active branches current

When another feature is merged into `main`, existing agent worktrees do not
update automatically. Their files and branch commits remain unchanged until
they explicitly synchronize.

Before integration, update an active feature branch with the latest `main` and
resolve conflicts in that feature worktree:

```shell
git fetch origin
git merge main
```

Use `git rebase main` only when explicitly requested. Do not rewrite a branch
that has already been shared without confirmation.

### Integrating a completed feature

Only the integration owner should merge a feature into `main`. When the user
explicitly asks to merge a completed feature into `main`, use a squash merge by
default unless the user requests a different non-fast-forward strategy.

Before each integration:

1. Confirm the feature worktree is clean and identify its branch and commits.
2. Confirm the integration worktree is clean except for known, preserved user
   changes.
3. Update local `main` with `git pull --ff-only`.
4. In the feature worktree, ensure the feature branch includes the current
   `main` and resolve any conflicts there. Use `git merge main`; use `git rebase
   main` only when explicitly requested.
5. Verify that the squash merge can apply cleanly before changing `main`, for
   example with `git merge-tree --write-tree main agent/<name>/<feature>`.
6. Run the required tests and linting in the feature worktree.
7. In the integration worktree, squash-merge the feature into `main` and create
   the commit.
8. Run the full verification and installation steps again from `main`.
9. After successful verification, remove the merged feature worktree and branch
   when the user's merge request explicitly includes cleanup permission. If it
   does not, ask for confirmation immediately before deleting them.

For a squash merge:

```shell
git switch main
git pull --ff-only
git merge --squash agent/<name>/<feature>
git commit -m "Add <feature>"
```

Before cleanup, inspect the final state and ensure the feature worktree is no
longer needed. Cleanup, when authorized, should use:

```shell
git worktree remove /absolute/path/to/worktree
git branch -d agent/<name>/<feature>
```

Never use `--force` for worktree or branch cleanup unless explicitly confirmed.

## Completion report

When reporting completed work, state whether the following succeeded:

- Tests and linting
- Release installation
- macOS code signing
- `git-navigator --help` smoke test
