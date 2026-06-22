use std::{env, error::Error, fmt, fs::OpenOptions, str::FromStr, time::Duration};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use radio_cat_rs::{
    supported_drivers, Frequency, LeveledSetting, Mode, Power, Radio, RadioConfig, RadioState,
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
    let mut command_input = String::new();
    let mut status = String::from("type 'help' for commands");

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

        terminal.draw(|frame| {
            draw(
                frame,
                state.as_ref(),
                last_update,
                session_summary,
                &status,
                &command_input,
            )
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                tracing::debug!(key = ?key.code, "key pressed");
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Esc => command_input.clear(),
                    KeyCode::Backspace => {
                        command_input.pop();
                    }
                    KeyCode::Enter => {
                        let line = command_input.trim().to_string();
                        if !line.is_empty() {
                            if line.eq_ignore_ascii_case("q") || line.eq_ignore_ascii_case("quit") {
                                break;
                            }
                            match execute_command(&radio, state.as_ref(), &line).await {
                                Ok(message) => status = message,
                                Err(error) => status = format!("error: {error}"),
                            }
                        }
                        command_input.clear();
                    }
                    KeyCode::Char(ch)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        command_input.push(ch);
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn draw(
    frame: &mut Frame<'_>,
    state: &RadioState,
    last_update: &str,
    session_summary: &str,
    status: &str,
    command_input: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled("radio-cat-rs TUI", Style::default().fg(Color::Cyan)),
        Span::raw(format!("  {session_summary}  ctrl-c/quit exit")),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let body = Paragraph::new(format_state(state))
        .block(Block::default().title("State").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, chunks[1]);

    let help = Paragraph::new(format!(
        "status: {status}\nlast update: {last_update}\nexamples: set-freq-main 14074000 | set-mode-main usb | set-power 50 | set-rit-main on | set-offset 250 | set-offset-xit 250 | send-cw CQ TEST\ncommands: help, refresh, set-*-main/sub/tx, set-power, set-ptt, set-data-ptt, set-split, set-xit, set-keyer-speed, stop-cw"
    ))
    .block(Block::default().title("Command Help").borders(Borders::ALL))
    .wrap(Wrap { trim: false });
    frame.render_widget(help, chunks[2]);

    let command = Paragraph::new(format!("> {command_input}"))
        .block(Block::default().title("Command Line").borders(Borders::ALL));
    frame.render_widget(command, chunks[3]);
}

fn format_state(state: &RadioState) -> String {
    let tx = state.tx.as_ref();
    let keyer = state.keyer.as_ref();
    let sub = state.sub_rx.as_ref();

    format!(
        "connection: {:?}\n\
main rx: freq={} mode={} filter_bw={:?} filter_shift={:?} preamp={} attn={} nb={} nr={} autonotch={:?}\n\
sub rx:  freq={} mode={} filter_bw={:?} filter_shift={:?} preamp={} attn={} nb={} nr={} autonotch={:?}\n\
tx:      freq={} mode={} power={:?} ptt={:?} split={:?}\n\
rit/xit: main_rit={:?} sub_rit={:?} xit={:?} main_offset={:?} xit_offset={:?} sub_offset={:?}\n\
keyer:   speed_wpm={:?} sending={:?}",
        state.connection,
        opt_freq(state.main_rx.frequency),
        opt_mode(state.main_rx.mode),
        state.main_rx.filter.bandwidth_hz,
        state.main_rx.filter.shift_hz,
        opt_setting(state.main_rx.rf.preamp),
        opt_setting(state.main_rx.rf.attenuator),
        opt_setting(state.main_rx.rf.noise_blanker),
        opt_setting(state.main_rx.rf.noise_reduction),
        state.main_rx.rf.auto_notch,
        opt_freq(sub.and_then(|rx| rx.frequency)),
        opt_mode(sub.and_then(|rx| rx.mode)),
        sub.and_then(|rx| rx.filter.bandwidth_hz),
        sub.and_then(|rx| rx.filter.shift_hz),
        opt_setting(sub.and_then(|rx| rx.rf.preamp)),
        opt_setting(sub.and_then(|rx| rx.rf.attenuator)),
        opt_setting(sub.and_then(|rx| rx.rf.noise_blanker)),
        opt_setting(sub.and_then(|rx| rx.rf.noise_reduction)),
        sub.and_then(|rx| rx.rf.auto_notch),
        opt_freq(tx.and_then(|tx| tx.frequency)),
        opt_mode(tx.and_then(|tx| tx.mode)),
        tx.and_then(|tx| tx.power),
        tx.and_then(|tx| tx.transmitting),
        tx.and_then(|tx| tx.split),
        state.rit_xit.main_rit_enabled,
        state.rit_xit.sub_rit_enabled,
        state.rit_xit.xit_enabled,
        state.rit_xit.offset_hz.map(|offset| offset.as_hz()),
        state.rit_xit.xit_offset_hz.map(|offset| offset.as_hz()),
        state.rit_xit.sub_offset_hz.map(|offset| offset.as_hz()),
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

fn opt_setting(setting: Option<LeveledSetting>) -> String {
    match setting {
        Some(LeveledSetting {
            enabled: Some(false),
            ..
        }) => "off".to_string(),
        Some(LeveledSetting {
            enabled: Some(true),
            level: Some(level),
        }) => format!("on@{level}"),
        Some(LeveledSetting {
            enabled: Some(true),
            level: None,
        }) => "on".to_string(),
        Some(LeveledSetting {
            enabled: None,
            level: Some(level),
        }) => format!("level={level}"),
        Some(LeveledSetting {
            enabled: None,
            level: None,
        }) => "unknown".to_string(),
        None => "n/a".to_string(),
    }
}

async fn execute_command(
    radio: &Radio,
    state: &RadioState,
    line: &str,
) -> Result<String, Box<dyn Error>> {
    let mut parts = line.split_whitespace();
    let command = parts
        .next()
        .ok_or_else(|| CliError("empty command".to_string()))?;

    match command {
        "help" => Ok(help_text()),
        "refresh" => {
            radio.refresh().await?;
            Ok("refresh requested".to_string())
        }
        "set-freq-main" => {
            let value = parse_frequency_arg(parts.next())?;
            radio.set_main_frequency(value).await?;
            Ok(format!("main frequency -> {}", value.hz()))
        }
        "set-freq-sub" => {
            let value = parse_frequency_arg(parts.next())?;
            radio.set_sub_frequency(value).await?;
            Ok(format!("sub frequency -> {}", value.hz()))
        }
        "set-freq-tx" => {
            let value = parse_frequency_arg(parts.next())?;
            radio.set_tx_frequency(value).await?;
            Ok(format!("tx frequency -> {}", value.hz()))
        }
        "set-mode-main" => {
            let value = parse_mode_arg(parts.next())?;
            radio.set_main_mode(value).await?;
            Ok(format!("main mode -> {value}"))
        }
        "set-mode-sub" => {
            let value = parse_mode_arg(parts.next())?;
            radio.set_sub_mode(value).await?;
            Ok(format!("sub mode -> {value}"))
        }
        "set-mode-tx" => {
            let value = parse_mode_arg(parts.next())?;
            radio.set_tx_mode(value).await?;
            Ok(format!("tx mode -> {value}"))
        }
        "set-filter-bw-main" => {
            let value = parse_u16_arg(parts.next(), "bandwidth_hz")?;
            radio.set_main_filter_bandwidth(value).await?;
            Ok(format!("main bandwidth -> {value}"))
        }
        "set-filter-bw-sub" => {
            let value = parse_u16_arg(parts.next(), "bandwidth_hz")?;
            radio.set_sub_filter_bandwidth(value).await?;
            Ok(format!("sub bandwidth -> {value}"))
        }
        "set-filter-shift-main" => {
            let value = parse_i16_arg(parts.next(), "shift_hz")?;
            radio.set_main_filter_shift(value).await?;
            Ok(format!("main filter shift -> {value}"))
        }
        "set-filter-shift-sub" => {
            let value = parse_i16_arg(parts.next(), "shift_hz")?;
            radio.set_sub_filter_shift(value).await?;
            Ok(format!("sub filter shift -> {value}"))
        }
        "set-preamp-main" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_main_preamp(value).await?;
            Ok("main preamp updated".to_string())
        }
        "set-preamp-sub" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_sub_preamp(value).await?;
            Ok("sub preamp updated".to_string())
        }
        "set-attenuator-main" | "set-attn-main" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_main_attenuator(value).await?;
            Ok("main attenuator updated".to_string())
        }
        "set-attenuator-sub" | "set-attn-sub" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_sub_attenuator(value).await?;
            Ok("sub attenuator updated".to_string())
        }
        "set-nb-main" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_main_noise_blanker(value).await?;
            Ok("main noise blanker updated".to_string())
        }
        "set-nb-sub" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_sub_noise_blanker(value).await?;
            Ok("sub noise blanker updated".to_string())
        }
        "set-nr-main" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_main_noise_reduction(value).await?;
            Ok("main noise reduction updated".to_string())
        }
        "set-nr-sub" => {
            let value = parse_leveled_setting_arg(parts.next())?;
            radio.set_sub_noise_reduction(value).await?;
            Ok("sub noise reduction updated".to_string())
        }
        "set-an-main" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_main_auto_notch(value).await?;
            Ok(format!("main auto notch -> {value}"))
        }
        "set-an-sub" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_sub_auto_notch(value).await?;
            Ok(format!("sub auto notch -> {value}"))
        }
        "set-power" | "set-tx-power" => {
            let value = parse_u16_arg(parts.next(), "watts")?;
            radio.set_tx_power(Power::from_watts(value)).await?;
            Ok(format!("tx power -> {value} W"))
        }
        "set-ptt" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_ptt(value).await?;
            Ok(format!("ptt -> {value}"))
        }
        "set-data-ptt" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_data_ptt(value).await?;
            Ok(format!("data ptt -> {value}"))
        }
        "set-split" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_split(value).await?;
            Ok(format!("split -> {value}"))
        }
        "set-rit-main" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_main_rit_enabled(value).await?;
            Ok(format!("main rit -> {value}"))
        }
        "set-rit-sub" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_sub_rit_enabled(value).await?;
            Ok(format!("sub rit -> {value}"))
        }
        "set-xit" => {
            let value = parse_bool_arg(parts.next(), "enabled")?;
            radio.set_xit_enabled(value).await?;
            Ok(format!("xit -> {value}"))
        }
        "set-offset" | "set-offset-main" => {
            let value = parse_i16_arg(parts.next(), "offset_hz")?;
            let value = value.clamp(RitXitOffsetHz::MIN, RitXitOffsetHz::MAX);
            radio
                .set_main_rit_offset(RitXitOffsetHz::new(value)?)
                .await?;
            Ok(format!("main rit offset -> {value}"))
        }
        "set-offset-xit" => {
            let value = parse_i16_arg(parts.next(), "offset_hz")?;
            let value = value.clamp(RitXitOffsetHz::MIN, RitXitOffsetHz::MAX);
            radio
                .set_main_xit_offset(RitXitOffsetHz::new(value)?)
                .await?;
            Ok(format!("main xit offset -> {value}"))
        }
        "set-offset-sub" => {
            let value = parse_i16_arg(parts.next(), "offset_hz")?;
            let value = value.clamp(RitXitOffsetHz::MIN, RitXitOffsetHz::MAX);
            radio
                .set_sub_rit_offset(RitXitOffsetHz::new(value)?)
                .await?;
            Ok(format!("sub rit offset -> {value}"))
        }
        "set-keyer-speed" => {
            let value = parse_u8_arg(parts.next(), "wpm")?;
            radio.set_keyer_speed(value).await?;
            Ok(format!("keyer speed -> {value}"))
        }
        "send-cw" => {
            let text = line
                .strip_prefix("send-cw")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError("send-cw requires text".to_string()))?;
            radio.send_cw(text).await?;
            Ok(format!("sending cw: {text}"))
        }
        "stop-cw" => {
            radio.stop_cw().await?;
            Ok("stop cw requested".to_string())
        }
        "status" => Ok(format_state(state)),
        other => Err(Box::new(CliError(format!("unknown command: {other}")))),
    }
}

