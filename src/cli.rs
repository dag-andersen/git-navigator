use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RenderMode {
    Uncommitted,
    Branch,
}

#[derive(Debug, Parser)]
#[command(
    name = "git-navigator",
    version,
    about = "Inspect changes across Git worktrees"
)]
pub struct Cli {
    /// Directory inside the repository to inspect
    pub directory: PathBuf,

    /// Base branch used by Branch mode
    #[arg(long, default_value = "main")]
    pub base: String,

    /// Render a deterministic text snapshot instead of opening the interactive TUI
    #[arg(long)]
    pub render: bool,

    /// Render the History panel, including its selected commit and markers
    #[arg(long)]
    pub history: bool,

    /// Comparison mode used by a rendered snapshot
    #[arg(long, value_enum, default_value_t = RenderMode::Uncommitted)]
    pub mode: RenderMode,

    /// Commit hash or short hash to select in a rendered History panel
    #[arg(long)]
    pub commit: Option<String>,

    /// Snapshot width in terminal cells
    #[arg(long, default_value_t = 120)]
    pub width: u16,

    /// Snapshot height in terminal cells
    #[arg(long, default_value_t = 40)]
    pub height: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_directory_as_the_first_argument() {
        let cli = Cli::try_parse_from(["git-navigator", "."])
            .expect("the current directory should be accepted");
        assert_eq!(cli.directory, PathBuf::from("."));
        assert_eq!(cli.base, "main");
        assert!(!cli.render);
        assert_eq!(cli.mode, RenderMode::Uncommitted);
    }

    #[test]
    fn accepts_a_custom_base_branch() {
        let cli = Cli::try_parse_from(["git-navigator", ".", "--base", "trunk"])
            .expect("a custom base should be accepted");
        assert_eq!(cli.base, "trunk");
    }

    #[test]
    fn accepts_render_snapshot_options() {
        let cli = Cli::try_parse_from([
            "git-navigator",
            ".",
            "--render",
            "--history",
            "--mode",
            "branch",
            "--commit",
            "abc123",
            "--width",
            "100",
            "--height",
            "30",
        ])
        .expect("render options should be accepted");

        assert!(cli.render);
        assert!(cli.history);
        assert_eq!(cli.mode, RenderMode::Branch);
        assert_eq!(cli.commit.as_deref(), Some("abc123"));
        assert_eq!(cli.width, 100);
        assert_eq!(cli.height, 30);
    }
}
