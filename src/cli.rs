use crate::{
    audio::Beat,
    config::{self, Audio, BUILT_INS, Config, Noise, Options},
};
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use rodio::{OutputStreamBuilder, Sink};
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    flag,
};
use std::{
    env, fs,
    io::{BufRead, BufReader, Read, Write},
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

#[derive(Parser)]
#[command(
    about = "Experimental binaural-beat player; headphones required",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    playback: PlaybackArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Run long-lived local IPC daemon.
    Daemon,
    /// Send a command to running daemon.
    Msg {
        #[command(subcommand)]
        command: Message,
    },
    /// List built-in and configured presets.
    Presets,
}

#[derive(Debug, PartialEq, Subcommand)]
enum Message {
    Status,
    Stop,
    Pause,
    Play,
    Preset { name: String },
    Volume { value: f32 },
    Noise { kind: Noise, volume: Option<f32> },
    Reload,
    Shutdown,
}

#[derive(Args)]
struct PlaybackArgs {
    #[arg(long)]
    preset: Option<String>,
    #[arg(long, requires = "beat", conflicts_with_all = ["left", "right"])]
    carrier: Option<f64>,
    #[arg(long, requires = "carrier", conflicts_with_all = ["left", "right"])]
    beat: Option<f64>,
    #[arg(long, requires = "right", conflicts_with_all = ["carrier", "beat"])]
    left: Option<f64>,
    #[arg(long, requires = "left", conflicts_with_all = ["carrier", "beat"])]
    right: Option<f64>,
    #[arg(long)]
    volume: Option<f32>,
    #[arg(long)]
    noise: Option<Noise>,
    #[arg(long)]
    noise_volume: Option<f32>,
    #[arg(long)]
    fade: Option<f64>,
}

impl From<PlaybackArgs> for Options {
    fn from(args: PlaybackArgs) -> Self {
        Self {
            preset: args.preset,
            carrier: args.carrier,
            beat: args.beat,
            left: args.left,
            right: args.right,
            volume: args.volume,
            noise: args.noise,
            noise_volume: args.noise_volume,
            fade: args.fade,
        }
    }
}

fn socket_path() -> Result<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("bbeats.sock"))
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is required for daemon mode"))
}

fn command_options(preset: &str) -> Options {
    Options {
        preset: Some(preset.into()),
        ..Options::default()
    }
}

const CONTROL_FADE: Duration = Duration::from_millis(100);
const CONTROL_STEPS: u32 = 20;
const IPC_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_MESSAGE_BYTES: usize = 4_096;
const COMMAND_HELP: &str =
    "commands: status stop pause play preset NAME volume N noise TYPE [VOLUME] reload shutdown";

fn load(sink: &Sink, config: &Config, options: &Options) -> Result<(String, Audio)> {
    let preset = options
        .preset
        .as_deref()
        .unwrap_or(&config.default)
        .to_owned();
    let (left, right, audio) = config::resolve(options, config)?;
    if !sink.empty() && !sink.is_paused() {
        fade_out(sink);
    }
    sink.clear();
    sink.set_volume(1.0);
    sink.append(Beat::new(left, right, audio));
    sink.pause();
    Ok((preset, audio))
}

fn fade_out(sink: &Sink) {
    for step in (0..CONTROL_STEPS).rev() {
        sink.set_volume(step as f32 / CONTROL_STEPS as f32);
        thread::sleep(CONTROL_FADE / CONTROL_STEPS);
    }
}

fn fade_in(sink: &Sink) {
    sink.set_volume(0.0);
    sink.play();
    for step in 1..=CONTROL_STEPS {
        sink.set_volume(step as f32 / CONTROL_STEPS as f32);
        thread::sleep(CONTROL_FADE / CONTROL_STEPS);
    }
}

fn parse_message(line: &str) -> Result<Message> {
    let line = line.trim();
    let command_end = line.find(char::is_whitespace).unwrap_or(line.len());
    let (command, arguments) = line.split_at(command_end);
    let arguments = arguments.trim();
    let words: Vec<_> = arguments.split_whitespace().collect();
    match (command, words.as_slice()) {
        ("status", []) => Ok(Message::Status),
        ("stop", []) => Ok(Message::Stop),
        ("pause", []) => Ok(Message::Pause),
        ("play", []) => Ok(Message::Play),
        ("preset", [_, ..]) => Ok(Message::Preset {
            name: arguments.to_owned(),
        }),
        ("volume", [value]) => Ok(Message::Volume {
            value: value
                .parse()
                .map_err(|_| anyhow!("volume must be a number"))?,
        }),
        ("noise", [kind]) => Ok(Message::Noise {
            kind: Noise::parse(kind)?,
            volume: None,
        }),
        ("noise", [kind, volume]) => Ok(Message::Noise {
            kind: Noise::parse(kind)?,
            volume: Some(
                volume
                    .parse()
                    .map_err(|_| anyhow!("noise volume must be a number"))?,
            ),
        }),
        ("reload", []) => Ok(Message::Reload),
        ("shutdown", []) => Ok(Message::Shutdown),
        _ => Err(anyhow!(COMMAND_HELP)),
    }
}