fn parse_frequency_arg(value: Option<&str>) -> Result<Frequency, Box<dyn Error>> {
    let value = parse_u64_arg(value, "frequency_hz")?;
    Ok(Frequency::from_hz(value))
}

fn parse_mode_arg(value: Option<&str>) -> Result<Mode, Box<dyn Error>> {
    let value = value.ok_or_else(|| CliError("missing mode".to_string()))?;
    Ok(Mode::from_str(value)?)
}

fn parse_bool_arg(value: Option<&str>, field: &str) -> Result<bool, Box<dyn Error>> {
    let value = value.ok_or_else(|| CliError(format!("missing {field}")))?;
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" | "enable" | "enabled" => Ok(true),
        "0" | "false" | "off" | "no" | "disable" | "disabled" => Ok(false),
        _ => Err(Box::new(CliError(format!("invalid {field}: {value}")))),
    }
}

fn parse_leveled_setting_arg(value: Option<&str>) -> Result<LeveledSetting, Box<dyn Error>> {
    let value = value.ok_or_else(|| CliError("missing setting".to_string()))?;
    match value.to_ascii_lowercase().as_str() {
        "off" | "false" | "0" => Ok(LeveledSetting::disabled()),
        "on" | "true" => Ok(LeveledSetting::enabled(1)),
        _ => {
            let level = value
                .parse::<u8>()
                .map_err(|_| CliError(format!("invalid leveled setting: {value}")))?;
            Ok(LeveledSetting::enabled(level))
        }
    }
}

