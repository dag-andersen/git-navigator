use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::{ControlArgs, ControlCommand, StartupFocus};

const SESSION_DIRECTORY: &str = "git-navigator-sessions";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Focus { panel: Panel },
    Expand,
    Collapse,
    Worktree { path: PathBuf },
    Repository { path: PathBuf },
    File { path: PathBuf },
    Commit { hash: String },
    Refresh,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Panel {
    Worktrees,
    History,
    Files,
    Diff,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn from_result(result: Result<()>) -> Self {
        match result {
            Ok(()) => Self {
                ok: true,
                error: None,
            },
            Err(error) => Self {
                ok: false,
                error: Some(format!("{error:#}")),
            },
        }
    }
}

#[derive(Debug)]
pub struct ControlMessage {
    pub request: Request,
    pub response: Sender<Response>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub pid: u32,
    pub repository: PathBuf,
    pub socket: PathBuf,
}

pub struct Session {
    info: SessionInfo,
    receiver: Receiver<ControlMessage>,
}

impl Session {
    pub fn start(repository: &Path) -> Result<Self> {
        let directory = session_directory()?;
        fs::create_dir_all(&directory)
            .with_context(|| format!("could not create {}", directory.display()))?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("could not secure {}", directory.display()))?;
        let id = format!("{}-{}", std::process::id(), timestamp());
        let socket = directory.join(format!("{id}.sock"));
        let metadata = directory.join(format!("{id}.json"));
        let listener = UnixListener::bind(&socket)
            .with_context(|| format!("could not bind control socket {}", socket.display()))?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("could not secure {}", socket.display()))?;
        let info = SessionInfo {
            id,
            pid: std::process::id(),
            repository: repository.to_path_buf(),
            socket: socket.clone(),
        };
        write_session_info(&metadata, &info)?;
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("git-navigator-control".into())
            .spawn(move || accept_connections(listener, sender))
            .context("could not start control listener")?;
        Ok(Self { info, receiver })
    }

    pub fn receiver(&self) -> &Receiver<ControlMessage> {
        &self.receiver
    }

    pub fn update_repository(&mut self, repository: &Path) -> Result<()> {
        if self.info.repository == repository {
            return Ok(());
        }
        let mut info = self.info.clone();
        info.repository = repository.to_path_buf();
        let metadata = self
            .info
            .socket
            .parent()
            .context("control socket has no parent directory")?
            .join(format!("{}.json", self.info.id));
        write_session_info(&metadata, &info)?;
        self.info = info;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.info.socket);
        if let Some(directory) = self.info.socket.parent() {
            let _ = fs::remove_file(directory.join(format!("{}.json", self.info.id)));
        }
    }
}

pub fn run_ctl(args: ControlArgs) -> Result<()> {
    if matches!(args.command, ControlCommand::Sessions) {
        for session in discover_sessions(args.directory.as_deref())? {
            println!(
                "{}\t{}\t{}\t{}",
                session.id,
                session.repository.display(),
                session.socket.display(),
                session.pid
            );
        }
        return Ok(());
    }

    let socket = find_socket(&args)?;
    let request = request_from_command(args.command)?;
    let mut stream = UnixStream::connect(&socket)
        .with_context(|| format!("could not connect to {}", socket.display()))?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    let response: Response = serde_json::from_str(&response).context("invalid TUI response")?;
    if response.ok {
        Ok(())
    } else {
        bail!(
            response
                .error
                .unwrap_or_else(|| "control command failed".into())
        )
    }
}

fn request_from_command(command: ControlCommand) -> Result<Request> {
    Ok(match command {
        ControlCommand::Focus { panel } => Request::Focus {
            panel: panel.into(),
        },
        ControlCommand::Expand => Request::Expand,
        ControlCommand::Collapse => Request::Collapse,
        ControlCommand::Worktree { path } => Request::Worktree { path },
        ControlCommand::Repository { path } => Request::Repository {
            path: path
                .canonicalize()
                .with_context(|| format!("cannot open repository {}", path.display()))?,
        },
        ControlCommand::File { path } => Request::File { path },
        ControlCommand::Commit { hash } => Request::Commit { hash },
        ControlCommand::Refresh => Request::Refresh,
        ControlCommand::Sessions => bail!("sessions does not send a control command"),
    })
}

