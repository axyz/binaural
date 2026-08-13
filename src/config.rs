use anyhow::{Context, Result, anyhow};
use clap::ValueEnum;
use directories::ProjectDirs;
use kdl::{KdlDocument, KdlNode, KdlValue};
use std::{fs, path::PathBuf};

const EXAMPLE: &str = r#"// All settings optional. `binaural` uses `default` when no preset is selected.
default preset="evening"

// Global audio settings. Custom presets inherit these unless they override them.
audio {
  volume 0.10
  noise "off" { volume 0.04 }
}

// A custom preset needs a tone unless it inherits one.
preset "reading" {
  tone carrier=220 beat=10
  volume 0.07
}

// `inherits` refers to a built-in or earlier custom preset.
preset "evening" inherits="wind-down" {
  noise "brown" { volume 0.03 }
}
"#;

#[derive(clap::ValueEnum, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Noise {
    #[default]
    Off,
    White,
    Pink,
    Brown,
}

impl Noise {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::White => "white",
            Self::Pink => "pink",
            Self::Brown => "brown",
        }
    }
    pub(super) fn parse(s: &str) -> Result<Self> {
        Self::from_str(s, false).map_err(|_| anyhow!("noise must be off, white, pink, or brown"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Audio {
    pub(super) volume: f32,
    pub(super) noise: Noise,
    pub(super) noise_volume: f32,
}
const AUDIO: Audio = Audio {
    volume: 0.10,
    noise: Noise::Off,
    noise_volume: 0.04,
};

pub(super) struct Preset {
    pub(super) name: &'static str,
    pub(super) left: f64,
    pub(super) right: f64,
    pub(super) description: &'static str,
}

pub(super) const BUILT_INS: &[Preset] = &[
    Preset {
        name: "calm",
        left: 195.,
        right: 205.,
        description: "10 Hz, alpha; quiet-break routine",
    },
    Preset {
        name: "study",
        left: 400.,
        right: 415.,
        description: "15 Hz, beta; study routine",
    },
    Preset {
        name: "focus",
        left: 113.,
        right: 127.,
        description: "14 Hz, beta; work-block routine",
    },
    Preset {
        name: "wind-down",
        left: 197.,
        right: 203.,
        description: "6 Hz, theta; pre-rest routine",
    },
];

pub(super) struct UserPreset {
    pub(super) name: String,
    pub(super) left: f64,
    pub(super) right: f64,
    pub(super) audio: Audio,
}

pub(super) struct Config {
    pub(super) default: String,
    pub(super) audio: Audio,
    pub(super) presets: Vec<UserPreset>,
}

#[derive(Clone, Default)]
pub(super) struct Options {
    pub(super) preset: Option<String>,
    pub(super) carrier: Option<f64>,
    pub(super) beat: Option<f64>,
    pub(super) left: Option<f64>,
    pub(super) right: Option<f64>,
    pub(super) volume: Option<f32>,
    pub(super) noise: Option<Noise>,
    pub(super) noise_volume: Option<f32>,
}

fn config_path() -> Result<PathBuf> {
    ProjectDirs::from("dev", "binaural", "binaural")
        .map(|dirs| dirs.config_dir().join("config.kdl"))
        .ok_or_else(|| anyhow!("cannot determine config directory"))
}

fn number(node: &KdlNode, key: &str) -> Result<Option<f64>> {
    let Some(value) = node.get(key) else {
        return Ok(None);
    };
    let number = match value {
        KdlValue::Integer(value) => *value as f64,
        KdlValue::Float(value) => *value,
        _ => return Err(anyhow!("{key} must be a number")),
    };
    number
        .is_finite()
        .then_some(Some(number))
        .ok_or_else(|| anyhow!("{key} must be finite"))
}

fn string<'a>(node: &'a KdlNode, key: &str) -> Result<Option<&'a str>> {
    node.get(key)
        .map(|value| {
            value
                .as_string()
                .ok_or_else(|| anyhow!("{key} must be a string"))
        })
        .transpose()
}

fn child<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlNode> {
    node.children()?
        .nodes()
        .iter()
        .find(|child| child.name().value() == name)
}

fn argument(node: &KdlNode, index: usize, name: &str) -> Result<f64> {
    match node.get(index) {
        Some(KdlValue::Integer(value)) => Ok(*value as f64),
        Some(KdlValue::Float(value)) if value.is_finite() => Ok(*value),
        _ => Err(anyhow!("{name} needs a finite number")),
    }
}

fn volume(node: &KdlNode, name: &str) -> Result<f32> {
    let value = argument(node, 0, name)?;
    if !(0.0..=0.25).contains(&value) {
        return Err(anyhow!("{name} must be 0..=0.25"));
    }
    Ok(value as f32)
}

fn validate_node(
    node: &KdlNode,
    expected_arguments: usize,
    allowed_properties: &[&str],
    allowed_children: &[&str],
) -> Result<()> {
    let node_name = node.name().value();
    let mut arguments = 0;
    for (index, entry) in node.entries().iter().enumerate() {
        let Some(name) = entry.name().map(|name| name.value()) else {
            arguments += 1;
            continue;
        };
        if !allowed_properties.contains(&name) {
            return Err(anyhow!("unknown {node_name} property: {name}"));
        }
        if node.entries()[..index]
            .iter()
            .any(|previous| previous.name().is_some_and(|other| other.value() == name))
        {
            return Err(anyhow!("duplicate {node_name} property: {name}"));
        }
    }
    if arguments != expected_arguments {
        return Err(anyhow!(
            "{node_name} expects {expected_arguments} positional argument(s), found {arguments}"
        ));
    }
    if let Some(document) = node.children() {
        for (index, nested) in document.nodes().iter().enumerate() {
            let name = nested.name().value();
            if !allowed_children.contains(&name) {
                return Err(anyhow!("unknown {node_name} child: {name}"));
            }
            if document.nodes()[..index]
                .iter()
                .any(|previous| previous.name().value() == name)
            {
                return Err(anyhow!("duplicate {node_name} child: {name}"));
            }
        }
    }
    Ok(())
}

fn validate_audio_nodes(node: &KdlNode) -> Result<()> {
    let Some(document) = node.children() else {
        return Ok(());
    };
    for nested in document.nodes() {
        match nested.name().value() {
            "volume" => validate_node(nested, 1, &[], &[])?,
            "noise" => {
                validate_node(nested, 1, &[], &["volume"])?;
                if let Some(volume) = child(nested, "volume") {
                    validate_node(volume, 1, &[], &[])?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_audio(node: &KdlNode, base: Audio) -> Result<Audio> {
    validate_audio_nodes(node)?;
    let mut audio = base;
    if let Some(node) = child(node, "volume") {
        audio.volume = volume(node, "volume")?;
    }
    if let Some(node) = child(node, "noise") {
        audio.noise = node
            .get(0)
            .and_then(KdlValue::as_string)
            .map(Noise::parse)
            .transpose()?
            .ok_or_else(|| anyhow!("noise needs type"))?;
        if let Some(volume_node) = child(node, "volume") {
            audio.noise_volume = volume(volume_node, "noise volume")?;
        }
    }
    if !(0.0..=0.25).contains(&audio.volume) || !(0.0..=0.25).contains(&audio.noise_volume) {
        return Err(anyhow!("audio values out of range"));
    }
    Ok(audio)
}

fn validate_playback(left: f64, right: f64, audio: Audio) -> Result<()> {
    if !(20.0..=1000.0).contains(&left)
        || !(20.0..=1000.0).contains(&right)
        || !(0.1..=40.0).contains(&(right - left).abs())
    {
        return Err(anyhow!(
            "frequencies must be 20..=1000 Hz with 0.1..=40 Hz difference"
        ));
    }
    if !(0.0..=0.25).contains(&audio.volume) || !(0.0..=0.25).contains(&audio.noise_volume) {
        return Err(anyhow!("volume values must be 0..=0.25"));
    }
    Ok(())
}

fn parse_config(text: &str) -> Result<Config> {
    let document: KdlDocument = text
        .parse()
        .map_err(|error| anyhow!("invalid KDL: {error}"))?;
    let mut config = Config {
        default: "calm".into(),
        audio: AUDIO,
        presets: vec![],
    };
    let mut has_default = false;
    let mut has_audio = false;

    for node in document.nodes() {
        match node.name().value() {
            "default" => {
                validate_node(node, 0, &["preset"], &[])?;
                if has_default {
                    return Err(anyhow!("duplicate default node"));
                }
                has_default = true;
                config.default = string(node, "preset")?
                    .ok_or_else(|| anyhow!("default needs preset"))?
                    .into();
            }
            "audio" => {
                validate_node(node, 0, &[], &["volume", "noise"])?;
                if has_audio {
                    return Err(anyhow!("duplicate audio node"));
                }
                has_audio = true;
                config.audio = parse_audio(node, AUDIO)?;
            }
            "preset" => {
                validate_node(node, 1, &["inherits"], &["tone", "volume", "noise"])?;
                validate_audio_nodes(node)?;
                if let Some(tone) = child(node, "tone") {
                    validate_node(tone, 0, &["carrier", "beat"], &[])?;
                }
            }
            name => return Err(anyhow!("unknown config node: {name}")),
        }
    }

    for preset in BUILT_INS {
        validate_playback(preset.left, preset.right, config.audio)
            .with_context(|| format!("invalid built-in preset: {}", preset.name))?;
    }

    for node in document
        .nodes()
        .iter()
        .filter(|node| node.name().value() == "preset")
    {
        let name = node
            .get(0)
            .and_then(KdlValue::as_string)
            .ok_or_else(|| anyhow!("preset needs name"))?
            .to_owned();
        if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
            return Err(anyhow!(
                "preset name cannot be empty or contain surrounding whitespace or control characters"
            ));
        }
        if BUILT_INS.iter().any(|preset| preset.name == name)
            || config.presets.iter().any(|preset| preset.name == name)
        {
            return Err(anyhow!("duplicate or built-in preset: {name}"));
        }
        let inherited = string(node, "inherits")?.unwrap_or("");
        let (base_left, base_right, base_audio) = if inherited.is_empty() {
            (0.0, 0.0, config.audio)
        } else {
            resolve_preset(&config, inherited)?
        };
        let (left, right) = match child(node, "tone") {
            Some(tone) => {
                let carrier =
                    number(tone, "carrier")?.ok_or_else(|| anyhow!("tone needs carrier"))?;
                let beat = number(tone, "beat")?.ok_or_else(|| anyhow!("tone needs beat"))?;
                (carrier - beat / 2.0, carrier + beat / 2.0)
            }
            None if !inherited.is_empty() => (base_left, base_right),
            None => return Err(anyhow!("preset {name} needs tone")),
        };
        let audio = parse_audio(node, base_audio)?;
        validate_playback(left, right, audio).with_context(|| format!("invalid preset: {name}"))?;
        config.presets.push(UserPreset {
            name,
            left,
            right,
            audio,
        });
    }

    resolve_preset(&config, &config.default)
        .with_context(|| format!("invalid default preset: {}", config.default))?;
    Ok(config)
}

pub(super) fn load() -> Result<Config> {
    let path = config_path()?;
    if !path
        .try_exists()
        .with_context(|| format!("inspecting config path: {}", path.display()))?
    {
        let directory = path
            .parent()
            .ok_or_else(|| anyhow!("invalid config path: {}", path.display()))?;
        fs::create_dir_all(directory)
            .with_context(|| format!("creating config directory for {}", path.display()))?;
        fs::write(&path, EXAMPLE).with_context(|| format!("creating {}", path.display()))?;
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse_config(&text).with_context(|| format!("parsing {}", path.display()))
}

fn resolve_preset(config: &Config, name: &str) -> Result<(f64, f64, Audio)> {
    if let Some(preset) = BUILT_INS.iter().find(|preset| preset.name == name) {
        return Ok((preset.left, preset.right, config.audio));
    }
    config
        .presets
        .iter()
        .find(|preset| preset.name == name)
        .map(|preset| (preset.left, preset.right, preset.audio))
        .ok_or_else(|| anyhow!("unknown preset: {name}"))
}

pub(super) fn resolve(options: &Options, config: &Config) -> Result<(f64, f64, Audio)> {
    let (preset_left, preset_right, mut audio) =
        resolve_preset(config, options.preset.as_deref().unwrap_or(&config.default))?;
    let uses_ear_frequencies = options.left.is_some() || options.right.is_some();
    let uses_carrier = options.carrier.is_some() || options.beat.is_some();
    if uses_ear_frequencies && uses_carrier {
        return Err(anyhow!("use either --left/--right or --carrier/--beat"));
    }
    let (left, right) = match (options.left, options.right, options.carrier, options.beat) {
        (Some(left), Some(right), _, _) => (left, right),
        (None, None, Some(carrier), Some(beat)) => (carrier - beat / 2.0, carrier + beat / 2.0),
        (None, None, None, None) => (preset_left, preset_right),
        (Some(_), None, _, _) | (None, Some(_), _, _) => {
            return Err(anyhow!("use --left and --right together"));
        }
        _ => return Err(anyhow!("use --carrier and --beat together")),
    };
    if let Some(volume) = options.volume {
        audio.volume = volume;
    }
    if let Some(noise) = options.noise {
        audio.noise = noise;
    }
    if let Some(noise_volume) = options.noise_volume {
        audio.noise_volume = noise_volume;
    }
    validate_playback(left, right, audio)?;
    Ok((left, right, audio))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_preset() {
        let config = parse_config("").unwrap();
        assert!((resolve(&Options::default(), &config).unwrap().0 - 195.0).abs() < f64::EPSILON);
    }

    #[test]
    fn global_audio_is_order_independent() {
        let config = parse_config(
            r#"
                preset "reading" {
                    tone carrier=220 beat=10
                }
                audio { volume 0.2 }
            "#,
        )
        .unwrap();
        assert!((config.presets[0].audio.volume - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_invalid_config() {
        let invalid = [
            "audio { volum 0.1 }",
            r#"preset "bad" { tone carrier=220 beat=10 typo=1 }"#,
            "audio { volume 0.1; volume 0.2 }",
            "audio { volume 0.2500000001 }",
            r#"preset "bad" { tone carrier=10 beat=2 }"#,
            r#"preset "bad" { tone carrier=220 beat=10; duration 600 }"#,
            r#"default preset="missing""#,
        ];
        for config in invalid {
            assert!(
                parse_config(config).is_err(),
                "accepted invalid config: {config}"
            );
        }
    }
}
