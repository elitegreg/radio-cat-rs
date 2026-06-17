use std::{env, error::Error, fmt, fs::OpenOptions, time::Duration};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use radio_cat_rs::{
    supported_drivers, Frequency, LeveledSetting, Mode, Radio, RadioConfig, RadioState,
    RitXitOffsetHz, StateUpdate, TransportConfig,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

const DEFAULT_BAUD_RATE: u32 = 38_400;
const DEFAULT_LOG_LEVEL: &str = "info";

#[derive(Debug)]
struct CliError(String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for CliError {}

struct LaunchConfig {
    radio_config: RadioConfig,
    radio_label: String,
    transport_label: String,
    log_level: tracing::Level,
    log_level_label: String,
    log_file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let Some(launch) = parse_launch_config()? else {
        return Ok(());
    };

    let LaunchConfig {
        radio_config,
        radio_label,
        transport_label,
        log_level,
        log_level_label,
        log_file,
    } = launch;

    init_tracing(log_level, log_file.as_deref())?;

    let log_target = log_file.as_deref().unwrap_or("stderr");
    let session_summary = format!(
        "radio={radio_label} transport={transport_label} log={log_level_label}->{log_target}"
    );

    tracing::info!(
        radio = %radio_label,
        transport = %transport_label,
        log_level = %log_level_label,
        log_target = %log_target,
        "starting TUI"
    );

    let radio = Radio::connect(radio_config).await?;
    let mut updates = radio.subscribe_updates();
    let mut state = radio.latest_state();
    let mut last_update = String::from("no updates yet");

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_ui(
        &mut terminal,
        radio,
        &mut state,
        &mut updates,
        &mut last_update,
        &session_summary,
    )
    .await;

    tracing::info!("tui loop exited");

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn parse_launch_config() -> Result<Option<LaunchConfig>, CliError> {
    let mut driver = String::from("dummy");
    let mut options = String::new();
    let mut serial_path: Option<String> = None;
    let mut baud_rate = DEFAULT_BAUD_RATE;
    let mut tcp_address: Option<String> = None;
    let mut tcp_host: Option<String> = None;
    let mut tcp_port: Option<u16> = None;
    let mut log_level = parse_log_level(DEFAULT_LOG_LEVEL)?;
    let mut log_level_label = DEFAULT_LOG_LEVEL.to_string();
    let mut log_file: Option<String> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return Ok(None);
            }
            "--list-radios" | "--list-drivers" => {
                print_supported_radios();
                return Ok(None);
            }
            "-d" | "--driver" => driver = next_arg_value(&mut args, &arg)?,
            "--serial" => serial_path = Some(next_arg_value(&mut args, "--serial")?),
            "--baud" => {
                let value = next_arg_value(&mut args, "--baud")?;
                baud_rate = value
                    .parse::<u32>()
                    .map_err(|_| CliError(format!("invalid baud rate: {value}")))?;
            }
            "--tcp" => tcp_address = Some(next_arg_value(&mut args, "--tcp")?),
            "--host" => tcp_host = Some(next_arg_value(&mut args, "--host")?),
            "--port" => {
                let value = next_arg_value(&mut args, "--port")?;
                tcp_port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|_| CliError(format!("invalid TCP port: {value}")))?,
                );
            }
            "--options" => options = next_arg_value(&mut args, "--options")?,
            "--log-level" => {
                let value = next_arg_value(&mut args, "--log-level")?;
                log_level = parse_log_level(&value)?;
                log_level_label = value.to_ascii_lowercase();
            }
            "--log-file" => log_file = Some(next_arg_value(&mut args, "--log-file")?),
            other => {
                return Err(CliError(format!(
                    "unknown argument: {other} (use --help for usage)"
                )))
            }
        }
    }

    let transport = resolve_transport(serial_path, baud_rate, tcp_address, tcp_host, tcp_port)?;

    let mut radio_config = RadioConfig::new(driver.clone()).with_transport(transport.clone());
    if !options.is_empty() {
        radio_config = radio_config.with_options(options);
    }

    Ok(Some(LaunchConfig {
        radio_config,
        radio_label: driver,
        transport_label: describe_transport(&transport),
        log_level,
        log_level_label,
        log_file,
    }))
}

