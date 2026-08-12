# bbeats

Experimental binaural-beat CLI. Stereo headphones/earbuds required.

```sh
bbeats                       # config default; `calm` on first run
bbeats --preset study
bbeats --preset wind-down --noise brown
bbeats --carrier 400 --beat 8 --seconds 600 --volume 0.08
bbeats --presets
```

## Daemon and IPC

Normal playback starts, plays finite audio, then exits. Use daemon only for live control by keybinds, status bars, or scripts:

```sh
bbeats --daemon
bbeats msg status
bbeats msg preset focus
bbeats msg volume 0.06
bbeats msg noise brown 0.03
bbeats msg pause
bbeats msg resume
bbeats msg stop       # stop playback; daemon remains alive
bbeats msg shutdown   # stop daemon and remove socket
```

Daemon owns `$XDG_RUNTIME_DIR/bbeats.sock`. It refuses a second live daemon, clears a stale socket from a crashed daemon, and removes socket on `shutdown`, `SIGTERM`, or `SIGINT`. `msg` reports error when daemon is absent. Commands are newline-delimited plain text on user-only Unix socket; use `bbeats msg`, not direct access.

`preset`, `volume`, and `noise` restart current audio from start with updated resolved settings. `pause`/`resume` preserve position. `stop` is playback-only.

## Configuration

On first run bbeats creates commented example config at OS config path:

- Linux: `$XDG_CONFIG_HOME/bbeats/config.kdl`, or `~/.config/bbeats/config.kdl`
- macOS: `~/Library/Application Support/bbeats/config.kdl`
- Windows: `%APPDATA%\bbeats\config.kdl`

Empty config retains sane defaults: `calm`, volume `0.10`, noise off, noise volume `0.04`, eight-second fades. CLI flags override config.

```kdl
// Used by bbeats without --preset.
default preset="evening"

// Applies to every built-in preset and is inherited by custom presets.
audio {
  volume 0.10
  fade 8
  noise "off" { volume 0.04 }
}

preset "reading" {
  tone carrier=220 beat=10
  duration 1500
  volume 0.07
}

// Inherit tone, duration, and resolved audio from built-in or earlier custom preset.
preset "evening" inherits="wind-down" {
  fade 12
  noise "brown" { volume 0.03 }
}
```

`carrier` and `beat` are Hz; `beat` is right minus left. `duration`, `fade`, and nested `volume` values use seconds and normalized digital gain. Custom presets inherit global `audio`. `inherits` additionally copies tone and duration from a built-in or earlier custom preset; child settings override inherited values. Invalid or unknown config nodes stop startup with an error.

## Built-ins

| Preset | Parameters | Routine, not promise |
| --- | --- | --- |
| `calm` | 195/205 Hz; 10 Hz; 10 min | Quiet-break routine. Alpha is common label, not guaranteed outcome. |
| `study` | 400/415 Hz; 15 Hz; 25 min | Study/work routine. Frequencies match preliminary adult-ADHD study; not treatment. |
| `focus` | 113/127 Hz; 14 Hz; 25 min | Work-block routine. |
| `wind-down` | 197/203 Hz; 6 Hz; 15 min | Pre-rest routine. No sleep-induction claim. |

## Noise and safety

Noise defaults off. Masking noise was not necessary in meta-analysis and may add sensory load. Opt in with `--noise white|pink|brown` or per-preset config. `--noise-volume` defaults to `0.04`.

Start low. Stop for headache, dizziness, nausea, tinnitus, distress, or altered hearing. Do not use while driving, operating machinery, or needing alertness. Do not use this as treatment or replace care.

## Sources

- [PLOS systematic review (2023)](https://doi.org/10.1371/journal.pone.0286023)
- [Meta-analysis (2019)](https://pubmed.ncbi.nlm.nih.gov/30073406/)
- [Pilot adult-ADHD randomized controlled trial](https://www.cambridge.org/core/journals/european-psychiatry/article/pilot-addon-randomizedcontrolled-trial-evaluating-the-effect-of-binaural-beats-on-study-performance-mindwandering-and-core-symptoms-of-adult-adhd-patients/33084BDFD5C5EB2B838AA0C147F54C19)
- [WHO: Safe listening](https://www.who.int/news-room/questions-and-answers/item/deafness-and-hearing-loss-safe-listening)
