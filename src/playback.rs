use crate::{
    audio::{Beat, PlaybackControl, PlaybackSource},
    config::{self, Audio, Config, Options},
    ipc::Message,
};
use anyhow::{Context, Result, bail};
use rodio::Player;
use std::sync::atomic::{AtomicBool, Ordering};

fn command_options(preset: &str) -> Options {
    Options {
        preset: Some(preset.into()),
        ..Options::default()
    }
}

fn resolve(config: &Config, options: &Options) -> Result<(String, Audio, Beat)> {
    let preset = options
        .preset
        .as_deref()
        .unwrap_or(&config.default)
        .to_owned();
    let (left, right, audio) = config::resolve(options, config)?;
    Ok((preset, audio, Beat::new(left, right, audio)))
}

pub(super) struct PlaybackState {
    current: Options,
    preset: String,
    audio: Audio,
    control: PlaybackControl,
    paused: bool,
    stopped: bool,
}

impl PlaybackState {
    pub(super) fn new(sink: &Player, config: &Config) -> Result<Self> {
        let current = command_options(&config.default);
        let (preset, audio, beat) = resolve(config, &current)?;
        let (control, source) = PlaybackSource::new(beat, true);
        sink.append(source);
        Ok(Self {
            current,
            preset,
            audio,
            control,
            paused: true,
            stopped: false,
        })
    }

    fn replace(&mut self, config: &Config, next: Options, play: bool) -> Result<()> {
        let (preset, audio, beat) = resolve(config, &next)?;
        self.control.replace(beat, play);
        self.current = next;
        self.preset = preset;
        self.audio = audio;
        self.paused = !play;
        self.stopped = false;
        Ok(())
    }

    fn reload(&mut self, config: &Config, play: bool) -> Result<()> {
        self.replace(config, self.current.clone(), play)
    }

    pub(super) fn apply(
        &mut self,
        command: Message,
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
                self.paused,
                !self.paused && !self.stopped
            )),
            Message::Stop => {
                self.control.stop();
                self.paused = true;
                self.stopped = true;
                eprintln!("binaural: stopped");
                Ok("ok stopped".into())
            }
            Message::Pause => {
                if !self.paused && !self.stopped {
                    self.control.pause();
                    self.paused = true;
                    eprintln!("binaural: paused");
                }
                Ok("ok paused".into())
            }
            Message::Play => {
                if self.stopped {
                    self.reload(config, true)?;
                } else if self.paused {
                    self.control.play();
                    self.paused = false;
                }
                eprintln!("binaural: preset={}; playing", self.preset);
                Ok("ok playing".into())
            }
            Message::Preset { name } => {
                self.replace(
                    config,
                    command_options(&name),
                    !self.paused && !self.stopped,
                )?;
                eprintln!(
                    "binaural: preset={}; {}",
                    self.preset,
                    if self.paused { "paused" } else { "playing" }
                );
                Ok("ok".into())
            }
            Message::Volume { value } => {
                if !(0.0..=0.25).contains(&value) {
                    bail!("volume must be 0..=0.25");
                }
                let mut next = self.current.clone();
                next.volume = Some(value);
                self.replace(config, next, !self.paused && !self.stopped)?;
                eprintln!(
                    "binaural: volume={:.2}; {}",
                    self.audio.volume,
                    if self.paused { "paused" } else { "playing" }
                );
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
                self.replace(config, next, !self.paused && !self.stopped)?;
                eprintln!(
                    "binaural: noise={} volume={:.2}; {}",
                    self.audio.noise.as_str(),
                    self.audio.noise_volume,
                    if self.paused { "paused" } else { "playing" }
                );
                Ok("ok".into())
            }
            Message::Reload => {
                let next = config::load().context("reloading configuration")?;
                let play = !self.paused && !self.stopped;
                self.reload(&next, play)?;
                *config = next;
                eprintln!("binaural: configuration reloaded; preset={}", self.preset);
                Ok("ok reloaded".into())
            }
            Message::Shutdown => {
                shutdown.store(true, Ordering::Relaxed);
                eprintln!("binaural: shutdown requested");
                Ok("ok shutting down".into())
            }
        }
    }
}
