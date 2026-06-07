use std::{
    env,
    io::{self, Write},
    process,
    time::Duration,
};

use radio_cat_rs::{
    create_radio, supported_radio_kinds, ConnectionConfig, ControllableRadio, Frequency, Mode,
    RadioKind,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn usage() -> String {
    format!(
        "\
Usage:
  cargo run --example console -- --radio dummy
  cargo run --example console -- --radio ts-590s --serial /dev/ttyUSB0 [--baud 38400]
  cargo run --example console -- --radio ic-7300 --serial /dev/ttyUSB0 --baud 19200 --options civ.rig_addr=0x94
  cargo run --example console -- --radio ft-991 --tcp host:port

Options:
  --radio <name>        Radio kind. Use --list-radios to show supported names.
  --serial <path>       Serial device path.
  --baud <rate>         Serial baud rate (default: 38400).
  --tcp <host:port>     TCP endpoint.
  --timeout-ms <ms>     Serial/TCP operation timeout (default: library default).
  --options <k=v,...>   Radio-specific options passed to create_radio().
  --list-radios         Print supported radio names and exit.
  -h, --help            Show this help.
"
    )
}

fn list_radios() {
    for kind in supported_radio_kinds() {
        println!("{:<24} {}", kind.as_str(), kind.display_name());
    }
}

#[derive(Debug)]
struct CliArgs {
    radio: RadioKind,
    connection: ConnectionConfig,
    options: String,
}

impl CliArgs {
    fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut radio = None;
        let mut serial = None;
        let mut baud_rate = 38_400_u32;
        let mut tcp = None;
        let mut timeout_ms = None;
        let mut options = String::new();

        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--radio" => {
                    let value = args.next().ok_or("missing value for --radio")?;
                    radio = Some(
                        value
                            .parse::<RadioKind>()
                            .map_err(|error| error.to_string())?,
                    );
                }
                "--serial" => {
                    serial = Some(args.next().ok_or("missing value for --serial")?);
                }
                "--baud" => {
                    let value = args.next().ok_or("missing value for --baud")?;
                    baud_rate = value
                        .parse()
                        .map_err(|_| format!("invalid baud rate `{value}`"))?;
                }
                "--tcp" => {
                    tcp = Some(args.next().ok_or("missing value for --tcp")?);
                }
                "--timeout-ms" => {
                    let value = args.next().ok_or("missing value for --timeout-ms")?;
                    timeout_ms = Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| format!("invalid timeout `{value}`"))?,
                    );
                }
                "--options" => {
                    options = args.next().ok_or("missing value for --options")?;
                }
                "--list-radios" => {
                    list_radios();
                    return Ok(None);
                }
                "-h" | "--help" => return Err(usage()),
                _ => return Err(format!("unknown argument `{arg}`\n\n{}", usage())),
            }
        }

        let radio = radio.ok_or_else(|| format!("missing --radio\n\n{}", usage()))?;
        let mut connection = match (serial, tcp) {
            (Some(path), None) => ConnectionConfig::serial(path, baud_rate),
            (None, Some(endpoint)) => {
                let (host, port) = parse_tcp_endpoint(&endpoint)?;
                ConnectionConfig::tcp(host, port)
            }
            (None, None) if radio == RadioKind::Dummy => ConnectionConfig::tcp("127.0.0.1", 0),
            (None, None) => {
                return Err(format!(
                    "one of --serial or --tcp must be supplied\n\n{}",
                    usage()
                ))
            }
            (Some(_), Some(_)) => {
                return Err(format!(
                    "--serial and --tcp are mutually exclusive\n\n{}",
                    usage()
                ))
            }
        };

        if let Some(timeout_ms) = timeout_ms {
            connection = connection.with_timeout(Duration::from_millis(timeout_ms));
        }

        Ok(Some(Self {
            radio,
            connection,
            options,
        }))
    }
}

fn parse_tcp_endpoint(value: &str) -> Result<(String, u16), String> {
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid TCP endpoint `{value}`; expected host:port"))?;

    if host.is_empty() {
        return Err(format!("invalid TCP endpoint `{value}`; host is empty"));
    }

    let port = port
        .parse()
        .map_err(|_| format!("invalid TCP port in `{value}`"))?;

    Ok((host.to_string(), port))
}

fn split_command(line: &str) -> (&str, &str) {
    if let Some(index) = line.find(char::is_whitespace) {
        let (command, rest) = line.split_at(index);
        (command, rest.trim_start())
    } else {
        (line, "")
    }
}

