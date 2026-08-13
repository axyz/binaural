use crate::{
    audio::{Beat, PlaybackSource},
    config::{self, BUILT_INS, Noise, Options},
    daemon,
    ipc::Message,
};
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use rodio::{DeviceSinkBuilder, Player};

#[derive(Parser)]
#[command(
    version,
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
        }
    }
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
        Some(Command::Daemon) => return daemon::run(),
        Some(Command::Msg { command }) => return daemon::message(command),
        Some(Command::Presets) => return run_presets(),
        None => {}
    }
    let config = config::load()?;
    let options: Options = cli.playback.into();
    let preset = options.preset.as_deref().unwrap_or(&config.default);
    let (left, right, audio) = config::resolve(&options, &config)?;
    eprintln!(
        "[{preset}] left {left:.2} Hz · right {right:.2} Hz · beat {:.2} Hz · tone {:.2} · noise {} {:.2}",
        (right - left).abs(),
        audio.volume,
        audio.noise.as_str(),
        audio.noise_volume,
    );
    let mut stream = DeviceSinkBuilder::open_default_sink()?;
    stream.log_on_drop(false);
    let sink = Player::connect_new(stream.mixer());
    let (_, source) = PlaybackSource::new(Beat::new(left, right, audio), false);
    sink.append(source);
    sink.sleep_until_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_playback_options_with_subcommands() {
        for arguments in [
            ["binaural", "presets", "--volume", "0.2"],
            ["binaural", "--volume", "0.2", "presets"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
    }
}