fn parse_log_level(value: &str) -> Result<tracing::Level, CliError> {
    match value.to_ascii_lowercase().as_str() {
        "trace" => Ok(tracing::Level::TRACE),
        "debug" => Ok(tracing::Level::DEBUG),
        "info" => Ok(tracing::Level::INFO),
        "warn" => Ok(tracing::Level::WARN),
        "error" => Ok(tracing::Level::ERROR),
        _ => Err(CliError(format!(
            "invalid log level: {value} (expected trace|debug|info|warn|error)"
        ))),
    }
}

fn init_tracing(log_level: tracing::Level, log_file: Option<&str>) -> Result<(), CliError> {
    if let Some(path) = log_file {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| CliError(format!("failed to open log file {path}: {error}")))?;

        tracing_subscriber::fmt()
            .with_max_level(log_level)
            .with_ansi(false)
            .with_writer(move || {
                file.try_clone()
                    .expect("log file handle should be clonable")
            })
            .try_init()
            .map_err(|error| CliError(format!("failed to initialize logging: {error}")))?;
    } else {
        tracing_subscriber::fmt()
            .with_max_level(log_level)
            .try_init()
            .map_err(|error| CliError(format!("failed to initialize logging: {error}")))?;
    }

    Ok(())
}

fn next_arg_value<I>(args: &mut I, flag: &str) -> Result<String, CliError>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| CliError(format!("missing value for {flag}")))
}

fn resolve_transport(
    serial_path: Option<String>,
    baud_rate: u32,
    tcp_address: Option<String>,
    tcp_host: Option<String>,
    tcp_port: Option<u16>,
) -> Result<TransportConfig, CliError> {
    let has_tcp_parts = tcp_address.is_some() || tcp_host.is_some() || tcp_port.is_some();

    if serial_path.is_some() && has_tcp_parts {
        return Err(CliError(
            "choose either serial (--serial) or TCP (--tcp / --host+--port), not both".to_string(),
        ));
    }

    if tcp_address.is_some() && (tcp_host.is_some() || tcp_port.is_some()) {
        return Err(CliError(
            "use either --tcp <host:port> or --host <host> --port <port>, not both".to_string(),
        ));
    }

    if let Some(path) = serial_path {
        return Ok(TransportConfig::serial(path, baud_rate));
    }

    if let Some(address) = tcp_address {
        return Ok(TransportConfig::tcp(address));
    }

    if tcp_host.is_some() || tcp_port.is_some() {
        let host = tcp_host
            .ok_or_else(|| CliError("missing --host (required when using --port)".to_string()))?;
        let port = tcp_port
            .ok_or_else(|| CliError("missing --port (required when using --host)".to_string()))?;
        return Ok(TransportConfig::tcp_socket(host, port));
    }

    Ok(TransportConfig::None)
}

fn describe_transport(transport: &TransportConfig) -> String {
    match transport {
        TransportConfig::None => "none".to_string(),
        TransportConfig::Serial { path, baud_rate } => format!("serial:{path}@{baud_rate}"),
        TransportConfig::Tcp { address } => format!("tcp:{address}"),
    }
}

fn print_supported_radios() {
    println!("Supported radios:");
    for driver in supported_drivers() {
        println!("  {:<16} {}", driver.id, driver.display_name);
    }
}

fn print_usage() {
    println!("radio-cat-rs TUI example");
    println!();
    println!("Usage:");
    println!("  cargo run --example tui -- [options]");
    println!();
    println!("Options:");
    println!("  -d, --driver <id>       Radio driver/profile id (default: dummy)");
    println!("      --serial <path>     Use serial CAT transport (e.g. /dev/ttyUSB0)");
    println!(
        "      --baud <rate>       Serial baud rate (default: {DEFAULT_BAUD_RATE}, with --serial)"
    );
    println!("      --tcp <host:port>   Use TCP CAT transport");
    println!("      --host <host>       TCP host (use with --port)");
    println!("      --port <port>       TCP port (use with --host)");
    println!("      --options <text>    Driver options string");
    println!("      --log-level <lvl>   Log level: trace|debug|info|warn|error (default: {DEFAULT_LOG_LEVEL})");
    println!("      --log-file <path>   Append logs to file instead of stderr");
    println!("      --list-radios       Show supported radio ids and exit");
    println!("  -h, --help              Show this help and exit");
}

