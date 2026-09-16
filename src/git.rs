use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::model::{
    ChangeMode, ChangedFile, Commit, DiffHunk, DiffRow, DiffRowKind, DiffView, FileStatus,
    HunkKind, Worktree,
};

const HUNK_CONTEXT: &str = "--unified=3";
const FULL_FILE_CONTEXT: &str = "--unified=2147483647";

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

pub fn common_git_dir(directory: &Path) -> Result<PathBuf> {
    let path = git_text(
        directory,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(path.trim()))
}

pub fn commit_history(worktree: &Path) -> Result<Vec<Commit>> {
    let output = git_text(
        worktree,
        &["log", "--format=%H%x00%h%x00%s", "--max-count=100"],
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            Some(Commit {
                hash: fields.next()?.to_string(),
                short_hash: fields.next()?.to_string(),
                subject: fields.next()?.to_string(),
            })
        })
        .collect())
}

pub fn history_base_commit(worktree: &Path, base: &str) -> Result<String> {
    let base_ref = resolve_base_ref(worktree, base)?;
    Ok(git_text(worktree, &["merge-base", "HEAD", &base_ref])?
        .trim()
        .to_string())
}

pub fn load_commit_changes(
    worktree: &Path,
    commit: &Commit,
    view: DiffView,
    mode: ChangeMode,
    base: &str,
) -> Result<Vec<ChangedFile>> {
    let start = match mode {
        ChangeMode::Uncommitted => git_text(worktree, &["rev-parse", &format!("{}^", commit.hash)])
            .map(|parent| parent.trim().to_string())
            .unwrap_or_else(|_| "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()),
        ChangeMode::Branch => {
            let base_ref = resolve_base_ref(worktree, base)?;
            git_text(worktree, &["merge-base", &commit.hash, &base_ref])?
                .trim()
                .to_string()
        }
    };
    let context = if view == DiffView::FullFile {
        FULL_FILE_CONTEXT
    } else {
        HUNK_CONTEXT
    };
    let patch = git_text(
        worktree,
        &[
            "diff",
            "--patch",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "--find-renames",
            context,
            &start,
            &commit.hash,
            "--",
        ],
    )?;
    let kind = if view == DiffView::FullFile {
        HunkKind::FullFile
    } else {
        HunkKind::Combined
    };
    Ok(parse_diff(&patch, kind))
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

pub fn load_changes(
    worktree: &Path,
    mode: ChangeMode,
    view: DiffView,
    base: &str,
) -> Result<Vec<ChangedFile>> {
    if view == DiffView::FullFile {
        return load_full_file_changes(worktree, mode, base);
    }

    let mut files = match mode {
        ChangeMode::Uncommitted => {
            let staged = git_text(
                worktree,
                &[
                    "diff",
                    "--cached",
                    "--patch",
                    "--no-ext-diff",
                    "--no-color",
                    "--no-textconv",
                    "--find-renames",
                    HUNK_CONTEXT,
                    "--",
                ],
            )?;
            let unstaged = git_text(
                worktree,
                &[
                    "diff",
                    "--patch",
                    "--no-ext-diff",
                    "--no-color",
                    "--no-textconv",
                    "--find-renames",
                    HUNK_CONTEXT,
                    "--",
                ],
            )?;
            let mut files = parse_diff(&staged, HunkKind::Staged);
            merge_files(&mut files, parse_diff(&unstaged, HunkKind::Unstaged));
            files
        }
        ChangeMode::Branch => {
            let base_ref = resolve_base_ref(worktree, base)?;
            let merge_base = git_text(worktree, &["merge-base", "HEAD", &base_ref])?;
            let merge_base = merge_base.trim();
            let patch = git_text(
                worktree,
                &[
                    "diff",
                    "--patch",
                    "--no-ext-diff",
                    "--no-color",
                    "--no-textconv",
                    "--find-renames",
                    HUNK_CONTEXT,
                    merge_base,
                    "--",
                ],
            )?;
            parse_diff(&patch, HunkKind::Combined)
        }
    };

    merge_files(&mut files, load_untracked_files(worktree)?);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn load_full_file_changes(
    worktree: &Path,
    mode: ChangeMode,
    base: &str,
) -> Result<Vec<ChangedFile>> {
    let start = match mode {
        ChangeMode::Uncommitted => "HEAD".to_string(),
        ChangeMode::Branch => {
            let base_ref = resolve_base_ref(worktree, base)?;
            git_text(worktree, &["merge-base", "HEAD", &base_ref])?
                .trim()
                .to_string()
        }
    };
    let patch = git_text(
        worktree,
        &[
            "diff",
            "--patch",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "--find-renames",
            FULL_FILE_CONTEXT,
            &start,
            "--",
        ],
    )?;
    let mut files = parse_diff(&patch, HunkKind::FullFile);
    merge_files(&mut files, load_untracked_files(worktree)?);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
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

fn resolve_base_ref(worktree: &Path, base: &str) -> Result<String> {
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

fn load_untracked_files(worktree: &Path) -> Result<Vec<ChangedFile>> {
    let output = git(
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    let mut files = Vec::new();

    for path_bytes in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|p| !p.is_empty())
    {
        let relative = path_from_git_bytes(path_bytes);
        let absolute = worktree.join(&relative);
        let Ok(contents) = fs::read(&absolute) else {
            continue;
        };
        let mut file = ChangedFile::empty(relative, FileStatus::Untracked);

        if contents.contains(&0) {
            file.binary = true;
            files.push(file);
            continue;
        }

        let Ok(text) = String::from_utf8(contents) else {
            file.binary = true;
            files.push(file);
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        file.additions = lines.len();
        if !lines.is_empty() {
            file.hunks.push(DiffHunk {
                header: format!("@@ -0,0 +1,{} @@", lines.len()),
                kind: HunkKind::Untracked,
                collapsed: false,
                rows: lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| DiffRow {
                        old_number: None,
                        new_number: Some(index + 1),
                        old_text: None,
                        new_text: Some(line.to_string()),
                        kind: DiffRowKind::Added,
                    })
                    .collect(),
            });
        }
        files.push(file);
    }

    Ok(files)
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn merge_files(target: &mut Vec<ChangedFile>, incoming: Vec<ChangedFile>) {
    for mut file in incoming {
        if let Some(existing) = target
            .iter_mut()
            .find(|candidate| candidate.path == file.path)
        {
            existing.additions += file.additions;
            existing.deletions += file.deletions;
            existing.binary |= file.binary;
            existing.hunks.append(&mut file.hunks);
            if existing.old_path.is_none() {
                existing.old_path = file.old_path;
            }
            existing.status = merge_status(existing.status, file.status);
        } else {
            target.push(file);
        }
    }
}

fn merge_status(left: FileStatus, right: FileStatus) -> FileStatus {
    use FileStatus::{Added, Conflicted, Deleted, Modified, Renamed, Untracked};
    match (left, right) {
        (Conflicted, _) | (_, Conflicted) => Conflicted,
        (Untracked, _) | (_, Untracked) => Untracked,
        (Added, _) | (_, Added) => Added,
        (Deleted, _) | (_, Deleted) => Deleted,
        (Renamed, _) | (_, Renamed) => Renamed,
        _ => Modified,
    }
}

fn parse_diff(patch: &str, kind: HunkKind) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    let mut file: Option<FileBuilder> = None;
    let mut hunk: Option<HunkBuilder> = None;

    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            flush_hunk(&mut file, &mut hunk);
            flush_file(&mut files, &mut file);
            file = Some(FileBuilder::new(path_from_diff_header(line)));
            continue;
        }

        let Some(current_file) = file.as_mut() else {
            continue;
        };

        if line.starts_with("new file mode ") {
            current_file.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode ") {
            current_file.status = FileStatus::Deleted;
        } else if let Some(path) = line.strip_prefix("rename from ") {
            current_file.old_path = Some(PathBuf::from(unquote_git_path(path)));
            current_file.status = FileStatus::Renamed;
        } else if let Some(path) = line.strip_prefix("rename to ") {
            current_file.path = PathBuf::from(unquote_git_path(path));
            current_file.status = FileStatus::Renamed;
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            current_file.binary = true;
        } else if let Some(path) = line.strip_prefix("--- ") {
            if path != "/dev/null" {
                let old_path = parse_patch_path(path);
                if current_file.path.as_os_str().is_empty() {
                    current_file.path.clone_from(&old_path);
                }
                current_file.old_path = Some(old_path);
            }
        } else if let Some(path) = line.strip_prefix("+++ ") {
            if path != "/dev/null" {
                current_file.path = parse_patch_path(path);
            }
        } else if line.starts_with("@@ ") {
            flush_hunk(&mut file, &mut hunk);
            if let Some((old_line, new_line)) = parse_hunk_header(line) {
                hunk = Some(HunkBuilder::new(line.to_string(), kind, old_line, new_line));
            }
        } else if let Some(current_hunk) = hunk.as_mut() {
            if line == "\\ No newline at end of file" {
                continue;
            }
            if let Some(text) = line.strip_prefix(' ') {
                current_hunk.push_context(text);
            } else if let Some(text) = line.strip_prefix('-') {
                current_hunk.push_deleted(text);
            } else if let Some(text) = line.strip_prefix('+') {
                current_hunk.push_added(text);
            }
        }
    }

    flush_hunk(&mut file, &mut hunk);
    flush_file(&mut files, &mut file);
    files
}

fn flush_hunk(file: &mut Option<FileBuilder>, hunk: &mut Option<HunkBuilder>) {
    if let (Some(file), Some(hunk)) = (file.as_mut(), hunk.take()) {
        let hunk = hunk.finish();
        file.additions += hunk
            .rows
            .iter()
            .filter(|row| matches!(row.kind, DiffRowKind::Added | DiffRowKind::Modified))
            .count();
        file.deletions += hunk
            .rows
            .iter()
            .filter(|row| matches!(row.kind, DiffRowKind::Deleted | DiffRowKind::Modified))
            .count();
        file.hunks.push(hunk);
    }
}

fn flush_file(files: &mut Vec<ChangedFile>, file: &mut Option<FileBuilder>) {
    if let Some(file) = file.take()
        && !file.path.as_os_str().is_empty()
    {
        files.push(file.finish());
    }
}

struct FileBuilder {
    path: PathBuf,
    old_path: Option<PathBuf>,
    status: FileStatus,
    additions: usize,
    deletions: usize,
    hunks: Vec<DiffHunk>,
    binary: bool,
}

impl FileBuilder {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            old_path: None,
            status: FileStatus::Modified,
            additions: 0,
            deletions: 0,
            hunks: Vec::new(),
            binary: false,
        }
    }

    fn finish(self) -> ChangedFile {
        ChangedFile {
            path: self.path,
            old_path: self.old_path,
            status: self.status,
            additions: self.additions,
            deletions: self.deletions,
            hunks: self.hunks,
            binary: self.binary,
        }
    }
}

struct HunkBuilder {
    header: String,
    kind: HunkKind,
    old_line: usize,
    new_line: usize,
    rows: Vec<DiffRow>,
    deleted: Vec<(usize, String)>,
    added: Vec<(usize, String)>,
}

impl HunkBuilder {
    fn new(header: String, kind: HunkKind, old_line: usize, new_line: usize) -> Self {
        Self {
            header,
            kind,
            old_line,
            new_line,
            rows: Vec::new(),
            deleted: Vec::new(),
            added: Vec::new(),
        }
    }

    fn push_context(&mut self, text: &str) {
        self.flush_changes();
        self.rows.push(DiffRow {
            old_number: Some(self.old_line),
            new_number: Some(self.new_line),
            old_text: Some(text.to_string()),
            new_text: Some(text.to_string()),
            kind: DiffRowKind::Context,
        });
        self.old_line += 1;
        self.new_line += 1;
    }

    fn push_deleted(&mut self, text: &str) {
        self.deleted.push((self.old_line, text.to_string()));
        self.old_line += 1;
    }

    fn push_added(&mut self, text: &str) {
        self.added.push((self.new_line, text.to_string()));
        self.new_line += 1;
    }

    fn flush_changes(&mut self) {
        let count = self.deleted.len().max(self.added.len());
        for index in 0..count {
            let deleted = self.deleted.get(index);
            let added = self.added.get(index);
            self.rows.push(DiffRow {
                old_number: deleted.map(|(number, _)| *number),
                new_number: added.map(|(number, _)| *number),
                old_text: deleted.map(|(_, text)| text.clone()),
                new_text: added.map(|(_, text)| text.clone()),
                kind: match (deleted, added) {
                    (Some(_), Some(_)) => DiffRowKind::Modified,
                    (Some(_), None) => DiffRowKind::Deleted,
                    (None, Some(_)) => DiffRowKind::Added,
                    (None, None) => unreachable!(),
                },
            });
        }
        self.deleted.clear();
        self.added.clear();
    }

    fn finish(mut self) -> DiffHunk {
        self.flush_changes();
        DiffHunk {
            header: self.header,
            kind: self.kind,
            rows: self.rows,
            collapsed: false,
        }
    }
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    let mut fields = header.split_whitespace();
    (fields.next()? == "@@").then_some(())?;
    let old = parse_range_start(fields.next()?, '-')?;
    let new = parse_range_start(fields.next()?, '+')?;
    Some((old, new))
}

fn parse_range_start(range: &str, prefix: char) -> Option<usize> {
    range.strip_prefix(prefix)?.split(',').next()?.parse().ok()
}

fn path_from_diff_header(line: &str) -> PathBuf {
    let rest = line.strip_prefix("diff --git ").unwrap_or_default();
    if let Some(position) = rest.rfind(" b/") {
        return PathBuf::from(unquote_git_path(&rest[position + 1..]));
    }
    PathBuf::new()
}

fn parse_patch_path(path: &str) -> PathBuf {
    let path = unquote_git_path(path);
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(&path);
    PathBuf::from(path)
}

fn unquote_git_path(path: &str) -> String {
    let path = path.trim();
    let Some(inner) = path
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return path
            .strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path)
            .to_string();
    };

    let mut decoded = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => decoded.push('\n'),
            Some('t') => decoded.push('\t'),
            Some('r') => decoded.push('\r'),
            Some('\\') => decoded.push('\\'),
            Some('"') => decoded.push('"'),
            Some(first @ '0'..='7') => {
                let mut octal = String::from(first);
                for _ in 0..2 {
                    if matches!(chars.peek(), Some('0'..='7')) {
                        octal.push(chars.next().expect("peeked character must exist"));
                    }
                }
                if let Ok(value) = u8::from_str_radix(&octal, 8) {
                    decoded.push(char::from(value));
                }
            }
            Some(other) => decoded.push(other),
            None => decoded.push('\\'),
        }
    }
    decoded
        .strip_prefix("a/")
        .or_else(|| decoded.strip_prefix("b/"))
        .unwrap_or(&decoded)
        .to_string()
}

