use std::path::PathBuf;

use clap::Parser;

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
    }

    #[test]
    fn accepts_a_custom_base_branch() {
        let cli = Cli::try_parse_from(["git-navigator", ".", "--base", "trunk"])
            .expect("a custom base should be accepted");
        assert_eq!(cli.base, "trunk");
    }
}
