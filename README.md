# radio-cat-rs

`radio-cat-rs` is an async CAT control library focused on Kenwood-style command families.

It currently implements a generalized Kenwood/Kenwood-like backend driven by protocol profiles, including Kenwood families, Elecraft K2/K3/K4 families, IC-10-derived profiles, and Kenwood-style Flex/PowerSDR emulations.

## Supported operations

The crate currently targets the `ControllableRadio` interface:

- Get/set frequency (`FA...`)
- Get/set mode (profile-specific mode maps)
- Send/stop CW (profile-specific formatting)
- Get/set CW keyer speed (`KS...`) where supported

Some profiles in the Kenwood document do not expose keyer/CW features; those methods return `RadioError::UnsupportedOperation`.

## Library example

```rust
use std::time::Duration;

use radio_cat_rs::{create_radio, ConnectionConfig, Frequency, Mode, RadioKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let radio = create_radio(
        RadioKind::KenwoodTs590,
        ConnectionConfig::serial("/dev/ttyUSB0", 38_400).with_timeout(Duration::from_secs(5)),
    )
    .await?;

    let frequency = radio.get_frequency().await?;
    println!("Current frequency: {frequency}");

    radio.set_frequency(Frequency::from_hz(14_074_000)).await?;
    radio.set_mode(Mode::Usb).await?;

    Ok(())
}
```

## Radio kinds

Call `supported_radio_kinds()` or run the CLI help to see all canonical profile names.

Aliases are also accepted for many model names from `docs/KENWOOD_PROTOCOLS.md` (for example `ts-590sg`, `k4`, `qcx`, `ts-440s`, `6xxx`, `powersdr`).

## CLI example

```bash
cargo run --example cli -- --radio kenwood-ts590 --tcp 127.0.0.1:5002
```

## Development

```bash
cargo test
```
