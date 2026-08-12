use directories::ProjectDirs;
use kdl::{KdlDocument, KdlNode, KdlValue};
use rodio::{OutputStreamBuilder, Sink, Source};
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    flag,
};
use std::{
    env,
    error::Error,
    f32::consts::TAU,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const RATE: u32 = 44_100;
const MAX_SECONDS: f64 = 86_400.0;
const EXAMPLE: &str = r#"// All settings optional. `bbeats` uses default preset.
default preset="calm"

audio {
  volume 0.10
  fade 8
  noise "off" { volume 0.04 }
}

// Custom presets inherit global audio. `inherits` starts from a built-in or earlier custom preset.
// preset "reading" {
//   tone carrier=220 beat=10
//   duration 1500
//   volume 0.07
// }
// preset "evening" inherits="wind-down" {
//   fade 12
//   noise "brown" { volume 0.03 }
// }
"#;

#[derive(Clone, Copy, Default)]
enum Noise {
    #[default]
    Off,
    White,
    Pink,
    Brown,
}
impl Noise {
    fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::White => "white",
            Self::Pink => "pink",
            Self::Brown => "brown",
        }
    }
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "off" => Ok(Self::Off),
            "white" => Ok(Self::White),
            "pink" => Ok(Self::Pink),
            "brown" => Ok(Self::Brown),
            _ => Err("noise must be off, white, pink, or brown".into()),
        }
    }
}
#[derive(Clone, Copy)]
struct Audio {
    volume: f32,
    noise: Noise,
    noise_volume: f32,
    fade: f64,
}
const AUDIO: Audio = Audio {
    volume: 0.10,
    noise: Noise::Off,
    noise_volume: 0.04,
    fade: 8.0,
};
struct Preset {
    name: &'static str,
    left: f64,
    right: f64,
    seconds: f64,
    description: &'static str,
}
const BUILT_INS: &[Preset] = &[
    Preset {
        name: "calm",
        left: 195.,
        right: 205.,
        seconds: 600.,
        description: "10 Hz, alpha; quiet-break routine",
    },
    Preset {
        name: "study",
        left: 400.,
        right: 415.,
        seconds: 1500.,
        description: "15 Hz, beta; study routine",
    },
    Preset {
        name: "focus",
        left: 113.,
        right: 127.,
        seconds: 1500.,
        description: "14 Hz, beta; work-block routine",
    },
    Preset {
        name: "wind-down",
        left: 197.,
        right: 203.,
        seconds: 900.,
        description: "6 Hz, theta; pre-rest routine",
    },
];
struct UserPreset {
    name: String,
    left: f64,
    right: f64,
    seconds: f64,
    audio: Audio,
}
struct Config {
    default: String,
    audio: Audio,
    presets: Vec<UserPreset>,
}
struct Options {
    preset: Option<String>,
    carrier: Option<f64>,
    beat: Option<f64>,
    left: Option<f64>,
    right: Option<f64>,
    seconds: Option<f64>,
    volume: Option<f32>,
    noise: Option<Noise>,
    noise_volume: Option<f32>,
    fade: Option<f64>,
}

struct Beat {
    frame: u64,
    frames: u64,
    left: f64,
    right: f64,
    audio: Audio,
    fade_frames: u64,
    channel: bool,
    noise_sample: f32,
    pink: [f32; 7],
    brown: f32,
    random: u32,
}
impl Beat {
    fn gain(&self) -> f32 {
        if self.fade_frames == 0 {
            return 1.;
        }
        let r = (self.frame as f64 / self.fade_frames as f64)
            .min((self.frames - self.frame) as f64 / self.fade_frames as f64)
            .min(1.);
        (r * std::f64::consts::FRAC_PI_2).sin() as f32
    }
    fn sample(&self, f: f64) -> f32 {
        (((TAU as f64 * f * self.frame as f64 / RATE as f64).sin() as f32 * self.audio.volume)
            + self.noise_sample * self.audio.noise_volume)
            * self.gain()
    }
    fn next_noise(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        let w = self.random as f32 / u32::MAX as f32 * 2. - 1.;
        match self.audio.noise {
            Noise::Off => 0.,
            Noise::White => w,
            Noise::Brown => {
                self.brown = (self.brown + w * 0.02).clamp(-1., 1.);
                self.brown
            }
            Noise::Pink => {
                self.pink[0] = 0.99886 * self.pink[0] + w * 0.0555179;
                self.pink[1] = 0.99332 * self.pink[1] + w * 0.0750759;
                self.pink[2] = 0.969 * self.pink[2] + w * 0.153_852;
                self.pink[3] = 0.8665 * self.pink[3] + w * 0.3104856;
                self.pink[4] = 0.55 * self.pink[4] + w * 0.5329522;
                self.pink[5] = -0.7616 * self.pink[5] - w * 0.016898;
                let p = self.pink[..6].iter().sum::<f32>() + self.pink[6] + w * 0.5362;
                self.pink[6] = w * 0.115926;
                p * 0.11
            }
        }
    }
}
impl Iterator for Beat {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.frame >= self.frames {
            return None;
        }
        if !self.channel {
            self.noise_sample = self.next_noise();
            self.channel = true;
            Some(self.sample(self.left))
        } else {
            self.channel = false;
            let x = self.sample(self.right);
            self.frame += 1;
            Some(x)
        }
    }
}
impl Source for Beat {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(self.frames as f64 / RATE as f64))
    }
}