fn canonicalize_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn git_text(directory: &Path, args: &[&str]) -> Result<String> {
    let output = git(directory, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    let output = git_allow_failure(directory, args)?;
    ensure_git_success(output, directory, &format!("git {}", args.join(" ")))
}

fn ensure_git_success(output: Output, directory: &Path, operation: &str) -> Result<Output> {
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = if message.trim().is_empty() {
            format!("process exited with {}", output.status)
        } else {
            message.trim().to_string()
        };
        return Err(anyhow!(message))
            .with_context(|| format!("{operation} failed in {}", directory.display()));
    }
    Ok(output)
}

fn git_allow_failure(directory: &Path, args: &[&str]) -> Result<Output> {
    git_command(directory)
        .args(args.iter().map(OsStr::new))
        .output()
        .with_context(|| format!("failed to run git in {}", directory.display()))
}

fn git_command(directory: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--no-optional-locks")
        .arg("-c")
        .arg("core.quotePath=false")
        .arg("-c")
        .arg("color.ui=false")
        .arg("-C")
        .arg(directory.as_os_str())
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C");
    command
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn parses_side_by_side_rows_and_line_numbers() {
        let patch = r#"diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,5 @@
 context
-old one
-old two
+new one
+new two
+new three
 context two
"#;

        let files = parse_diff(patch, HunkKind::Staged);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(files[0].additions, 3);
        assert_eq!(files[0].deletions, 2);
        assert_eq!(files[0].hunks[0].kind, HunkKind::Staged);
        assert_eq!(files[0].hunks[0].rows[1].kind, DiffRowKind::Modified);
        assert_eq!(files[0].hunks[0].rows[3].kind, DiffRowKind::Added);
        assert_eq!(files[0].hunks[0].rows[3].new_number, Some(4));
    }

    #[test]
    fn parses_new_and_deleted_files() {
        let patch = r#"diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
diff --git a/old.txt b/old.txt
deleted file mode 100644
index 1111111..0000000
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-goodbye
"#;

        let files = parse_diff(patch, HunkKind::Combined);
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[0].path, PathBuf::from("new.txt"));
        assert_eq!(files[1].status, FileStatus::Deleted);
        assert_eq!(files[1].path, PathBuf::from("old.txt"));
    }

    #[test]
    fn parses_hunk_ranges_with_and_without_counts() {
        assert_eq!(parse_hunk_header("@@ -12 +15 @@"), Some((12, 15)));
        assert_eq!(parse_hunk_header("@@ -2,3 +4,8 @@ name"), Some((2, 4)));
    }

    #[test]
    fn preserves_a_deleted_path_containing_spaces() {
        let patch = r#"diff --git "a/file with spaces.txt" "b/file with spaces.txt"
deleted file mode 100644
index 1111111..0000000
--- "a/file with spaces.txt"
+++ /dev/null
@@ -1 +0,0 @@
-goodbye
"#;

        let files = parse_diff(patch, HunkKind::Combined);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("file with spaces.txt"));
    }

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

    #[test]
    fn loads_staged_unstaged_and_untracked_changes() {
        let repository = TestRepository::new();
        fs::write(repository.path().join("tracked.txt"), "changed\n")
            .expect("tracked file should be writable");
        run_git(repository.path(), &["add", "tracked.txt"]);
        fs::write(repository.path().join("tracked.txt"), "changed\nworking\n")
            .expect("tracked file should be writable");
        fs::write(repository.path().join("new.txt"), "new\n")
            .expect("untracked file should be writable");

        let files = load_changes(
            repository.path(),
            ChangeMode::Uncommitted,
            DiffView::Hunks,
            "main",
        )
        .expect("changes should load");
        let tracked = files
            .iter()
            .find(|file| file.path == Path::new("tracked.txt"))
            .expect("tracked change should exist");
        assert!(
            tracked
                .hunks
                .iter()
                .any(|hunk| hunk.kind == HunkKind::Staged)
        );
        assert!(
            tracked
                .hunks
                .iter()
                .any(|hunk| hunk.kind == HunkKind::Unstaged)
        );
        assert!(files.iter().any(|file| {
            file.path == Path::new("new.txt") && file.status == FileStatus::Untracked
        }));
    }

    #[test]
    fn branch_mode_includes_committed_and_working_tree_changes() {
        let repository = TestRepository::new();
        run_git(repository.path(), &["switch", "-c", "feature"]);
        fs::write(repository.path().join("committed.txt"), "committed\n")
            .expect("committed file should be writable");
        run_git(repository.path(), &["add", "committed.txt"]);
        run_git(repository.path(), &["commit", "-m", "feature change"]);
        fs::write(repository.path().join("tracked.txt"), "working\n")
            .expect("tracked file should be writable");

        let files = load_changes(
            repository.path(),
            ChangeMode::Branch,
            DiffView::Hunks,
            "main",
        )
        .expect("branch changes should load");
        assert!(
            files
                .iter()
                .any(|file| file.path == Path::new("committed.txt"))
        );
        assert!(
            files
                .iter()
                .any(|file| file.path == Path::new("tracked.txt"))
        );
        assert!(files.iter().all(|file| {
            file.hunks
                .iter()
                .all(|hunk| hunk.kind == HunkKind::Combined)
        }));
    }

    #[test]
    fn full_file_view_includes_unchanged_lines_outside_hunk_context() {
        let repository = TestRepository::new();
        let original = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(repository.path().join("long.txt"), &original)
            .expect("long file should be writable");
        run_git(repository.path(), &["add", "long.txt"]);
        run_git(repository.path(), &["commit", "-m", "add long file"]);

        let changed = original.replace("line 10", "changed line 10");
        fs::write(repository.path().join("long.txt"), changed)
            .expect("long file should be writable");

        let hunks = load_changes(
            repository.path(),
            ChangeMode::Uncommitted,
            DiffView::Hunks,
            "main",
        )
        .expect("hunk changes should load");
        let full_file = load_changes(
            repository.path(),
            ChangeMode::Uncommitted,
            DiffView::FullFile,
            "main",
        )
        .expect("full-file changes should load");

        assert!(hunks[0].hunks[0].rows.len() < 20);
        assert_eq!(full_file[0].hunks[0].rows.len(), 20);
        assert_eq!(full_file[0].hunks[0].kind, HunkKind::FullFile);
        assert_eq!(
            full_file[0].hunks[0].rows[0].old_text.as_deref(),
            Some("line 1")
        );
        assert_eq!(
            full_file[0].hunks[0].rows[19].new_text.as_deref(),
            Some("line 20")
        );
    }

    struct TestRepository {
        root: TempDir,
        repository: PathBuf,
    }

    impl TestRepository {
        fn new() -> Self {
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

        fn path(&self) -> &Path {
            &self.repository
        }
    }

    fn run_git(directory: &Path, args: &[&str]) {
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

    fn path_text(path: &Path) -> &str {
        path.to_str().expect("temporary path should be UTF-8")
    }
}