fn print_help() {
    println!("Commands:");
    println!("  ? | help                 Show this help");
    println!("  status                   Read frequency, mode, CW WPM, and RIT");
    println!("  get-freq | freq          Read current frequency");
    println!("  set-freq <hz>            Set current frequency in Hz");
    println!("  get-mode | mode          Read current mode");
    println!("  set-mode <mode>          Set mode (CW, USB, LSB, FM, AM, RTTY, PKTUSB, ...)");
    println!("  send-cw <text>           Queue CW text (max 60 ASCII bytes)");
    println!("  stop-cw                  Abort queued CW and return to receive");
    println!("  get-cw-wpm | wpm         Read keyer speed in WPM");
    println!("  set-cw-wpm <wpm>         Set keyer speed in WPM");
    println!("  get-rit | rit            Read RIT offset in Hz; prints 0 when RIT is off");
    println!("  set-rit <offset-hz>      Enable RIT and set offset (-9999..9999 Hz)");
    println!("  clear-rit                Clear/zero the RIT offset");
    println!("  quit | exit              Leave the prompt");
}

async fn print_status(radio: &dyn ControllableRadio) {
    match radio.get_frequency().await {
        Ok(value) => println!("frequency: {value}"),
        Err(error) => println!("frequency: error: {error}"),
    }

    match radio.get_mode().await {
        Ok(value) => println!("mode:      {value}"),
        Err(error) => println!("mode:      error: {error}"),
    }

    match radio.get_cw_wpm().await {
        Ok(value) => println!("cw wpm:    {value}"),
        Err(error) => println!("cw wpm:    error: {error}"),
    }

    match radio.get_rit().await {
        Ok(value) => println!("rit:       {value} Hz"),
        Err(error) => println!("rit:       error: {error}"),
    }
}

async fn handle_command(line: &str, radio: &dyn ControllableRadio) -> Result<bool, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(true);
    }

    let (command, rest) = split_command(line);

    match command {
        "?" | "help" => {
            print_help();
            Ok(true)
        }
        "status" => {
            print_status(radio).await;
            Ok(true)
        }
        "get-freq" | "get-frequency" | "freq" => {
            let frequency = radio
                .get_frequency()
                .await
                .map_err(|error| error.to_string())?;
            println!("{frequency}");
            Ok(true)
        }
        "set-freq" | "set-frequency" => {
            let frequency_hz = rest
                .parse::<u64>()
                .map_err(|_| format!("invalid frequency `{rest}`"))?;
            radio
                .set_frequency(Frequency::from_hz(frequency_hz))
                .await
                .map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "get-mode" | "mode" => {
            let mode = radio.get_mode().await.map_err(|error| error.to_string())?;
            println!("{mode}");
            Ok(true)
        }
        "set-mode" => {
            let mode = rest.parse::<Mode>().map_err(|error| error.to_string())?;
            radio
                .set_mode(mode)
                .await
                .map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "send-cw" | "cw" => {
            radio
                .send_cw(rest)
                .await
                .map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "stop-cw" => {
            radio.stop_cw().await.map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "get-cw-wpm" | "get-wpm" | "wpm" => {
            let wpm = radio
                .get_cw_wpm()
                .await
                .map_err(|error| error.to_string())?;
            println!("{wpm}");
            Ok(true)
        }
        "set-cw-wpm" | "set-wpm" => {
            let wpm = rest
                .parse::<u16>()
                .map_err(|_| format!("invalid WPM `{rest}`"))?;
            radio
                .set_cw_wpm(wpm)
                .await
                .map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "get-rit" | "rit" => {
            let offset_hz = radio.get_rit().await.map_err(|error| error.to_string())?;
            println!("{offset_hz}");
            Ok(true)
        }
        "set-rit" => {
            let offset_hz = rest
                .parse::<i32>()
                .map_err(|_| format!("invalid RIT offset `{rest}`"))?;
            radio
                .set_rit(offset_hz)
                .await
                .map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "clear-rit" => {
            radio.clear_rit().await.map_err(|error| error.to_string())?;
            println!("ok");
            Ok(true)
        }
        "quit" | "exit" => Ok(false),
        _ => Err(format!("unknown command `{command}`")),
    }
}

async fn repl(radio: &dyn ControllableRadio) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    print_help();
    loop {
        print!("radio> ");
        io::stdout().flush()?;

        let Some(line) = lines.next_line().await? else {
            break;
        };

        match handle_command(&line, radio).await {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => eprintln!("error: {error}"),
        }
    }

    Ok(())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(args) = CliArgs::parse(env::args().skip(1)).map_err(io::Error::other)? else {
        return Ok(());
    };

    let radio = create_radio(args.radio, args.connection, &args.options).await?;

    println!(
        "Connected to {} ({}).",
        args.radio.as_str(),
        args.radio.display_name()
    );

    repl(radio.as_ref()).await
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    if let Err(error) = run().await {
        eprintln!("{error}");
        process::exit(1);
    }
}
