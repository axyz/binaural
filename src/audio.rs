use crate::config::{Audio, Noise};
use rodio::{
    Source,
    source::noise::{Brownian, Pink, WhiteUniform},
};
use std::{
    f64::consts::TAU,
    num::{NonZeroU16, NonZeroU32},
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
    fade_frames: u64,
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
            fade_frames: (audio.fade * f64::from(RATE)).ceil() as u64,
            left_channel: true,
            noise_sample: 0.0,
            noise: NoiseSource::new(audio.noise),
        }
    }

    fn gain(&self) -> f32 {
        if self.fade_frames == 0 {
            return 1.0;
        }
        let progress = (self.frame as f64 / self.fade_frames as f64).min(1.0);
        (progress * std::f64::consts::FRAC_PI_2).sin() as f32
    }

    fn sample(&self, frequency: f64) -> f32 {
        (((TAU * frequency * self.frame as f64 / f64::from(RATE)).sin() as f32 * self.audio.volume)
            + self.noise_sample * self.audio.noise_volume)
            * self.gain()
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

    fn total_duration(&self) -> Option<std::time::Duration> {
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
            fade: 0.0,
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
            fade: 0.0,
        };
        let mut beat = Beat::new(200.0, 210.0, audio);
        let samples = [beat.next().unwrap(), beat.next().unwrap()];
        assert_eq!(samples[0].to_bits(), samples[1].to_bits());
    }
}