fn parse_u8_arg(value: Option<&str>, field: &str) -> Result<u8, Box<dyn Error>> {
    value
        .ok_or_else(|| CliError(format!("missing {field}")))?
        .parse::<u8>()
        .map_err(|_| Box::new(CliError(format!("invalid {field}"))) as Box<dyn Error>)
}

fn parse_u16_arg(value: Option<&str>, field: &str) -> Result<u16, Box<dyn Error>> {
    value
        .ok_or_else(|| CliError(format!("missing {field}")))?
        .parse::<u16>()
        .map_err(|_| Box::new(CliError(format!("invalid {field}"))) as Box<dyn Error>)
}

fn parse_u64_arg(value: Option<&str>, field: &str) -> Result<u64, Box<dyn Error>> {
    value
        .ok_or_else(|| CliError(format!("missing {field}")))?
        .parse::<u64>()
        .map_err(|_| Box::new(CliError(format!("invalid {field}"))) as Box<dyn Error>)
}

fn parse_i16_arg(value: Option<&str>, field: &str) -> Result<i16, Box<dyn Error>> {
    value
        .ok_or_else(|| CliError(format!("missing {field}")))?
        .parse::<i16>()
        .map_err(|_| Box::new(CliError(format!("invalid {field}"))) as Box<dyn Error>)
}