fn format_message(command: Message) -> Result<String> {
    Ok(match command {
        Message::Status => "status".into(),
        Message::Stop => "stop".into(),
        Message::Pause => "pause".into(),
        Message::Play => "play".into(),
        Message::Preset { name } => {
            if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
                bail!(
                    "preset name cannot be empty or contain surrounding whitespace or control characters"
                );
            }
            format!("preset {name}")
        }
        Message::Volume { value } => format!("volume {value}"),
        Message::Noise { kind, volume } => match volume {
            Some(volume) => format!("noise {} {volume}", kind.as_str()),
            None => format!("noise {}", kind.as_str()),
        },
        Message::Reload => "reload".into(),
        Message::Shutdown => "shutdown".into(),
    })
}

fn read_message(reader: impl Read) -> Result<Option<String>> {
    let mut message = String::new();
    let mut reader = BufReader::new(reader).take((MAX_MESSAGE_BYTES + 1) as u64);
    let bytes = reader.read_line(&mut message)?;
    if bytes == 0 {
        return Ok(None);
    }
    if bytes > MAX_MESSAGE_BYTES {
        bail!("IPC message exceeds {MAX_MESSAGE_BYTES} bytes");
    }
    Ok(Some(message))
}

struct PlaybackState {
    current: Options,
    preset: String,
    audio: Audio,
}

impl PlaybackState {
    fn new(sink: &Sink, config: &Config) -> Result<Self> {
        let current = command_options(&config.default);
        let (preset, audio) = load(sink, config, &current)?;
        Ok(Self {
            current,
            preset,
            audio,
        })
    }

    fn replace(&mut self, sink: &Sink, config: &Config, next: Options) -> Result<()> {
        let (preset, audio) = load(sink, config, &next)?;
        self.current = next;
        self.preset = preset;
        self.audio = audio;
        Ok(())
    }

    fn reload(&mut self, sink: &Sink, config: &Config) -> Result<()> {
        let (preset, audio) = load(sink, config, &self.current)?;
        self.preset = preset;
        self.audio = audio;
        Ok(())
    }

    fn apply(
        &mut self,
        command: Message,
        sink: &Sink,
        config: &mut Config,
        shutdown: &AtomicBool,
    ) -> Result<String> {
        match command {
            Message::Status => Ok(format!(
                "ok preset={} volume={:.2} noise={} noise-volume={:.2} paused={} playing={}",
                self.preset,
                self.audio.volume,
                self.audio.noise.as_str(),
                self.audio.noise_volume,
                sink.is_paused(),
                !sink.is_paused() && !sink.empty()
            )),
            Message::Stop => {
                if !sink.is_paused() && !sink.empty() {
                    fade_out(sink);
                }
                sink.clear();
                sink.set_volume(1.0);
                eprintln!("bbeats: stopped");
                Ok("ok stopped".into())
            }
            Message::Pause => {
                if !sink.is_paused() && !sink.empty() {
                    fade_out(sink);
                }
                sink.pause();
                sink.set_volume(1.0);
                eprintln!("bbeats: paused");
                Ok("ok paused".into())
            }
            Message::Play => {
                if sink.empty() {
                    self.reload(sink, config)?;
                }
                if sink.is_paused() {
                    fade_in(sink);
                }
                eprintln!("bbeats: preset={}; playing", self.preset);
                Ok("ok playing".into())
            }
            Message::Preset { name } => {
                self.replace(sink, config, command_options(&name))?;
                fade_in(sink);
                eprintln!("bbeats: preset={}; playing", self.preset);
                Ok("ok playing".into())
            }
            Message::Volume { value } => {
                if !(0.0..=0.25).contains(&value) {
                    bail!("volume must be 0..=0.25");
                }
                let mut next = self.current.clone();
                next.volume = Some(value);
                self.replace(sink, config, next)?;
                eprintln!("bbeats: volume={:.2}; paused", self.audio.volume);
                Ok("ok".into())
            }
            Message::Noise { kind, volume } => {
                if volume.is_some_and(|volume| !(0.0..=0.25).contains(&volume)) {
                    bail!("noise volume must be 0..=0.25");
                }
                let mut next = self.current.clone();
                next.noise = Some(kind);
                if let Some(volume) = volume {
                    next.noise_volume = Some(volume);
                }
                self.replace(sink, config, next)?;
                eprintln!(
                    "bbeats: noise={} volume={:.2}; paused",
                    self.audio.noise.as_str(),
                    self.audio.noise_volume
                );
                Ok("ok".into())
            }
            Message::Reload => {
                let next = config::load().context("reloading configuration")?;
                let playing = !sink.is_paused() && !sink.empty();
                self.reload(sink, &next)?;
                *config = next;
                if playing {
                    fade_in(sink);
                }
                eprintln!("bbeats: configuration reloaded; preset={}", self.preset);
                Ok("ok reloaded".into())
            }
            Message::Shutdown => {
                shutdown.store(true, Ordering::Relaxed);
                eprintln!("bbeats: shutdown requested");
                Ok("ok shutting down".into())
            }
        }
    }
}

