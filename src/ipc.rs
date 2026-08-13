use crate::config::Noise;
use anyhow::{Result, anyhow, bail};
use clap::Subcommand;
use std::io::{BufRead, BufReader, Read};

pub(super) const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_MESSAGE_BYTES: usize = 4_096;
const COMMAND_HELP: &str =
    "commands: status stop pause play preset NAME volume N noise TYPE [VOLUME] reload shutdown";

#[derive(Debug, PartialEq, Subcommand)]
pub(super) enum Message {
    Status,
    Stop,
    Pause,
    Play,
    Preset { name: String },
    Volume { value: f32 },
    Noise { kind: Noise, volume: Option<f32> },
    Reload,
    Shutdown,
}

pub(super) fn parse(line: &str) -> Result<Message> {
    let line = line.trim();
    let command_end = line.find(char::is_whitespace).unwrap_or(line.len());
    let (command, arguments) = line.split_at(command_end);
    let arguments = arguments.trim();
    let words: Vec<_> = arguments.split_whitespace().collect();
    match (command, words.as_slice()) {
        ("status", []) => Ok(Message::Status),
        ("stop", []) => Ok(Message::Stop),
        ("pause", []) => Ok(Message::Pause),
        ("play", []) => Ok(Message::Play),
        ("preset", [_, ..]) => Ok(Message::Preset {
            name: arguments.to_owned(),
        }),
        ("volume", [value]) => Ok(Message::Volume {
            value: value
                .parse()
                .map_err(|_| anyhow!("volume must be a number"))?,
        }),
        ("noise", [kind]) => Ok(Message::Noise {
            kind: Noise::parse(kind)?,
            volume: None,
        }),
        ("noise", [kind, volume]) => Ok(Message::Noise {
            kind: Noise::parse(kind)?,
            volume: Some(
                volume
                    .parse()
                    .map_err(|_| anyhow!("noise volume must be a number"))?,
            ),
        }),
        ("reload", []) => Ok(Message::Reload),
        ("shutdown", []) => Ok(Message::Shutdown),
        _ => Err(anyhow!(COMMAND_HELP)),
    }
}

pub(super) fn format(command: Message) -> Result<String> {
    Ok(match command {
        Message::Status => "status".into(),
        Message::Stop => "stop".into(),
        Message::Pause => "pause".into(),
        Message::Play => "play".into(),
        Message::Preset { name } => {
            if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
                bail!(
                    "preset name cannot be empty or contain surrounding whitespace or control characters"
                );
            }
            format!("preset {name}")
        }
        Message::Volume { value } => format!("volume {value}"),
        Message::Noise { kind, volume } => match volume {
            Some(volume) => format!("noise {} {volume}", kind.as_str()),
            None => format!("noise {}", kind.as_str()),
        },
        Message::Reload => "reload".into(),
        Message::Shutdown => "shutdown".into(),
    })
}

pub(super) fn read(reader: impl Read) -> Result<Option<String>> {
    let mut message = String::new();
    let mut reader = BufReader::new(reader).take((MAX_MESSAGE_BYTES + 1) as u64);
    let bytes = reader.read_line(&mut message)?;
    if bytes == 0 {
        return Ok(None);
    }
    if bytes > MAX_MESSAGE_BYTES {
        bail!("IPC message exceeds {MAX_MESSAGE_BYTES} bytes");
    }
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_protocol_round_trips_spaced_preset_and_reload() {
        let encoded = format(Message::Preset {
            name: "late focus".into(),
        })
        .unwrap();
        assert_eq!(
            parse(&encoded).unwrap(),
            Message::Preset {
                name: "late focus".into()
            }
        );
        assert_eq!(parse("reload").unwrap(), Message::Reload);
        assert_eq!(format(Message::Reload).unwrap(), "reload");
        assert!(
            format(Message::Preset {
                name: "focus\nshutdown".into()
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_oversized_ipc_command() {
        let request = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        assert!(read(request.as_slice()).is_err());
    }
}
