# binaural

Experimental binaural-beat CLI for Unix-like systems. Stereo headphones/earbuds required.

## Install

Cargo builds from source and requires Rust 1.95 or newer. Linux also needs ALSA development files:

```sh
# Debian/Ubuntu
sudo apt install pkg-config libasound2-dev

# Fedora
sudo dnf install pkgconf-pkg-config alsa-lib-devel

cargo install binaural --locked
```

Install directly from GitHub before the first crates.io release:

```sh
cargo install --git https://github.com/axyz/binaural --locked
```

Prebuilt archives and checksums are attached to [GitHub Releases](https://github.com/axyz/binaural/releases).

## Platforms

| Platform | Audio backend | Status |
| --- | --- | --- |
| Linux x86_64/ARM64 | ALSA | Supported |
| macOS Intel/Apple silicon | CoreAudio | Supported |
| Windows | — | Not supported; IPC uses Unix sockets |

## Usage

```sh
binaural                       # config default; `calm` on first run
binaural --preset study
binaural --preset wind-down --noise brown
binaural --carrier 400 --beat 8 --volume 0.08
binaural presets
```

## Daemon and IPC

Normal playback continues until interrupted. Use daemon for live control by keybinds, status bars, or scripts:

```sh
binaural daemon
binaural msg status
binaural msg preset focus
binaural msg volume 0.06
binaural msg noise brown 0.03
binaural msg reload    # validate config, then reload current preset
binaural msg pause
binaural msg play
binaural msg stop       # stop playback; daemon remains alive
binaural msg shutdown   # stop daemon and remove socket
```

Daemon owns `$XDG_RUNTIME_DIR/binaural.sock`. On macOS without `XDG_RUNTIME_DIR`, it uses `$TMPDIR/binaural.sock`. It refuses a second live daemon, clears a stale socket from a crashed daemon, and removes socket on `shutdown`, `SIGTERM`, or `SIGINT`. `msg` reports error when daemon is absent. Commands are newline-delimited plain text on a mode-`0600` Unix socket; use `binaural msg`, not direct access.

Daemon loads config at startup. `reload` rereads and validates config, keeps previous config on failure, then reloads current preset; it fails if preset was removed. `preset` loads and starts playback. `volume` and `noise` load updated audio paused; use `play` to begin. `pause`/`play` preserve position; `stop` clears playback only.

## Configuration

On first run binaural creates commented example config at OS config path:

- Linux: `$XDG_CONFIG_HOME/binaural/config.kdl`, or `~/.config/binaural/config.kdl`
- macOS: `~/Library/Application Support/dev.binaural.binaural/config.kdl`

Empty config retains sane defaults: `calm`, volume `0.10`, noise off, noise volume `0.04`, eight-second fades. CLI flags override config.

```kdl
// Used by binaural without --preset.
default preset="evening"

// Applies to every built-in preset and is inherited by custom presets.
audio {
  volume 0.10
  fade 8
  noise "off" { volume 0.04 }
}

preset "reading" {
  tone carrier=220 beat=10
  volume 0.07
}

// Inherit tone and resolved audio from built-in or earlier custom preset.
preset "evening" inherits="wind-down" {
  fade 12
  noise "brown" { volume 0.03 }
}
```

`carrier` and `beat` are Hz; `beat` is right minus left. `fade` and nested `volume` values use seconds and normalized digital gain. Custom presets inherit global `audio`. `inherits` additionally copies tone from a built-in or earlier custom preset; child settings override inherited values. Invalid or unknown config nodes stop startup with an error.

## Built-ins

| Preset | Parameters | Routine, not promise |
| --- | --- | --- |
| `calm` | 195/205 Hz; 10 Hz | Quiet-break routine. Alpha is common label, not guaranteed outcome. |
| `study` | 400/415 Hz; 15 Hz | Study/work routine. Frequencies match preliminary adult-ADHD study; not treatment. |
| `focus` | 113/127 Hz; 14 Hz | Work-block routine. |
| `wind-down` | 197/203 Hz; 6 Hz | Pre-rest routine. No sleep-induction claim. |

## Noise and safety

Noise defaults off. Masking noise was not necessary in meta-analysis and may add sensory load. Opt in with `--noise white|pink|brown` or per-preset config. `--noise-volume` defaults to `0.04`.

Start low. Stop for headache, dizziness, nausea, tinnitus, distress, or altered hearing. Do not use while driving, operating machinery, or needing alertness. Do not use this as treatment or replace care.

## Development

Required checks:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

CI runs these checks on Linux, tests macOS, and verifies the declared minimum Rust version. Maintainers: see [RELEASING.md](RELEASING.md).

## Sources

- [PLOS systematic review (2023)](https://doi.org/10.1371/journal.pone.0286023)
- [Meta-analysis (2019)](https://pubmed.ncbi.nlm.nih.gov/30073406/)
- [Pilot adult-ADHD randomized controlled trial](https://www.cambridge.org/core/journals/european-psychiatry/article/pilot-addon-randomizedcontrolled-trial-evaluating-the-effect-of-binaural-beats-on-study-performance-mindwandering-and-core-symptoms-of-adult-adhd-patients/33084BDFD5C5EB2B838AA0C147F54C19)
- [WHO: Safe listening](https://www.who.int/news-room/questions-and-answers/item/deafness-and-hearing-loss-safe-listening)
