use crate::{
    audio::{Beat, PlaybackControl, PlaybackSource},
    config::{self, Audio, Config, Options},
};
use anyhow::{Context, Result};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Paragraph, Row, Table},
};
use rodio::{DeviceSinkBuilder, Player};
use std::time::Duration;

const FRAME_TIME: Duration = Duration::from_millis(100);

fn band(beat: f64) -> &'static str {
    match beat {
        ..4.0 => "delta",
        ..8.0 => "theta",
        ..13.0 => "alpha",
        ..30.0 => "beta",
        _ => "gamma",
    }
}

fn preset_names(config: &Config) -> Vec<String> {
    config::BUILT_INS
        .iter()
        .map(|preset| preset.name.into())
        .chain(config.presets.iter().map(|preset| preset.name.clone()))
        .collect()
}

fn options(name: String) -> Options {
    Options {
        preset: Some(name),
        ..Options::default()
    }
}

struct App {
    names: Vec<String>,
    selected: usize,
    preset: String,
    left: f64,
    right: f64,
    audio: Audio,
    paused: bool,
}

fn render(frame: &mut ratatui::Frame, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(6),
        Constraint::Length(3),
    ])
    .split(frame.area());
    let beat = (app.right - app.left).abs();
    let status = if app.paused { "PAUSED" } else { "PLAYING" };
    let row = |cells: [String; 4]| Row::new(cells.map(Cell::from));
    let details = vec![
        row(["PRESET".into(), app.preset.clone(), "".into(), "".into()]),
        row([
            "LEFT".into(),
            format!("{:.1} Hz", app.left),
            "RIGHT".into(),
            format!("{:.1} Hz", app.right),
        ]),
        row([
            "BEAT".into(),
            format!("{beat:.1} Hz"),
            "PERIOD".into(),
            format!("{:.0} ms", 1_000.0 / beat),
        ]),
        row([
            "BAND".into(),
            band(beat).into(),
            "TONE".into(),
            format!("{:.2}", app.audio.volume),
        ]),
        row([
            "NOISE".into(),
            app.audio.noise.as_str().into(),
            "LEVEL".into(),
            format!("{:.2}", app.audio.noise_volume),
        ]),
    ];
    frame.render_widget(
        Table::new(
            details,
            [
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Fill(1),
            ],
        )
        .column_spacing(1)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" BINAURAL · {status} ")),
        ),
        rows[0],
    );
    let presets: Vec<_> = app
        .names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let marker = if index == app.selected { ">" } else { " " };
            let style = if index == app.selected {
                Style::default().fg(Color::Cyan).bold()
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!(" {marker} {name}"), style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(presets).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" PRESETS "),
        ),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(" ↑/↓ or j/k preset · space pause/play · q quit")
            .block(Block::bordered().border_type(BorderType::Rounded)),
        rows[2],
    );
}

fn replace(
    control: &PlaybackControl,
    config: &Config,
    name: String,
    play: bool,
) -> Result<(f64, f64, Audio)> {
    let (left, right, audio) = config::resolve(&options(name), config)?;
    control.replace(Beat::new(left, right, audio), play);
    Ok((left, right, audio))
}

fn toggle(control: &PlaybackControl, paused: &mut bool) {
    if *paused {
        control.play();
    } else {
        control.pause();
    }
    *paused = !*paused;
}

pub(super) fn run(cli_options: Options) -> Result<()> {
    let config = config::load().context("loading configuration")?;
    let (left, right, audio) = config::resolve(&cli_options, &config)?;
    let names = preset_names(&config);
    let selected = names
        .iter()
        .position(|name| cli_options.preset.as_deref() == Some(name))
        .unwrap_or_else(|| {
            names
                .iter()
                .position(|name| name == &config.default)
                .unwrap_or_default()
        });
    let mut app = App {
        preset: names[selected].clone(),
        names,
        selected,
        left,
        right,
        audio,
        paused: false,
    };
    let mut stream = DeviceSinkBuilder::open_default_sink().context("opening audio output")?;
    stream.log_on_drop(false);
    let sink = Player::connect_new(stream.mixer());
    let (control, source) = PlaybackSource::new(Beat::new(left, right, audio), false);
    sink.append(source);

    let mut terminal = ratatui::try_init().context("opening terminal UI")?;
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| render(frame, &app))?;
            if !event::poll(FRAME_TIME)? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => toggle(&control, &mut app.paused),
                KeyCode::Up | KeyCode::Char('k') if app.selected > 0 => {
                    app.selected -= 1;
                    app.preset = app.names[app.selected].clone();
                    (app.left, app.right, app.audio) =
                        replace(&control, &config, app.preset.clone(), !app.paused)?;
                }
                KeyCode::Down | KeyCode::Char('j') if app.selected + 1 < app.names.len() => {
                    app.selected += 1;
                    app.preset = app.names[app.selected].clone();
                    (app.left, app.right, app.audio) =
                        replace(&control, &config, app.preset.clone(), !app.paused)?;
                }
                _ => {}
            }
        }
        Ok(())
    })();
    ratatui::try_restore().context("restoring terminal")?;
    sink.stop();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_brainwave_bands() {
        assert_eq!(band(6.0), "theta");
        assert_eq!(band(14.0), "beta");
    }
}
