use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, anyhow};

pub fn common_git_dir(directory: &Path) -> Result<PathBuf> {
    let path = git_text(
        directory,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(path.trim()))
}

pub(crate) fn git_text(directory: &Path, args: &[&str]) -> Result<String> {
    let output = git(directory, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    let output = git_allow_failure(directory, args)?;
    ensure_git_success(output, directory, &format!("git {}", args.join(" ")))
}

pub(crate) fn ensure_git_success(
    output: Output,
    directory: &Path,
    operation: &str,
) -> Result<Output> {
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

pub(crate) fn git_allow_failure(directory: &Path, args: &[&str]) -> Result<Output> {
    git_command(directory)
        .args(args.iter().map(OsStr::new))
        .output()
        .with_context(|| format!("failed to run git in {}", directory.display()))
}

pub(crate) fn git_command(directory: &Path) -> Command {
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
