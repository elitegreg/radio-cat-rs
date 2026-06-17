use std::{env, error::Error, fmt};

use radio_cat_rs::protocol::kenwood_ascii::profile_by_id;
use radio_cat_rs::{
    capabilities::{
        Capability, RadioCapabilities, ReceiverCapabilities, ReceiverRfCapabilities,
        RitXitCapabilities, StateUpdateCapability, TransmitterCapabilities,
    },
    supported_drivers, ReceiverKind,
};

#[derive(Debug)]
struct CliError(String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for CliError {}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(driver_id) = parse_driver_id()? else {
        return Ok(());
    };

    let descriptor = supported_drivers()
        .iter()
        .find(|driver| driver.id.eq_ignore_ascii_case(&driver_id))
        .ok_or_else(|| CliError(format!("unknown radio id: {driver_id}")))?;

    let capabilities = if descriptor.id.eq_ignore_ascii_case("dummy") {
        RadioCapabilities::dummy_all()
    } else {
        profile_by_id(descriptor.id)
            .ok_or_else(|| {
                CliError(format!(
                    "no capability profile for radio id: {}",
                    descriptor.id
                ))
            })?
            .capabilities
    };

    print_radio_capabilities(
        descriptor.id,
        descriptor.display_name,
        descriptor.description,
        &capabilities,
    );
    Ok(())
}

fn parse_driver_id() -> Result<Option<String>, CliError> {
    let mut args = env::args().skip(1);

    let mut driver_id: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return Ok(None);
            }
            "--list-radios" => {
                print_supported_radios();
                return Ok(None);
            }
            value if value.starts_with('-') => {
                return Err(CliError(format!(
                    "unknown argument: {value} (use --help for usage)"
                )));
            }
            value => {
                if driver_id.is_some() {
                    return Err(CliError(format!(
                        "unexpected extra argument: {value} (use --help for usage)"
                    )));
                }
                driver_id = Some(value.to_string());
            }
        }
    }

    Ok(driver_id)
}

fn print_radio_capabilities(
    id: &str,
    display_name: &str,
    description: &str,
    caps: &RadioCapabilities,
) {
    println!("{display_name} ({id})");
    println!("{description}");
    println!();

    println!(
        "receiver kind: {}",
        describe_receiver_kind(caps.receiver_kind)
    );
    print_receiver("main rx", &caps.main_rx);
    print_optional_receiver("sub rx", caps.sub_rx.as_ref());
    print_optional_tx(caps.tx.as_ref());
    print_rit_xit(&caps.rit_xit);
    print_optional_keyer(caps.keyer);
    println!(
        "state updates: {}",
        describe_state_updates(caps.state_updates)
    );
}

fn print_receiver(title: &str, caps: &ReceiverCapabilities) {
    println!("{title}:");
    println!("  frequency: {}", describe_capability(caps.frequency));
    println!("  mode: {}", describe_capability(caps.mode));
    println!(
        "  filter bandwidth: {}",
        describe_capability(caps.filter_bandwidth)
    );
    println!("  filter shift: {}", describe_capability(caps.filter_shift));
    print_rf(&caps.rf, "  rf");
    println!();
}

fn print_optional_receiver(title: &str, caps: Option<&ReceiverCapabilities>) {
    match caps {
        Some(caps) => print_receiver(title, caps),
        None => {
            println!("{title}: not available\n");
        }
    }
}

fn print_rf(caps: &ReceiverRfCapabilities, title: &str) {
    println!("{title}:");
    println!("    preamp: {}", describe_capability(caps.preamp));
    println!("    attenuator: {}", describe_capability(caps.attenuator));
    println!(
        "    noise blanker: {}",
        describe_capability(caps.noise_blanker)
    );
    println!(
        "    noise reduction: {}",
        describe_capability(caps.noise_reduction)
    );
    println!("    auto notch: {}", describe_capability(caps.auto_notch));
}

fn print_optional_tx(caps: Option<&TransmitterCapabilities>) {
    match caps {
        Some(caps) => {
            println!("tx:");
            println!("  frequency: {}", describe_capability(caps.frequency));
            println!("  mode: {}", describe_capability(caps.mode));
            println!("  power: {}", describe_capability(caps.power));
            println!("  ptt: {}", describe_capability(caps.ptt));
            println!("  split: {}", describe_capability(caps.split));
            println!();
        }
        None => println!("tx: not available\n"),
    }
}

fn print_rit_xit(caps: &RitXitCapabilities) {
    println!("rit/xit:");
    println!(
        "  main rit enabled: {}",
        describe_capability(caps.main_rit_enabled)
    );
    println!(
        "  sub rit enabled: {}",
        describe_capability(caps.sub_rit_enabled)
    );
    println!("  xit enabled: {}", describe_capability(caps.xit_enabled));
    println!("  main offset: {}", describe_capability(caps.offset));
    println!("  sub offset: {}", describe_capability(caps.sub_offset));
    println!();
}

fn print_optional_keyer(caps: Option<radio_cat_rs::KeyerCapabilities>) {
    match caps {
        Some(caps) => {
            println!("keyer:");
            println!("  speed wpm: {}", describe_capability(caps.speed_wpm));
            println!("  sending: {}", describe_capability(caps.sending));
            println!("  send cw: {}", describe_capability(caps.send_cw));
            println!("  stop cw: {}", describe_capability(caps.stop_cw));
            println!();
        }
        None => println!("keyer: not available\n"),
    }
}

fn describe_capability(capability: Capability) -> &'static str {
    match capability {
        Capability::Unsupported => "not supported",
        Capability::ReadOnly => "readable",
        Capability::WriteOnly => "writable",
        Capability::ReadWrite => "read/write",
    }
}

fn describe_state_updates(updates: StateUpdateCapability) -> &'static str {
    match updates {
        StateUpdateCapability::Native => "native",
        StateUpdateCapability::Polling => "polling",
        StateUpdateCapability::Hybrid => "hybrid (native + polling)",
    }
}

fn describe_receiver_kind(kind: ReceiverKind) -> &'static str {
    match kind {
        ReceiverKind::SingleVfo => "single vfo",
        ReceiverKind::DualVfo => "dual vfo (second vfo)",
        ReceiverKind::DualRx => "dual rx (real sub receiver)",
    }
}

fn print_supported_radios() {
    println!("supported radio ids:");
    for driver in supported_drivers() {
        println!("  {:<16} {}", driver.id, driver.display_name);
    }
}

fn print_usage() {
    println!("radio-cat-rs capabilities example");
    println!();
    println!("Usage:");
    println!("  cargo run --example capabilities -- <radio-id>");
    println!();
    println!("Options:");
    println!("  --list-radios   Show supported radio ids and exit");
    println!("  -h, --help      Show this help and exit");
}
