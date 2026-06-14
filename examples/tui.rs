use std::{error::Error, time::Duration};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use radio_cat_rs::{
    Frequency, LeveledSetting, Mode, Radio, RadioConfig, RadioState, RitXitOffsetHz,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let radio = Radio::connect(RadioConfig::dummy()).await?;
    let mut updates = radio.subscribe_updates();
    let mut last_update = String::from("no updates yet");

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_ui(&mut terminal, radio, &mut updates, &mut last_update).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_ui(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    radio: Radio,
    updates: &mut tokio::sync::broadcast::Receiver<radio_cat_rs::StateUpdate>,
    last_update: &mut String,
) -> Result<(), Box<dyn Error>> {
    loop {
        while let Ok(update) = updates.try_recv() {
            *last_update = format!(
                "source={:?} flags={:?} fields={:?}",
                update.source, update.changes, update.fields
            );
        }

        terminal.draw(|frame| draw(frame, &radio.latest_state(), last_update))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('f') => cycle_main_frequency(&radio).await?,
                    KeyCode::Char('m') => cycle_main_mode(&radio).await?,
                    KeyCode::Char('p') => {
                        let next = !radio
                            .latest_state()
                            .tx
                            .as_ref()
                            .and_then(|tx| tx.transmitting)
                            .unwrap_or(false);
                        radio.set_ptt(next).await?;
                    }
                    KeyCode::Char('s') => {
                        let next = !radio
                            .latest_state()
                            .tx
                            .as_ref()
                            .and_then(|tx| tx.split)
                            .unwrap_or(false);
                        radio.set_split(next).await?;
                    }
                    KeyCode::Char('r') => {
                        let next = !radio.latest_state().rit_xit.rit_enabled.unwrap_or(false);
                        radio.set_rit_enabled(next).await?;
                    }
                    KeyCode::Char('+') => bump_rit(&radio, 100).await?,
                    KeyCode::Char('-') => bump_rit(&radio, -100).await?,
                    KeyCode::Char('k') => {
                        let speed = radio
                            .latest_state()
                            .keyer
                            .as_ref()
                            .and_then(|keyer| keyer.speed_wpm)
                            .unwrap_or(20);
                        radio.set_keyer_speed(speed.saturating_add(1)).await?;
                    }
                    KeyCode::Char('c') => radio.send_cw("CQ TEST").await?,
                    KeyCode::Char('x') => radio.stop_cw().await?,
                    KeyCode::Char('n') => {
                        let enabled = !radio
                            .latest_state()
                            .main_rx
                            .rf
                            .noise_reduction
                            .as_ref()
                            .and_then(|setting| setting.enabled)
                            .unwrap_or(false);
                        radio
                            .set_main_noise_reduction(if enabled {
                                LeveledSetting::enabled(3)
                            } else {
                                LeveledSetting::disabled()
                            })
                            .await?;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn draw(frame: &mut Frame<'_>, state: &RadioState, last_update: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled("radio-cat-rs dummy TUI", Style::default().fg(Color::Cyan)),
        Span::raw("  q quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let body = Paragraph::new(format_state(state))
        .block(Block::default().title("State").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, chunks[1]);

    let help = Paragraph::new(format!(
        "keys: f freq | m mode | p PTT | s split | r RIT | +/- RIT offset | k WPM | c send CW | x stop CW | n NR\nlast update: {last_update}"
    ))
    .block(Block::default().title("Commands").borders(Borders::ALL))
    .wrap(Wrap { trim: false });
    frame.render_widget(help, chunks[2]);
}

fn format_state(state: &RadioState) -> String {
    let tx = state.tx.as_ref();
    let keyer = state.keyer.as_ref();
    let sub = state.sub_rx.as_ref();

    format!(
        "connection: {:?}\n\
main rx: freq={} mode={} filter_bw={:?} filter_shift={:?} nr={:?}\n\
sub rx:  freq={} mode={}\n\
tx:      freq={} mode={} power_deci_mw={:?} ptt={:?} split={:?}\n\
rit/xit: rit={:?} xit={:?} offset={:?}\n\
keyer:   speed_wpm={:?} sending={:?}",
        state.connection,
        opt_freq(state.main_rx.frequency),
        opt_mode(state.main_rx.mode),
        state.main_rx.filter.bandwidth_hz,
        state.main_rx.filter.shift_hz,
        state.main_rx.rf.noise_reduction,
        opt_freq(sub.and_then(|rx| rx.frequency)),
        opt_mode(sub.and_then(|rx| rx.mode)),
        opt_freq(tx.and_then(|tx| tx.frequency)),
        opt_mode(tx.and_then(|tx| tx.mode)),
        tx.and_then(|tx| tx.power_deci_mw),
        tx.and_then(|tx| tx.transmitting),
        tx.and_then(|tx| tx.split),
        state.rit_xit.rit_enabled,
        state.rit_xit.xit_enabled,
        state.rit_xit.offset_hz.map(|offset| offset.as_hz()),
        keyer.and_then(|keyer| keyer.speed_wpm),
        keyer.and_then(|keyer| keyer.sending),
    )
}

fn opt_freq(frequency: Option<Frequency>) -> String {
    frequency
        .map(|frequency| frequency.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn opt_mode(mode: Option<Mode>) -> String {
    mode.map(|mode| mode.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn cycle_main_frequency(radio: &Radio) -> Result<(), Box<dyn Error>> {
    let current = radio.latest_state().main_rx.frequency;
    let next = match current.map(|frequency| frequency.hz()) {
        Some(14_074_000) => Frequency::from_hz(7_074_000),
        Some(7_074_000) => Frequency::from_hz(21_074_000),
        _ => Frequency::from_hz(14_074_000),
    };
    radio.set_main_frequency(next).await?;
    radio.set_tx_frequency(next).await?;
    Ok(())
}

async fn cycle_main_mode(radio: &Radio) -> Result<(), Box<dyn Error>> {
    let current = radio.latest_state().main_rx.mode;
    let next = match current {
        Some(Mode::Usb) => Mode::Cw,
        Some(Mode::Cw) => Mode::Am,
        Some(Mode::Am) => Mode::Fm,
        _ => Mode::Usb,
    };
    radio.set_main_mode(next).await?;
    radio.set_tx_mode(next).await?;
    Ok(())
}

async fn bump_rit(radio: &Radio, delta: i16) -> Result<(), Box<dyn Error>> {
    let current = radio
        .latest_state()
        .rit_xit
        .offset_hz
        .map(|offset| offset.as_hz())
        .unwrap_or(0);
    let next = (current + delta).clamp(RitXitOffsetHz::MIN, RitXitOffsetHz::MAX);
    radio.set_rit_xit_offset(RitXitOffsetHz::new(next)?).await?;
    Ok(())
}