fn reply(stream: &mut UnixStream, text: &str) -> Result<()> {
    writeln!(stream, "{text}").context("replying to IPC client")
}

fn handle_client(
    mut client: UnixStream,
    state: &mut PlaybackState,
    sink: &Sink,
    config: &mut Config,
    shutdown: &AtomicBool,
) -> Result<()> {
    client
        .set_read_timeout(Some(IPC_TIMEOUT))
        .context("setting IPC read timeout")?;
    client
        .set_write_timeout(Some(IPC_TIMEOUT))
        .context("setting IPC write timeout")?;
    let Some(request) = read_message(&client).context("reading IPC command")? else {
        return Ok(());
    };
    let response = match parse_message(&request) {
        Ok(command) => state
            .apply(command, sink, config, shutdown)
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

fn daemon() -> Result<()> {
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
        "bbeats: daemon started; socket={}; preset={}; paused",
        path.display(),
        config.default
    );
    let shutdown = Arc::new(AtomicBool::new(false));
    flag::register(SIGINT, Arc::clone(&shutdown))?;
    flag::register(SIGTERM, Arc::clone(&shutdown))?;
    let stream = OutputStreamBuilder::open_default_stream().context("opening audio output")?;
    let sink = Sink::connect_new(stream.mixer());
    let mut state = PlaybackState::new(&sink, &config)?;
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((client, _)) => {
                if let Err(error) =
                    handle_client(client, &mut state, &sink, &mut config, shutdown.as_ref())
                {
                    eprintln!("bbeats: IPC client error: {error:#}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error).context("accepting IPC connection"),
        }
    }
    sink.stop();
    eprintln!("bbeats: daemon stopped");
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

fn message(command: Message) -> Result<()> {
    let command = format_message(command)?;
    let mut stream = UnixStream::connect(socket_path()?).context("connecting to bbeats daemon")?;
    stream
        .set_read_timeout(Some(IPC_TIMEOUT))
        .context("setting daemon reply timeout")?;
    stream
        .set_write_timeout(Some(IPC_TIMEOUT))
        .context("setting daemon command timeout")?;
    writeln!(stream, "{command}").context("sending daemon command")?;
    let Some(response) = read_message(stream).context("reading daemon reply")? else {
        bail!("daemon closed connection without replying");
    };
    if let Some(error) = response.strip_prefix("error ") {
        bail!("daemon: {}", error.trim_end());
    }
    print!("{response}");
    Ok(())
}

fn run_presets() -> Result<()> {
    let config = config::load().context("loading configuration")?;
    for preset in BUILT_INS {
        println!(
            "{}: {:0.1}/{:0.1} Hz — {}",
            preset.name, preset.left, preset.right, preset.description
        );
    }
    for preset in &config.presets {
        println!(
            "{}: {:0.1}/{:0.1} Hz",
            preset.name, preset.left, preset.right
        );
    }
    Ok(())
}

pub(super) fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Daemon) => return daemon(),
        Some(Command::Msg { command }) => return message(command),
        Some(Command::Presets) => return run_presets(),
        None => {}
    }
    let config = config::load()?;
    let options: Options = cli.playback.into();
    let preset = options.preset.as_deref().unwrap_or(&config.default);
    let (left, right, audio) = config::resolve(&options, &config)?;
    eprintln!(
        "[{preset}] left {left:.2} Hz · right {right:.2} Hz · beat {:.2} Hz · tone {:.2} · noise {} {:.2} · fade {:.0}s",
        (right - left).abs(),
        audio.volume,
        audio.noise.as_str(),
        audio.noise_volume,
        audio.fade,
    );
    let stream = OutputStreamBuilder::open_default_stream()?;
    let sink = Sink::connect_new(stream.mixer());
    sink.append(Beat::new(left, right, audio));
    sink.sleep_until_end();
    Ok(())
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
        env::temp_dir().join(format!("bbeats-{}-{unique}.sock", std::process::id()))
    }

    #[test]
    fn message_protocol_round_trips_spaced_preset_and_reload() {
        let encoded = format_message(Message::Preset {
            name: "late focus".into(),
        })
        .unwrap();
        assert_eq!(
            parse_message(&encoded).unwrap(),
            Message::Preset {
                name: "late focus".into()
            }
        );
        assert_eq!(parse_message("reload").unwrap(), Message::Reload);
        assert_eq!(format_message(Message::Reload).unwrap(), "reload");
        assert!(
            format_message(Message::Preset {
                name: "focus\nshutdown".into()
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_oversized_ipc_command() {
        let request = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        assert!(read_message(request.as_slice()).is_err());
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

    #[test]
    fn rejects_playback_options_with_subcommands() {
        for arguments in [
            ["bbeats", "presets", "--volume", "0.2"],
            ["bbeats", "--volume", "0.2", "presets"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
    }
}
