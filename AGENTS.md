# git-navigator development instructions

This repository builds the `git-navigator` binary installed in the global
Cargo bin directory.

## Hard rules

- Keep `main` checked out in the primary worktree. Use that worktree only for
  integration.
- Never implement, edit, or commit feature work on `main`.
- Every task must use its own feature branch and linked worktree.
- Each feature branch has exactly one linked worktree. Store it at
  `<repo-name>-worktrees/<branch-name>`, next to the repository, using the
  exact branch name as the worktree directory. The repository files must be
  checked out directly in that directory. Do not add a literal `code`
  directory. Preserve slashes in the branch name as directories. No particular
  branch naming prefix is required.
- Commit every completed task before reporting completion. Do not leave
  completed work only in the working tree.
- Feature branch commits and pushes are allowed. Never push `main`, `master`,
  `develop`, or `production` without explicit user permission.
- Do not create a pull request or merge into `main` unless explicitly asked.
- Do not modify unrelated files. Preserve unrelated changes and untracked files,
  especially `test.txt`.
- Do not use force push, hard reset, rebase, or cleanup commands without explicit
  permission.
- Do not edit another agent's worktree or branch.
- Use a regular hyphen instead of an em dash in prose, comments, and commits.

## Required verification

Run the complete workflow from the repository root after every task that changes
repository files:

```shell
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo install --path . --force
codesign --force --sign - "$HOME/.cargo/bin/git-navigator"
git-navigator --help
```

The install must update:

```text
$HOME/.cargo/bin/git-navigator
```

If any command fails, stop and report the failure. Do not report completion or
claim that the global binary was updated.

## Starting a task

Always start from an up-to-date local `main` in the primary worktree:

```shell
git switch main
git pull --ff-only
repo_name="$(basename "$PWD")"
branch="<feature-branch-name>"
worktree="../${repo_name}-worktrees/${branch}"
mkdir -p "$(dirname "$worktree")"
git worktree add -b "$branch" "$worktree" main
```

If the primary worktree is not clean, stop and preserve the existing changes.
Do not stash, overwrite, or discard them. If the branch or worktree already
exists, inspect it and reuse it only when continuing that same task.

Perform all implementation work in the new worktree. When the task is complete:

1. Commit only the files related to the task.
2. Run the complete required verification workflow in the feature worktree.
3. Push the feature branch if needed.
4. Report the branch, worktree, commit, and verification result.
5. Wait for an explicit request before merging into `main`.

## Merging into main

An explicit request to merge authorizes the integration commit, but not a push
to `main`. Use a squash merge by default unless the user requests a regular
merge commit. Never use a fast-forward merge.

Before changing `main`:

1. Confirm the feature worktree and primary worktree are clean.
2. Update `main` in the primary worktree:

   ```shell
   git switch main
   git pull --ff-only
   ```

3. In the feature worktree, merge the updated local `main` into the feature
   branch:

   ```shell
   git merge main
   ```

4. Resolve conflicts in the feature worktree, review the result, and commit the
   merge if Git requires it.
5. Run the complete required verification workflow again in the feature
   worktree. Stop if any command fails.
6. Confirm the feature worktree is clean.

Then integrate from the primary worktree:

```shell
git switch main
git merge --squash <feature-branch-name>
git commit -m "Add <feature>"
```

Run the complete required verification workflow again on the resulting `main`.
If it fails, stop, report the failure, and do not push `main`.

Do not remove the feature worktree or branch unless the user explicitly asks
for cleanup. Do not run integrations into `main` concurrently.

## Completion report

Report:

```text
Branch: <feature-branch-name>
Worktree: /absolute/path/to/worktree
Commit: <commit>
Tests and linting: passed or failed
Release installation: passed or failed
macOS code signing: passed or failed
git-navigator --help: passed or failed
Conflicts with current main: known or none
```
