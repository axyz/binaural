use crate::config::{Audio, Noise};
use rodio::{
    Source,
    source::noise::{Brownian, Pink, WhiteUniform},
};
use std::{
    f64::consts::{FRAC_PI_2, TAU},
    num::{NonZeroU16, NonZeroU32},
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

const RATE: u32 = 44_100;
const SAMPLE_RATE: NonZeroU32 = NonZeroU32::new(RATE).unwrap();
const CHANNELS: NonZeroU16 = NonZeroU16::new(2).unwrap();

enum NoiseSource {
    Off,
    White(WhiteUniform),
    Pink(Pink),
    Brown(Brownian),
}

impl NoiseSource {
    fn new(noise: Noise) -> Self {
        match noise {
            Noise::Off => Self::Off,
            Noise::White => Self::White(WhiteUniform::new(SAMPLE_RATE)),
            Noise::Pink => Self::Pink(Pink::new(SAMPLE_RATE)),
            Noise::Brown => Self::Brown(Brownian::new(SAMPLE_RATE)),
        }
    }

    fn next_sample(&mut self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::White(source) => source.next().unwrap_or_default(),
            Self::Pink(source) => source.next().unwrap_or_default(),
            Self::Brown(source) => source.next().unwrap_or_default(),
        }
    }
}

pub(super) struct Beat {
    frame: u64,
    left: f64,
    right: f64,
    audio: Audio,
    left_channel: bool,
    noise_sample: f32,
    noise: NoiseSource,
}

impl Beat {
    pub(super) fn new(left: f64, right: f64, audio: Audio) -> Self {
        Self {
            frame: 0,
            left,
            right,
            audio,
            left_channel: true,
            noise_sample: 0.0,
            noise: NoiseSource::new(audio.noise),
        }
    }

    fn sample(&self, frequency: f64) -> f32 {
        ((TAU * frequency * self.frame as f64 / f64::from(RATE)).sin() as f32 * self.audio.volume)
            + self.noise_sample * self.audio.noise_volume
    }
}

impl Iterator for Beat {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.left_channel {
            self.noise_sample = self.noise.next_sample();
            self.left_channel = false;
            Some(self.sample(self.left))
        } else {
            self.left_channel = true;
            let sample = self.sample(self.right);
            self.frame += 1;
            Some(sample)
        }
    }
}

impl Source for Beat {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        CHANNELS
    }

    fn sample_rate(&self) -> NonZeroU32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

const FADE_FRAMES: usize = RATE as usize / 2;

pub(super) struct PlaybackControl(Sender<Command>);

enum Command {
    Play,
    Pause,
    Stop,
    Replace { beat: Box<Beat>, play: bool },
}

impl PlaybackControl {
    pub(super) fn play(&self) {
        let _ = self.0.send(Command::Play);
    }

    pub(super) fn pause(&self) {
        let _ = self.0.send(Command::Pause);
    }

    pub(super) fn stop(&self) {
        let _ = self.0.send(Command::Stop);
    }

    pub(super) fn replace(&self, beat: Beat, play: bool) {
        let _ = self.0.send(Command::Replace {
            beat: Box::new(beat),
            play,
        });
    }
}

pub(super) struct PlaybackSource {
    current: Option<Beat>,
    outgoing: Option<Beat>,
    receiver: Receiver<Command>,
    transition: Transition,
    paused: bool,
    samples: [f32; 2],
    next_sample: usize,
}

enum Transition {
    Steady,
    FadeIn {
        frame: usize,
    },
    FadeOut {
        frame: usize,
        next: Option<Box<Beat>>,
        stop: bool,
    },
    Crossfade {
        frame: usize,
    },
}

