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
- Do not modify unrelated files.
- Preserve unrelated untracked files, especially `test.txt`.
- Do not remove or clean worktrees, generated files, or untracked files without
  explicit confirmation.
- Use a regular hyphen instead of an em dash in prose, comments, and commits.

## Completion report

When reporting completed work, state whether the following succeeded:

- Tests and linting
- Release installation
- macOS code signing
- `git-navigator --help` smoke test
