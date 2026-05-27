use std::{
    env,
    io::{self, Write},
    process,
};

use tokio::io::{AsyncBufReadExt, BufReader};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use radio_cat_rs::{
    create_radio, supported_radio_kinds, ConnectionConfig, ControllableRadio, Frequency, Mode,
    RadioKind,
};

fn usage() -> String {
    let supported = supported_radio_names();

    format!(
        "\
Usage:
  cargo run --example cli -- --radio ts-590s --serial /dev/ttyUSB0 [--baud 38400]
  cargo run --example cli -- --radio ts-590s --tcp host:port

Options:
  --radio <name>     Radio kind. Supported: {supported}
  --serial <path>    Serial device path
  --baud <rate>      Serial baud rate (default: 38400)
  --tcp <host:port>  TCP endpoint
"
    )
}

fn supported_radio_names() -> String {
    supported_radio_kinds()
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

struct CliArgs {
    radio: RadioKind,
    connection: ConnectionConfig,
}

impl CliArgs {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut radio = None;
        let mut serial = None;
        let mut baud_rate = 38_400_u32;
        let mut tcp = None;

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
                "-h" | "--help" => return Err(usage()),
                _ => {
                    return Err(format!("unknown argument `{arg}`\n\n{}", usage()));
                }
            }
        }

        let radio = radio.ok_or_else(|| format!("missing --radio\n\n{}", usage()))?;
        let connection = match (serial, tcp) {
            (Some(path), None) => ConnectionConfig::serial(path, baud_rate),
            (None, Some(endpoint)) => {
                let (host, port) = parse_tcp_endpoint(&endpoint)?;
                ConnectionConfig::tcp(host, port)
            }
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

        Ok(Self { radio, connection })
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
    println!("  ? | help            Show this help");
    println!("  get-freq            Read VFO A frequency");
    println!("  set-freq <hz>       Set VFO A frequency in Hz");
    println!("  get-mode            Read the current mode");
    println!("  set-mode <mode>     Set mode (examples: CW, USB, LSB, FM, AM, RTTY, PKTUSB)");
    println!("  send-cw <text>      Queue CW text (max 60 ASCII bytes)");
    println!("  stop-cw             Abort queued CW and return to receive");
    println!("  get-cw-wpm          Read keyer speed in WPM");
    println!("  set-cw-wpm <wpm>    Set keyer speed in WPM");
    println!("  quit | exit         Leave the prompt");
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
        "get-freq" => {
            let frequency = radio
                .get_frequency()
                .await
                .map_err(|error| error.to_string())?;
            println!("{frequency}");
            Ok(true)
        }
        "set-freq" => {
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
        "get-mode" => {
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
        "send-cw" => {
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
        "get-cw-wpm" => {
            let wpm = radio
                .get_cw_wpm()
                .await
                .map_err(|error| error.to_string())?;
            println!("{wpm}");
            Ok(true)
        }
        "set-cw-wpm" => {
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
        "quit" | "exit" => Ok(false),
        _ => Err(format!("unknown command `{command}`")),
    }
}

async fn repl(radio: &dyn ControllableRadio) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

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
    let args = CliArgs::parse(env::args().skip(1)).map_err(io::Error::other)?;
    let radio = create_radio(args.radio, args.connection).await?;

    println!("Connected to {}. Type ? for help.", args.radio.as_str());

    repl(radio.as_ref()).await
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("radio_cat_rs=debug")),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    if let Err(error) = run().await {
        eprintln!("{error}");
        process::exit(1);
    }
}
