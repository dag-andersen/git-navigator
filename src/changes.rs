use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::model::{
    ChangeMode, ChangedFile, Commit, DiffHunk, DiffRow, DiffRowKind, DiffView, FileStatus, HunkId,
    HunkKind,
};

use super::{
    history::resolve_base_ref,
    process::{git, git_text},
};

const HUNK_CONTEXT: &str = "--unified=3";
const FULL_FILE_CONTEXT: &str = "--unified=2147483647";

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
                id: HunkId::synthetic(format!("{}:untracked", file.path.display())),
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
    assign_hunk_ids(&mut files);
    files
}

fn assign_hunk_ids(files: &mut [ChangedFile]) {
    for file in files {
        let mut occurrences = HashMap::<String, usize>::new();
        for hunk in &mut file.hunks {
            let fingerprint = format!(
                "{}:{}",
                hunk.kind.label(),
                hunk.rows
                    .iter()
                    .map(|row| {
                        format!(
                            "{:?}|{}|{}",
                            row.kind,
                            row.old_text.as_deref().unwrap_or_default(),
                            row.new_text.as_deref().unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\u{1f}"),
            );
            let occurrence = occurrences.entry(fingerprint.clone()).or_default();
            hunk.id = HunkId::synthetic(format!("{}:{}", fingerprint, *occurrence));
            *occurrence += 1;
        }
    }
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
            id: HunkId::synthetic("parsed"),
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use super::*;
    use crate::{
        git::test_support::{TestRepository, run_git},
        model::FileStatus,
    };

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
    fn assigns_distinct_ids_to_duplicate_hunks_and_preserves_change_kind() {
        let patch = r#"diff --git a/file.txt b/file.txt
index 1111111..2222222 100644
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-old
+new
@@ -1 +1 @@
-old
+new
"#;

        let staged = parse_diff(patch, HunkKind::Staged);
        assert_eq!(staged[0].hunks.len(), 2);
        assert_ne!(staged[0].hunks[0].id, staged[0].hunks[1].id);
        assert_eq!(staged[0].hunks[0].kind, HunkKind::Staged);

        let mut merged = staged;
        merge_files(&mut merged, parse_diff(patch, HunkKind::Unstaged));
        assert_eq!(merged[0].hunks.len(), 4);
        assert_eq!(merged[0].hunks[2].kind, HunkKind::Unstaged);
        assert_eq!(merged[0].hunks[3].kind, HunkKind::Unstaged);
        assert_ne!(merged[0].hunks[0].id, merged[0].hunks[2].id);
        assert_ne!(merged[0].hunks[1].id, merged[0].hunks[3].id);
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
}
