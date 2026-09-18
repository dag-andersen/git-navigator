use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ChangeMode {
    #[default]
    Uncommitted,
    Branch,
}

impl ChangeMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Uncommitted => Self::Branch,
            Self::Branch => Self::Uncommitted,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiffView {
    #[default]
    Hunks,
    FullFile,
}

impl DiffView {
    pub fn toggle(self) -> Self {
        match self {
            Self::Hunks => Self::FullFile,
            Self::FullFile => Self::Hunks,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hunks => "HUNKS",
            Self::FullFile => "FULL FILE",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiffLayout {
    #[default]
    Split,
    Unified,
}

impl DiffLayout {
    pub fn toggle(self) -> Self {
        match self {
            Self::Split => Self::Unified,
            Self::Unified => Self::Split,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Split => "SPLIT",
            Self::Unified => "UNIFIED",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub head: String,
    pub dirty: bool,
    pub is_current: bool,
    pub is_main: bool,
    pub available: bool,
    pub prunable_reason: Option<String>,
    pub locked_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub short_hash: String,
    pub subject: String,
    pub graph: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistorySelection {
    Wip,
    Commit { hash: String },
}

impl Worktree {
    pub fn is_missing(&self) -> bool {
        !self.available || self.prunable_reason.is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub status: FileStatus,
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<DiffHunk>,
    pub binary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileTreeRow {
    pub path: PathBuf,
    pub kind: FileTreeRowKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileTreeRowKind {
    Directory,
    File,
}

impl FileTreeRow {
    pub fn is_directory(&self) -> bool {
        self.kind == FileTreeRowKind::Directory
    }
}

impl ChangedFile {
    pub fn empty(path: PathBuf, status: FileStatus) -> Self {
        Self {
            path,
            old_path: None,
            status,
            additions: 0,
            deletions: 0,
            hunks: Vec::new(),
            binary: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HunkKind {
    Staged,
    Unstaged,
    Combined,
    FullFile,
    Untracked,
}

impl HunkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "STAGED",
            Self::Unstaged => "UNSTAGED",
            Self::Combined => "BRANCH",
            Self::FullFile => "FULL FILE",
            Self::Untracked => "UNTRACKED",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffHunk {
    pub id: HunkId,
    pub header: String,
    pub kind: HunkKind,
    pub rows: Vec<DiffRow>,
    pub collapsed: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HunkId(pub String);

impl HunkId {
    pub fn synthetic(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRow {
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub kind: DiffRowKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffRowKind {
    Context,
    Added,
    Deleted,
    Modified,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_diff_views() {
        assert_eq!(DiffView::Hunks.toggle(), DiffView::FullFile);
        assert_eq!(DiffView::FullFile.toggle(), DiffView::Hunks);
    }

    #[test]
    fn toggles_diff_layouts() {
        assert_eq!(DiffLayout::Split.toggle(), DiffLayout::Unified);
        assert_eq!(DiffLayout::Unified.toggle(), DiffLayout::Split);
    }
}