async fn run_ui(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    radio: Radio,
    state: &mut radio_cat_rs::SharedRadioState,
    updates: &mut tokio::sync::broadcast::Receiver<StateUpdate>,
    last_update: &mut String,
    session_summary: &str,
) -> Result<(), Box<dyn Error>> {
    loop {
        loop {
            match updates.try_recv() {
                Ok(update) => {
                    tracing::debug!(
                        source = ?update.source,
                        changes = ?update.changes,
                        fields = ?update.fields,
                        "received state update"
                    );
                    *state = update.state.clone();
                    *last_update = format!(
                        "source={:?} flags={:?} fields={:?}",
                        update.source, update.changes, update.fields
                    );
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                    *last_update = format!("lagged {skipped} updates");
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => return Ok(()),
            }
        }

        terminal.draw(|frame| draw(frame, state.as_ref(), last_update, session_summary))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                tracing::debug!(key = ?key.code, "key pressed");
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('f') => cycle_main_frequency(&radio, state.as_ref()).await?,
                    KeyCode::Char('m') => cycle_main_mode(&radio, state.as_ref()).await?,
                    KeyCode::Char('p') => {
                        let next = !state
                            .tx
                            .as_ref()
                            .and_then(|tx| tx.transmitting)
                            .unwrap_or(false);
                        radio.set_ptt(next).await?;
                    }
                    KeyCode::Char('s') => {
                        let next = !state.tx.as_ref().and_then(|tx| tx.split).unwrap_or(false);
                        radio.set_split(next).await?;
                    }
                    KeyCode::Char('r') => {
                        let next = !state.rit_xit.main_rit_enabled.unwrap_or(false);
                        radio.set_main_rit_enabled(next).await?;
                    }
                    KeyCode::Char('+') => bump_rit(&radio, state.as_ref(), 100).await?,
                    KeyCode::Char('-') => bump_rit(&radio, state.as_ref(), -100).await?,
                    KeyCode::Char('k') => {
                        let speed = state
                            .keyer
                            .as_ref()
                            .and_then(|keyer| keyer.speed_wpm)
                            .unwrap_or(20);
                        radio.set_keyer_speed(speed.saturating_add(1)).await?;
                    }
                    KeyCode::Char('c') => radio.send_cw("CQ TEST").await?,
                    KeyCode::Char('x') => radio.stop_cw().await?,
                    KeyCode::Char('n') => {
                        let enabled = !state
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

fn draw(frame: &mut Frame<'_>, state: &RadioState, last_update: &str, session_summary: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled("radio-cat-rs TUI", Style::default().fg(Color::Cyan)),
        Span::raw(format!("  {session_summary}  q quit")),
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
tx:      freq={} mode={} power={:?} ptt={:?} split={:?}\n\
rit/xit: main_rit={:?} sub_rit={:?} xit={:?} offset={:?}\n\
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
        tx.and_then(|tx| tx.power),
        tx.and_then(|tx| tx.transmitting),
        tx.and_then(|tx| tx.split),
        state.rit_xit.main_rit_enabled,
        state.rit_xit.sub_rit_enabled,
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

async fn cycle_main_frequency(radio: &Radio, state: &RadioState) -> Result<(), Box<dyn Error>> {
    let current = state.main_rx.frequency;
    let next = match current.map(|frequency| frequency.hz()) {
        Some(14_074_000) => Frequency::from_hz(7_074_000),
        Some(7_074_000) => Frequency::from_hz(21_074_000),
        _ => Frequency::from_hz(14_074_000),
    };
    radio.set_main_frequency(next).await?;
    radio.set_tx_frequency(next).await?;
    Ok(())
}

async fn cycle_main_mode(radio: &Radio, state: &RadioState) -> Result<(), Box<dyn Error>> {
    let current = state.main_rx.mode;
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

async fn bump_rit(radio: &Radio, state: &RadioState, delta: i16) -> Result<(), Box<dyn Error>> {
    let current = state
        .rit_xit
        .offset_hz
        .map(|offset| offset.as_hz())
        .unwrap_or(0);
    let next = (current + delta).clamp(RitXitOffsetHz::MIN, RitXitOffsetHz::MAX);
    radio.set_rit_xit_offset(RitXitOffsetHz::new(next)?).await?;
    Ok(())
}
