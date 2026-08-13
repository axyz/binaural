# binaural

Binaural-beat player for Linux and macOS. Stereo headphones required.

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
// Used when no --preset is passed.
default preset="evening"

// Defaults for every preset.
audio {
  volume 0.10
  noise "off" { volume 0.04 }
}

// New tone, inheriting global audio defaults.
preset "reading" {
  tone carrier=220 beat=10
  volume 0.07
}

// Inherit tone and audio from a built-in or earlier custom preset.
preset "evening" inherits="wind-down" {
  noise "brown" { volume 0.03 }
}
```

`default` selects startup preset. `audio` supplies global `volume`, `noise`, and noise `volume`. A `preset` either defines `tone carrier=… beat=…` or `inherits` a built-in or earlier custom preset; preset audio settings override inherited values. Noise types: `off`, `white`, `pink`, `brown`.

CLI flags override config values. Frequencies use Hz and volume uses normalized gain.

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