fn find_socket(args: &ControlArgs) -> Result<PathBuf> {
    if let Some(socket) = &args.socket {
        return Ok(socket.clone());
    }
    let sessions = discover_sessions(args.directory.as_deref())?;
    if let Some(id) = &args.session {
        return sessions
            .into_iter()
            .find(|session| session.id == *id)
            .map(|session| session.socket)
            .with_context(|| format!("no running git-navigator session named {id}"));
    }
    match sessions.as_slice() {
        [session] => Ok(session.socket.clone()),
        [] => bail!("no running git-navigator session found"),
        _ => bail!("multiple sessions found; pass --session or --socket"),
    }
}

fn discover_sessions(repository: Option<&Path>) -> Result<Vec<SessionInfo>> {
    let directory = session_directory()?;
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", directory.display()));
        }
    };
    let repository = repository
        .map(Path::canonicalize)
        .transpose()?
        .map(|path| repository_identity(&path))
        .transpose()?;
    let mut sessions = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("json")
        {
            continue;
        }
        let Ok(contents) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(session) = serde_json::from_slice::<SessionInfo>(&contents) else {
            continue;
        };
        if !session.socket.exists() || !process_exists(session.pid) {
            let _ = fs::remove_file(entry.path());
            let _ = fs::remove_file(&session.socket);
            continue;
        }
        if repository.as_deref().is_none_or(|path| {
            repository_identity(&session.repository)
                .ok()
                .is_some_and(|session_identity| session_identity == *path)
        }) {
            sessions.push(session);
        }
    }
    sessions.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(sessions)
}

fn repository_identity(path: &Path) -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            path.to_string_lossy().as_ref(),
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ])
        .output()
        .with_context(|| format!("could not inspect Git repository {}", path.display()))?;
    if !output.status.success() {
        bail!("{} is not inside a Git repository", path.display());
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn accept_connections(listener: UnixListener, sender: Sender<ControlMessage>) {
    for stream in listener.incoming().flatten() {
        let sender = sender.clone();
        thread::spawn(move || handle_connection(stream, sender));
    }
}

fn handle_connection(stream: UnixStream, sender: Sender<ControlMessage>) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => {
            let (response_sender, response_receiver) = mpsc::channel();
            if sender
                .send(ControlMessage {
                    request,
                    response: response_sender,
                })
                .is_err()
            {
                return;
            }
            response_receiver.recv().unwrap_or_else(|_| Response {
                ok: false,
                error: Some("TUI stopped before handling the command".into()),
            })
        }
        Err(error) => Response {
            ok: false,
            error: Some(format!("invalid control command: {error}")),
        },
    };
    let mut stream = reader.into_inner();
    let _ = serde_json::to_writer(&mut stream, &response);
    let _ = stream.write_all(b"\n");
}

fn session_directory() -> Result<PathBuf> {
    Ok(std::env::temp_dir().join(SESSION_DIRECTORY))
}

fn write_session_info(metadata: &Path, info: &SessionInfo) -> Result<()> {
    let temporary = metadata.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(info)?)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("could not secure {}", temporary.display()))?;
    fs::rename(&temporary, metadata)
        .with_context(|| format!("could not replace {}", metadata.display()))?;
    Ok(())
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn process_exists(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

impl From<StartupFocus> for Panel {
    fn from(panel: StartupFocus) -> Self {
        match panel {
            StartupFocus::Worktrees => Self::Worktrees,
            StartupFocus::History => Self::History,
            StartupFocus::Files => Self::Files,
            StartupFocus::Diff => Self::Diff,
        }
    }
}
