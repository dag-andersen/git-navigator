use std::{env, io, path::Path, process::Command};

#[derive(Debug, Eq, PartialEq)]
struct EditorCommand {
    program: String,
    args: Vec<String>,
}

pub fn open(path: &Path) -> io::Result<()> {
    let command = command_for_path(
        path,
        env::var("VISUAL")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        env::var("EDITOR")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    );

    Command::new(&command.program)
        .args(&command.args)
        .spawn()
        .map(|_| ())
}

fn command_for_path(path: &Path, visual: Option<String>, editor: Option<String>) -> EditorCommand {
    if let Some(command) = visual.or(editor) {
        let mut parts = command.split_whitespace();
        let program = parts
            .next()
            .expect("editor commands are filtered for empty values")
            .to_owned();
        let mut args = parts.map(str::to_owned).collect::<Vec<_>>();
        args.push(path.display().to_string());
        return EditorCommand { program, args };
    }

    #[cfg(target_os = "macos")]
    let (program, args) = ("open".to_owned(), vec![path.display().to_string()]);

    #[cfg(target_os = "linux")]
    let (program, args) = ("xdg-open".to_owned(), vec![path.display().to_string()]);

    #[cfg(target_os = "windows")]
    let (program, args) = (
        "cmd".to_owned(),
        vec![
            "/C".to_owned(),
            "start".to_owned(),
            "".to_owned(),
            path.display().to_string(),
        ],
    );

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let (program, args) = ("xdg-open".to_owned(), vec![path.display().to_string()]);

    EditorCommand { program, args }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_takes_precedence_over_editor() {
        assert_eq!(
            command_for_path(
                Path::new("/repo/worktree"),
                Some("code --reuse-window".into()),
                Some("vim".into()),
            ),
            EditorCommand {
                program: "code".into(),
                args: vec!["--reuse-window".into(), "/repo/worktree".into()],
            }
        );
    }

    #[test]
    fn editor_is_used_when_visual_is_not_set() {
        assert_eq!(
            command_for_path(Path::new("/repo/worktree"), None, Some("zed".into())),
            EditorCommand {
                program: "zed".into(),
                args: vec!["/repo/worktree".into()],
            }
        );
    }

    #[test]
    fn paths_with_spaces_remain_one_argument() {
        assert_eq!(
            command_for_path(Path::new("/repo/my worktree"), Some("code".into()), None,).args,
            vec!["/repo/my worktree"]
        );
    }

    #[test]
    fn empty_editor_values_use_the_platform_opener() {
        let command = command_for_path(Path::new("/repo"), None, None);
        assert_eq!(command.args, vec!["/repo"]);
    }
}