fn usage() {
    println!(
        "bbeats [--preset NAME] [--carrier HZ --beat HZ | --left HZ --right HZ]\nbbeats --daemon\nbbeats msg status|stop|pause|resume|preset NAME|volume N|noise TYPE [VOLUME]|shutdown\n\nNo arguments use config default. Config: OS config folder/bbeats/config.kdl\n--volume 0..0.25 --noise off|white|pink|brown --noise-volume 0..0.25 --fade SECONDS\n--seconds N --presets --help"
    );
}
fn number(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<f64, String> {
    let v = args
        .next()
        .ok_or_else(|| format!("{flag} needs a value"))?
        .parse::<f64>()
        .map_err(|_| format!("{flag} needs a number"))?;
    v.is_finite()
        .then_some(v)
        .ok_or_else(|| format!("{flag} must be finite"))
}
fn text(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}
fn options() -> Result<Options, String> {
    let mut o = Options {
        preset: None,
        carrier: None,
        beat: None,
        left: None,
        right: None,
        seconds: None,
        volume: None,
        noise: None,
        noise_volume: None,
        fade: None,
    };
    let mut a = env::args().skip(1);
    while let Some(x) = a.next() {
        match x.as_str() {
            "-h" | "--help" => {
                usage();
                std::process::exit(0)
            }
            "--preset" => o.preset = Some(text(&mut a, "--preset")?),
            "--carrier" => o.carrier = Some(number(&mut a, "--carrier")?),
            "--beat" => o.beat = Some(number(&mut a, "--beat")?),
            "--left" => o.left = Some(number(&mut a, "--left")?),
            "--right" => o.right = Some(number(&mut a, "--right")?),
            "--seconds" => o.seconds = Some(number(&mut a, "--seconds")?),
            "--volume" => o.volume = Some(number(&mut a, "--volume")? as f32),
            "--noise" => o.noise = Some(Noise::parse(&text(&mut a, "--noise")?)?),
            "--noise-volume" => o.noise_volume = Some(number(&mut a, "--noise-volume")? as f32),
            "--fade" => o.fade = Some(number(&mut a, "--fade")?),
            "--presets" => o.preset = Some("--presets".into()),
            _ => return Err(format!("unknown option: {x}")),
        }
    }
    Ok(o)
}
fn path() -> Result<PathBuf, String> {
    ProjectDirs::from("dev", "bbeats", "bbeats")
        .map(|p| p.config_dir().join("config.kdl"))
        .ok_or_else(|| "cannot determine config directory".into())
}
fn num(n: &KdlNode, key: &str) -> Result<Option<f64>, String> {
    let Some(v) = n.get(key) else {
        return Ok(None);
    };
    let x = match v {
        KdlValue::Integer(x) => *x as f64,
        KdlValue::Float(x) => *x,
        _ => return Err(format!("{key} must be a number")),
    };
    x.is_finite()
        .then_some(Some(x))
        .ok_or_else(|| format!("{key} must be finite"))
}
fn strv<'a>(n: &'a KdlNode, key: &str) -> Result<Option<&'a str>, String> {
    n.get(key)
        .map(|v| {
            v.as_string()
                .ok_or_else(|| format!("{key} must be a string"))
        })
        .transpose()
}
fn child<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlNode> {
    node.children()?
        .nodes()
        .iter()
        .find(|child| child.name().value() == name)
}
fn argument(node: &KdlNode, index: usize, name: &str) -> Result<f64, String> {
    match node.get(index) {
        Some(KdlValue::Integer(value)) => Ok(*value as f64),
        Some(KdlValue::Float(value)) if value.is_finite() => Ok(*value),
        _ => Err(format!("{name} needs a finite number")),
    }
}
fn audio(node: &KdlNode, base: Audio) -> Result<Audio, String> {
    let mut audio = base;
    if let Some(node) = child(node, "volume") {
        audio.volume = argument(node, 0, "volume")? as f32;
    }
    if let Some(node) = child(node, "fade") {
        audio.fade = argument(node, 0, "fade")?;
    }
    if let Some(node) = child(node, "noise") {
        audio.noise = node
            .get(0)
            .and_then(KdlValue::as_string)
            .map(Noise::parse)
            .transpose()?
            .ok_or("noise needs type")?;
        if let Some(volume) = child(node, "volume") {
            audio.noise_volume = argument(volume, 0, "noise volume")? as f32;
        }
    }
    if !(0. ..=0.25).contains(&audio.volume)
        || !(0. ..=0.25).contains(&audio.noise_volume)
        || audio.fade < 0.
    {
        return Err("audio values out of range".into());
    }
    Ok(audio)
}
fn config() -> Result<Config, String> {
    let p = path()?;
    if !p.exists() {
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&p, EXAMPLE).map_err(|e| e.to_string())?;
    }
    let doc: KdlDocument = fs::read_to_string(&p)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e| format!("invalid {}: {e}", p.display()))?;
    let mut c = Config {
        default: "calm".into(),
        audio: AUDIO,
        presets: vec![],
    };
    for n in doc.nodes() {
        match n.name().value() {
            "default" => c.default = strv(n, "preset")?.ok_or("default needs preset")?.into(),
            "audio" => c.audio = audio(n, c.audio)?,
            "preset" => {
                let name = n
                    .get(0)
                    .and_then(KdlValue::as_string)
                    .ok_or("preset needs name")?
                    .to_owned();
                if BUILT_INS.iter().any(|preset| preset.name == name)
                    || c.presets.iter().any(|preset| preset.name == name)
                {
                    return Err(format!("duplicate or built-in preset: {name}"));
                }
                let inherited = strv(n, "inherits")?.unwrap_or("");
                let (base_left, base_right, base_seconds, base_audio) = if inherited.is_empty() {
                    (0.0, 0.0, 0.0, c.audio)
                } else {
                    find(&c, inherited)?
                };
                let tone = child(n, "tone");
                let duration = child(n, "duration");
                let (left, right) = match tone {
                    Some(tone) => {
                        let carrier = num(tone, "carrier")?.ok_or("tone needs carrier")?;
                        let beat = num(tone, "beat")?.ok_or("tone needs beat")?;
                        (carrier - beat / 2.0, carrier + beat / 2.0)
                    }
                    None if !inherited.is_empty() => (base_left, base_right),
                    None => return Err(format!("preset {name} needs tone")),
                };
                let seconds = match duration {
                    Some(duration) => argument(duration, 0, "duration")?,
                    None if !inherited.is_empty() => base_seconds,
                    None => return Err(format!("preset {name} needs duration")),
                };
                c.presets.push(UserPreset {
                    name,
                    left,
                    right,
                    seconds,
                    audio: audio(n, base_audio)?,
                });
            }
            x => return Err(format!("unknown config node: {x}")),
        }
    }
    Ok(c)
}
fn find(c: &Config, name: &str) -> Result<(f64, f64, f64, Audio), String> {
    if let Some(p) = BUILT_INS.iter().find(|p| p.name == name) {
        return Ok((p.left, p.right, p.seconds, c.audio));
    }
    c.presets
        .iter()
        .find(|p| p.name == name)
        .map(|p| (p.left, p.right, p.seconds, p.audio))
        .ok_or_else(|| format!("unknown preset: {name}"))
}
fn settings(o: &Options, c: &Config) -> Result<(f64, f64, f64, Audio), String> {
    let (pl, pr, pd, mut a) = find(c, o.preset.as_deref().unwrap_or(&c.default))?;
    let ears = o.left.is_some() || o.right.is_some();
    let carrier = o.carrier.is_some() || o.beat.is_some();
    if ears && carrier {
        return Err("use either --left/--right or --carrier/--beat".into());
    }
    let (l, r) = match (o.left, o.right, o.carrier, o.beat) {
        (Some(l), Some(r), _, _) => (l, r),
        (None, None, Some(c), Some(b)) => (c - b / 2., c + b / 2.),
        (None, None, None, None) => (pl, pr),
        (Some(_), None, _, _) | (None, Some(_), _, _) => {
            return Err("use --left and --right together".into());
        }
        _ => return Err("use --carrier and --beat together".into()),
    };
    if let Some(x) = o.volume {
        a.volume = x
    }
    if let Some(x) = o.noise {
        a.noise = x
    }
    if let Some(x) = o.noise_volume {
        a.noise_volume = x
    }
    if let Some(x) = o.fade {
        a.fade = x
    };
    let d = o.seconds.unwrap_or(pd);
    if !(20. ..=1000.0).contains(&l)
        || !(20. ..=1000.0).contains(&r)
        || !(0.1..=40.0).contains(&(r - l).abs())
    {
        return Err("frequencies must be 20..=1000 Hz with 0.1..=40 Hz difference".into());
    }
    if !(0. ..=0.25).contains(&a.volume)
        || !(0. ..=0.25).contains(&a.noise_volume)
        || !(0.0..=MAX_SECONDS).contains(&d)
        || d == 0.
        || !(0.0..=d / 2.0).contains(&a.fade)
    {
        return Err("invalid audio values, duration, or fade".into());
    }
    Ok((l, r, d, a))
}
fn socket_path() -> Result<PathBuf, String> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("bbeats.sock"))
        .ok_or_else(|| "XDG_RUNTIME_DIR is required for daemon mode".into())
}
fn command_options(preset: &str) -> Options {
    Options {
        preset: Some(preset.into()),
        carrier: None,
        beat: None,
        left: None,
        right: None,
        seconds: None,
        volume: None,
        noise: None,
        noise_volume: None,
        fade: None,
    }
}
fn play(sink: &Sink, config: &Config, options: &Options) -> Result<(String, Audio), String> {
    let preset = options
        .preset
        .as_deref()
        .unwrap_or(&config.default)
        .to_owned();
    let (left, right, seconds, audio) = settings(options, config)?;
    sink.clear();
    sink.append(Beat {
        frame: 0,
        frames: (seconds * RATE as f64) as u64,
        left,
        right,
        audio,
        fade_frames: (audio.fade * RATE as f64) as u64,
        channel: false,
        noise_sample: 0.0,
        pink: [0.0; 7],
        brown: 0.0,
        random: 0x9E37_79B9,
    });
    Ok((preset, audio))
}
fn reply(stream: &mut UnixStream, text: &str) -> Result<(), String> {
    writeln!(stream, "{text}").map_err(|error| error.to_string())
}
fn daemon() -> Result<(), String> {
    let config = config()?;
    let path = socket_path()?;
    if path.exists() {
        match UnixStream::connect(&path) {
            Ok(_) => return Err(format!("daemon already running: {}", path.display())),
            Err(_) => fs::remove_file(&path).map_err(|error| error.to_string())?,
        }
    }
    let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let _socket = SocketGuard(path);
    let shutdown = Arc::new(AtomicBool::new(false));
    flag::register(SIGINT, Arc::clone(&shutdown)).map_err(|error| error.to_string())?;
    flag::register(SIGTERM, Arc::clone(&shutdown)).map_err(|error| error.to_string())?;
    let stream = OutputStreamBuilder::open_default_stream().map_err(|error| error.to_string())?;
    let sink = Sink::connect_new(stream.mixer());
    let mut current = command_options(&config.default);
    let (mut preset, mut audio) = play(&sink, &config, &current)?;
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut client, _)) => {
                let mut line = String::new();
                BufReader::new(client.try_clone().map_err(|error| error.to_string())?)
                    .read_line(&mut line)
                    .map_err(|error| error.to_string())?;
                let words: Vec<_> = line.split_whitespace().collect();
                let result = match words.as_slice() {
                    ["status"] => reply(
                        &mut client,
                        &format!(
                            "ok preset={preset} volume={:.2} noise={} noise-volume={:.2} paused={} playing={}",
                            audio.volume,
                            audio.noise.name(),
                            audio.noise_volume,
                            sink.is_paused(),
                            !sink.empty()
                        ),
                    ),
                    ["stop"] => {
                        sink.clear();
                        reply(&mut client, "ok stopped")
                    }
                    ["pause"] => {
                        sink.pause();
                        reply(&mut client, "ok paused")
                    }
                    ["resume"] => {
                        sink.play();
                        reply(&mut client, "ok playing")
                    }
                    ["preset", name] => {
                        current = command_options(name);
                        match play(&sink, &config, &current) {
                            Ok((new_preset, new_audio)) => {
                                preset = new_preset;
                                audio = new_audio;
                                reply(&mut client, "ok")
                            }
                            Err(error) => reply(&mut client, &format!("error {error}")),
                        }
                    }
                    ["volume", value] => match value.parse::<f32>() {
                        Ok(value) if (0.0..=0.25).contains(&value) => {
                            current.volume = Some(value);
                            match play(&sink, &config, &current) {
                                Ok((new_preset, new_audio)) => {
                                    preset = new_preset;
                                    audio = new_audio;
                                    reply(&mut client, "ok")
                                }
                                Err(error) => reply(&mut client, &format!("error {error}")),
                            }
                        }
                        _ => reply(&mut client, "error volume must be 0..=0.25"),
                    },
                    ["noise", kind] | ["noise", kind, _] => match Noise::parse(kind) {
                        Ok(noise) => {
                            current.noise = Some(noise);
                            if let ["noise", _, volume] = words.as_slice() {
                                match volume.parse::<f32>() {
                                    Ok(volume) if (0.0..=0.25).contains(&volume) => {
                                        current.noise_volume = Some(volume)
                                    }
                                    _ => {
                                        return reply(
                                            &mut client,
                                            "error noise volume must be 0..=0.25",
                                        );
                                    }
                                }
                            }
                            match play(&sink, &config, &current) {
                                Ok((new_preset, new_audio)) => {
                                    preset = new_preset;
                                    audio = new_audio;
                                    reply(&mut client, "ok")
                                }
                                Err(error) => reply(&mut client, &format!("error {error}")),
                            }
                        }
                        Err(error) => reply(&mut client, &format!("error {error}")),
                    },
                    ["shutdown"] => {
                        shutdown.store(true, Ordering::Relaxed);
                        reply(&mut client, "ok shutting down")
                    }
                    _ => reply(
                        &mut client,
                        "error commands: status stop pause resume preset NAME volume N noise TYPE [VOLUME] shutdown",
                    ),
                };
                result?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    sink.stop();
    Ok(())
}
struct SocketGuard(PathBuf);
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn message() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(2).collect();
    if args.is_empty() {
        return Err("msg needs a command".into());
    }
    let mut stream =
        UnixStream::connect(socket_path()?).map_err(|_| "daemon is not running".to_string())?;
    writeln!(stream, "{}", args.join(" ")).map_err(|error| error.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|error| error.to_string())?;
    print!("{reply}");
    if reply.starts_with("error") {
        return Err("command failed".into());
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    match env::args().nth(1).as_deref() {
        Some("--daemon") => return daemon().map_err(Into::into),
        Some("msg") => return message().map_err(Into::into),
        _ => {}
    }
    let c = config()?;
    let o = options().map_err(|e| format!("{e}\nTry --help"))?;
    if o.preset.as_deref() == Some("--presets") {
        for p in BUILT_INS {
            println!(
                "{}: {:0.1}/{:0.1} Hz — {}",
                p.name, p.left, p.right, p.description
            );
        }
        for p in &c.presets {
            println!("{}: {:0.1}/{:0.1} Hz", p.name, p.left, p.right);
        }
        return Ok(());
    }
    let preset = o.preset.as_deref().unwrap_or(&c.default);
    let (l, r, d, a) = settings(&o, &c)?;
    eprintln!(
        "[{preset}] left {l:.2} Hz · right {r:.2} Hz · beat {:.2} Hz · {:.0}s · tone {:.2} · noise {} {:.2} · fade {:.0}s",
        (r - l).abs(),
        d,
        a.volume,
        a.noise.name(),
        a.noise_volume,
        a.fade,
    );
    let stream = OutputStreamBuilder::open_default_stream()?;
    let sink = Sink::connect_new(stream.mixer());
    sink.append(Beat {
        frame: 0,
        frames: (d * RATE as f64) as u64,
        left: l,
        right: r,
        audio: a,
        fade_frames: (a.fade * RATE as f64) as u64,
        channel: false,
        noise_sample: 0.,
        pink: [0.; 7],
        brown: 0.,
        random: 0x9E37_79B9,
    });
    sink.sleep_until_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_config_is_calm() {
        let c = Config {
            default: "calm".into(),
            audio: AUDIO,
            presets: vec![],
        };
        let o = Options {
            preset: None,
            carrier: None,
            beat: None,
            left: None,
            right: None,
            seconds: None,
            volume: None,
            noise: None,
            noise_volume: None,
            fade: None,
        };
        assert_eq!(settings(&o, &c).unwrap().0, 195.);
    }
}
