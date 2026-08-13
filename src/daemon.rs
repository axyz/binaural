use crate::{
    config::{self, Config},
    ipc::{self, Message},
    playback::PlaybackState,
};
use anyhow::{Context, Result, anyhow, bail};
use rodio::{DeviceSinkBuilder, Player};
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    flag,
};
use std::{
    env, fs,
    io::Write,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn socket_path() -> Result<PathBuf> {
    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(runtime_dir).join("binaural.sock"));
    }
    #[cfg(target_os = "macos")]
    {
        env::var_os("TMPDIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map(|path| path.join("binaural.sock"))
            .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR or TMPDIR is required for daemon mode"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(anyhow!("XDG_RUNTIME_DIR is required for daemon mode"))
    }
}

fn reply(stream: &mut UnixStream, text: &str) -> Result<()> {
    writeln!(stream, "{text}").context("replying to IPC client")
}

fn handle_client(
    mut client: UnixStream,
    state: &mut PlaybackState,
    config: &mut Config,
    shutdown: &AtomicBool,
) -> Result<()> {
    client
        .set_read_timeout(Some(ipc::TIMEOUT))
        .context("setting IPC read timeout")?;
    client
        .set_write_timeout(Some(ipc::TIMEOUT))
        .context("setting IPC write timeout")?;
    let Some(request) = ipc::read(&client).context("reading IPC command")? else {
        return Ok(());
    };
    let response = match ipc::parse(&request) {
        Ok(command) => state
            .apply(command, config, shutdown)
            .unwrap_or_else(|error| format!("error {error:#}")),
        Err(error) => format!("error {error}"),
    };
    reply(&mut client, &response)
}

fn clear_stale_socket(path: &Path) -> Result<()> {
    let connect_error = match UnixStream::connect(path) {
        Ok(_) => return Err(anyhow!("daemon already running: {}", path.display())),
        Err(error) => error,
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspecting daemon socket: {}", path.display()));
        }
    };
    if !metadata.file_type().is_socket() {
        bail!("refusing to replace non-socket path: {}", path.display());
    }
    if connect_error.kind() != std::io::ErrorKind::ConnectionRefused {
        return Err(connect_error)
            .with_context(|| format!("connecting to daemon socket: {}", path.display()));
    }
    fs::remove_file(path).with_context(|| format!("removing stale socket: {}", path.display()))
}

pub(super) fn run() -> Result<()> {
    let mut config = config::load().context("loading configuration")?;
    let path = socket_path()?;
    clear_stale_socket(&path)?;
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("binding daemon socket: {}", path.display()))?;
    let _socket = SocketGuard::new(path.clone())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("securing daemon socket: {}", path.display()))?;
    listener
        .set_nonblocking(true)
        .context("configuring daemon socket")?;
    eprintln!(
        "binaural: daemon started; socket={}; preset={}; paused",
        path.display(),
        config.default
    );
    let shutdown = Arc::new(AtomicBool::new(false));
    flag::register(SIGINT, Arc::clone(&shutdown))?;
    flag::register(SIGTERM, Arc::clone(&shutdown))?;
    let mut stream = DeviceSinkBuilder::open_default_sink().context("opening audio output")?;
    stream.log_on_drop(false);
    let sink = Player::connect_new(stream.mixer());
    let mut state = PlaybackState::new(&sink, &config)?;
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((client, _)) => {
                if let Err(error) =
                    handle_client(client, &mut state, &mut config, shutdown.as_ref())
                {
                    eprintln!("binaural: IPC client error: {error:#}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error).context("accepting IPC connection"),
        }
    }
    sink.stop();
    eprintln!("binaural: daemon stopped");
    Ok(())
}

pub(super) fn message(command: Message) -> Result<()> {
    let command = ipc::format(command)?;
    let mut stream =
        UnixStream::connect(socket_path()?).context("connecting to binaural daemon")?;
    stream
        .set_read_timeout(Some(ipc::TIMEOUT))
        .context("setting daemon reply timeout")?;
    stream
        .set_write_timeout(Some(ipc::TIMEOUT))
        .context("setting daemon command timeout")?;
    writeln!(stream, "{command}").context("sending daemon command")?;
    let Some(response) = ipc::read(stream).context("reading daemon reply")? else {
        bail!("daemon closed connection without replying");
    };
    if let Some(error) = response.strip_prefix("error ") {
        bail!("daemon: {}", error.trim_end());
    }
    print!("{response}");
    Ok(())
}

struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl SocketGuard {
    fn new(path: PathBuf) -> Result<Self> {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting bound socket: {}", path.display()))?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.dev() == self.device && metadata.ino() == self.inode)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_socket_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("binaural-{}-{unique}.sock", std::process::id()))
    }

    #[test]
    fn stale_socket_cleanup_preserves_regular_file() {
        let path = temp_socket_path();
        fs::write(&path, "keep").unwrap();
        assert!(clear_stale_socket(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"keep");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn socket_guard_preserves_replacement_socket() {
        let path = temp_socket_path();
        let original = UnixListener::bind(&path).unwrap();
        let guard = SocketGuard::new(path.clone()).unwrap();
        fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        drop(guard);
        assert!(path.exists());
        drop((original, replacement));
        fs::remove_file(path).unwrap();
    }
}
