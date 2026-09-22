use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RenderMode {
    Uncommitted,
    Branch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum StartupFocus {
    Worktrees,
    History,
    Files,
    Diff,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Send a command to a running git-navigator TUI
    Ctl {
        #[command(flatten)]
        control: ControlArgs,
    },
}

#[derive(Debug, Args)]
pub struct ControlArgs {
    /// Repository or worktree used to discover the target TUI
    #[arg(short, long)]
    pub directory: Option<PathBuf>,

    /// Session ID returned by `git-navigator ctl sessions`
    #[arg(short, long)]
    pub session: Option<String>,

    /// Connect directly to this Unix socket
    #[arg(long)]
    pub socket: Option<PathBuf>,

    #[command(subcommand)]
    pub command: ControlCommand,
}

#[derive(Debug, Subcommand)]
pub enum ControlCommand {
    /// List running git-navigator TUI sessions
    Sessions,
    /// Focus a panel without changing the current layout
    Focus { panel: StartupFocus },
    /// Expand the currently focused panel
    Expand,
    /// Restore the normal multi-panel layout
    Collapse,
    /// Select a worktree by its absolute path
    Worktree { path: PathBuf },
    /// Switch the TUI to another Git repository
    #[command(alias = "repo")]
    Repository { path: PathBuf },
    /// Select a changed file by its repository-relative path
    File { path: PathBuf },
    /// Select a commit by full or short hash
    Commit { hash: String },
    /// Refresh the displayed Git state
    Refresh,
}

#[derive(Debug, Parser)]
#[command(
    name = "git-navigator",
    version,
    about = "Inspect changes across Git worktrees"
)]
pub struct Cli {
    /// Directory inside the repository to inspect
    pub directory: Option<PathBuf>,

    /// Base branch used by Branch mode
    #[arg(long, default_value = "main")]
    pub base: String,

    /// Render a deterministic text snapshot instead of opening the interactive TUI
    #[arg(long)]
    pub render: bool,

    /// Preserve terminal colors and modifiers in rendered output
    #[arg(long)]
    pub ansi: bool,

    /// Start with this panel focused and expanded
    #[arg(long, value_enum)]
    pub focus: Option<StartupFocus>,

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

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_directory_as_the_first_argument() {
        let cli = Cli::try_parse_from(["git-navigator", "."])
            .expect("the current directory should be accepted");
        assert_eq!(cli.directory, Some(PathBuf::from(".")));
        assert_eq!(cli.base, "main");
        assert!(!cli.render);
        assert!(!cli.ansi);
        assert_eq!(cli.mode, RenderMode::Uncommitted);
        assert_eq!(cli.focus, None);
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
            "--focus",
            "history",
            "--mode",
            "branch",
            "--commit",
            "abc123",
            "--width",
            "100",
            "--height",
            "30",
            "--ansi",
        ])
        .expect("render options should be accepted");

        assert!(cli.render);
        assert_eq!(cli.mode, RenderMode::Branch);
        assert_eq!(cli.commit.as_deref(), Some("abc123"));
        assert_eq!(cli.width, 100);
        assert_eq!(cli.height, 30);
        assert!(cli.ansi);
        assert_eq!(cli.focus, Some(StartupFocus::History));
    }

    #[test]
    fn branch_mode_can_use_the_default_startup_focus() {
        let cli = Cli::try_parse_from(["git-navigator", ".", "--ansi", "--mode", "branch"])
            .expect("branch mode should be accepted without an explicit focus");
        assert_eq!(cli.mode, RenderMode::Branch);
        assert_eq!(cli.focus, None);
    }

    #[test]
    fn accepts_control_commands_without_a_directory() {
        let cli = Cli::try_parse_from(["git-navigator", "ctl", "sessions"])
            .expect("control commands should be accepted");
        assert!(matches!(cli.command, Some(Command::Ctl { .. })));
        assert_eq!(cli.directory, None);
    }

    #[test]
    fn accepts_explicit_layout_control_commands() {
        for command in ["expand", "collapse"] {
            let cli = Cli::try_parse_from(["git-navigator", "ctl", command])
                .expect("layout control commands should be accepted");
            assert!(matches!(cli.command, Some(Command::Ctl { .. })));
        }
    }

    #[test]
    fn accepts_repository_control_commands() {
        let cli =
            Cli::try_parse_from(["git-navigator", "ctl", "repository", "/path/to/repository"])
                .expect("repository control command should be accepted");
        let Some(Command::Ctl { control }) = cli.command else {
            panic!("expected a control command");
        };
        assert!(matches!(
            control.command,
            ControlCommand::Repository { path }
                if path.as_path() == std::path::Path::new("/path/to/repository")
        ));
    }
}
