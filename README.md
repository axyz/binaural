# binaural

Binaural-beat player for Linux and macOS. Stereo headphones required. Windows is not supported.

## Install

Requires Rust 1.95 or newer.

```sh
cargo install binaural --locked
```

Linux build dependencies:

```sh
# Debian/Ubuntu
sudo apt install pkg-config libasound2-dev

# Fedora
sudo dnf install pkgconf-pkg-config alsa-lib-devel
```

[Release binaries](https://github.com/axyz/binaural/releases)

## Usage

```sh
binaural
binaural --preset study
binaural --preset wind-down --noise brown
binaural --carrier 400 --beat 8 --volume 0.08
binaural presets
```

Run `binaural --help` for all options.

## Daemon

```sh
binaural daemon
binaural msg status
binaural msg preset focus
binaural msg volume 0.06
binaural msg noise brown 0.03
binaural msg pause
binaural msg play
binaural msg stop
binaural msg shutdown
```

Preset, volume, and noise changes load paused. Run `binaural msg play` to resume.

## Configuration

Config file:

- Linux: `$XDG_CONFIG_HOME/binaural/config.kdl` or `~/.config/binaural/config.kdl`
- macOS: `~/Library/Application Support/dev.binaural.binaural/config.kdl`

```kdl
default preset="evening"

audio {
  volume 0.10
  fade 8
  noise "off" { volume 0.04 }
}

preset "reading" {
  tone carrier=220 beat=10
  volume 0.07
}
```

CLI flags override config values. Frequencies use Hz, volume uses normalized gain, and fade uses seconds.

## Presets

| Preset | Left/right | Beat |
| --- | --- | --- |
| `calm` | 195/205 Hz | 10 Hz |
| `study` | 400/415 Hz | 15 Hz |
| `focus` | 113/127 Hz | 14 Hz |
| `wind-down` | 197/203 Hz | 6 Hz |

## Safety

Start at low volume. Stop if you experience headache, dizziness, nausea, tinnitus, distress, or altered hearing. Do not use while driving or operating machinery. This is not medical treatment.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

## References

- [PLOS systematic review (2023)](https://doi.org/10.1371/journal.pone.0286023)
- [Meta-analysis (2019)](https://pubmed.ncbi.nlm.nih.gov/30073406/)
- [Pilot adult-ADHD randomized controlled trial](https://www.cambridge.org/core/journals/european-psychiatry/article/pilot-addon-randomizedcontrolled-trial-evaluating-the-effect-of-binaural-beats-on-study-performance-mindwandering-and-core-symptoms-of-adult-adhd-patients/33084BDFD5C5EB2B838AA0C147F54C19)
- [WHO safe listening](https://www.who.int/news-room/questions-and-answers/item/deafness-and-hearing-loss-safe-listening)