impl PlaybackSource {
    pub(super) fn new(beat: Beat, paused: bool) -> (PlaybackControl, Self) {
        let (sender, receiver) = mpsc::channel();
        (
            PlaybackControl(sender),
            Self {
                current: Some(beat),
                outgoing: None,
                receiver,
                transition: Transition::Steady,
                paused,
                samples: [0.0; 2],
                next_sample: 0,
            },
        )
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Play if self.paused && self.current.is_some() => {
                self.paused = false;
                self.transition = Transition::FadeIn { frame: 0 };
            }
            Command::Pause if !self.paused && self.current.is_some() => {
                self.outgoing = None;
                self.transition = Transition::FadeOut {
                    frame: 0,
                    next: None,
                    stop: false,
                };
            }
            Command::Stop if self.current.is_some() => {
                self.outgoing = None;
                self.transition = Transition::FadeOut {
                    frame: 0,
                    next: None,
                    stop: true,
                };
            }
            Command::Replace { beat, play } if play && !self.paused && self.current.is_some() => {
                self.outgoing = self.current.replace(*beat);
                self.transition = Transition::Crossfade { frame: 0 };
            }
            Command::Replace { beat, play: false } if !self.paused && self.current.is_some() => {
                self.outgoing = None;
                self.transition = Transition::FadeOut {
                    frame: 0,
                    next: Some(beat),
                    stop: false,
                };
            }
            Command::Replace { beat, play } => {
                self.current = Some(*beat);
                self.outgoing = None;
                self.paused = !play;
                self.transition = if play {
                    Transition::FadeIn { frame: 0 }
                } else {
                    Transition::Steady
                };
            }
            _ => {}
        }
    }

    fn next_frame(&mut self) -> [f32; 2] {
        while let Ok(command) = self.receiver.try_recv() {
            self.command(command);
        }
        if self.paused {
            return [0.0; 2];
        }
        let Some(current) = self.current.as_mut() else {
            return [0.0; 2];
        };
        let current = [current.next().unwrap(), current.next().unwrap()];
        match &mut self.transition {
            Transition::Steady => current,
            Transition::FadeIn { frame } => {
                let gain = (*frame as f64 / FADE_FRAMES as f64 * FRAC_PI_2).sin() as f32;
                *frame += 1;
                if *frame >= FADE_FRAMES {
                    self.transition = Transition::Steady;
                }
                [current[0] * gain, current[1] * gain]
            }
            Transition::FadeOut { frame, next, stop } => {
                let gain = (*frame as f64 / FADE_FRAMES as f64 * FRAC_PI_2).cos() as f32;
                *frame += 1;
                if *frame >= FADE_FRAMES {
                    if let Some(next) = next.take() {
                        self.current = Some(*next);
                    } else if *stop {
                        self.current = None;
                    }
                    self.paused = true;
                    self.transition = Transition::Steady;
                }
                [current[0] * gain, current[1] * gain]
            }
            Transition::Crossfade { frame } => {
                let outgoing = self
                    .outgoing
                    .as_mut()
                    .expect("crossfade needs outgoing source");
                let outgoing = [outgoing.next().unwrap(), outgoing.next().unwrap()];
                let angle = *frame as f64 / FADE_FRAMES as f64 * FRAC_PI_2;
                *frame += 1;
                if *frame >= FADE_FRAMES {
                    self.outgoing = None;
                    self.transition = Transition::Steady;
                }
                let incoming_gain = angle.sin() as f32;
                let outgoing_gain = angle.cos() as f32;
                [
                    current[0] * incoming_gain + outgoing[0] * outgoing_gain,
                    current[1] * incoming_gain + outgoing[1] * outgoing_gain,
                ]
            }
        }
    }
}

impl Iterator for PlaybackSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.next_sample == 0 {
            self.samples = self.next_frame();
        }
        let sample = self.samples[self.next_sample];
        self.next_sample = (self.next_sample + 1) % 2;
        Some(sample)
    }
}

impl Source for PlaybackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        CHANNELS
    }

    fn sample_rate(&self) -> NonZeroU32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yields_complete_interleaved_frames() {
        let audio = Audio {
            volume: 1.0,
            noise: Noise::Off,
            noise_volume: 0.0,
        };
        let mut beat = Beat::new(f64::from(RATE) / 4.0, f64::from(RATE) / 2.0, audio);
        let samples: [f32; 4] = std::array::from_fn(|_| beat.next().unwrap());
        assert_eq!(samples.len(), 4);
        assert!(samples[0].abs() < f32::EPSILON);
        assert!(samples[1].abs() < f32::EPSILON);
        assert!(samples[2] > 0.999);
        assert!(samples[3].abs() < 0.000_001);
    }

    #[test]
    fn shares_noise_across_each_stereo_frame() {
        let audio = Audio {
            volume: 0.0,
            noise: Noise::White,
            noise_volume: 1.0,
        };
        let mut beat = Beat::new(200.0, 210.0, audio);
        let samples = [beat.next().unwrap(), beat.next().unwrap()];
        assert_eq!(samples[0].to_bits(), samples[1].to_bits());
    }

    #[test]
    fn playback_control_fades_per_frame() {
        let audio = Audio {
            volume: 1.0,
            noise: Noise::Off,
            noise_volume: 0.0,
        };
        let (control, mut source) = PlaybackSource::new(Beat::new(11_025.0, 11_025.0, audio), true);
        control.play();
        assert!(source.next().unwrap().abs() < f32::EPSILON);
        let samples: Vec<_> = (0..(FADE_FRAMES * 2 + 16))
            .map(|_| source.next().unwrap().abs())
            .collect();
        assert!(samples[..FADE_FRAMES].iter().any(|sample| *sample > 0.1));
        assert!(
            samples[FADE_FRAMES * 2..]
                .iter()
                .any(|sample| *sample > 0.99)
        );
    }

    #[test]
    fn playback_control_crossfades_replacement() {
        let audio = Audio {
            volume: 1.0,
            noise: Noise::Off,
            noise_volume: 0.0,
        };
        let (control, mut source) = PlaybackSource::new(Beat::new(200.0, 210.0, audio), false);
        control.replace(Beat::new(300.0, 310.0, audio), true);
        source.next();
        assert!(matches!(source.transition, Transition::Crossfade { .. }));
        for _ in 0..FADE_FRAMES * 2 {
            source.next();
        }
        assert!(matches!(source.transition, Transition::Steady));
        assert!(source.outgoing.is_none());
    }
}