fn help_text() -> String {
    [
        "help",
        "refresh",
        "status",
        "set-freq-main <hz>",
        "set-freq-sub <hz>",
        "set-freq-tx <hz>",
        "set-mode-main <mode>",
        "set-mode-sub <mode>",
        "set-mode-tx <mode>",
        "set-filter-bw-main <hz>",
        "set-filter-bw-sub <hz>",
        "set-filter-shift-main <hz>",
        "set-filter-shift-sub <hz>",
        "set-preamp-main <off|on|level>",
        "set-preamp-sub <off|on|level>",
        "set-attenuator-main <off|on|level>",
        "set-attenuator-sub <off|on|level>",
        "set-attn-main <off|on|level>",
        "set-attn-sub <off|on|level>",
        "set-nb-main <off|on|level>",
        "set-nb-sub <off|on|level>",
        "set-nr-main <off|on|level>",
        "set-nr-sub <off|on|level>",
        "set-an-main <on|off>",
        "set-an-sub <on|off>",
        "set-power <watts>",
        "set-tx-power <watts>",
        "set-ptt <on|off>",
        "set-data-ptt <on|off>",
        "set-split <on|off>",
        "set-rit-main <on|off>",
        "set-rit-sub <on|off>",
        "set-xit <on|off>",
        "set-offset <hz>",
        "set-offset-main <hz>",
        "set-offset-xit <hz>",
        "set-offset-sub <hz>",
        "set-keyer-speed <wpm>",
        "send-cw <text>",
        "stop-cw",
        "quit",
    ]
    .join(" | ")
}
