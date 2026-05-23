# radio-cat-rs

`radio-cat-rs` is an async Rust library for CAT (Computer Aided Transceiver) control of amateur radios.

The crate currently includes one backend:

- `generic-elecraft` for Elecraft-style CAT devices, including the `k4` alias accepted by the factory API and CLI example

It supports:

- Reading and setting VFO A frequency
- Reading and setting mode (`CW`, `USB`, `LSB`, `FM`)
- Sending and aborting queued CW text
- Reading and setting CW keyer speed
- Serial and TCP transports

## Status

This is an early crate. The current implementation is intentionally small and focused around the `ControllableRadio` trait and a single Elecraft-compatible backend.

## Requirements

- Rust 2021 toolchain
- A CAT-capable radio or CAT server
- Either:
  - A serial device path such as `/dev/ttyUSB0`
  - A TCP endpoint such as `127.0.0.1:5002`

## Add To Your Project

```toml
[dependencies]
radio-cat-rs = "0.1.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Library Example

```rust
use radio_cat_rs::{create_radio, ConnectionConfig, Frequency, Mode, RadioKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let radio = create_radio(
        RadioKind::GenericElecraft,
        ConnectionConfig::serial("/dev/ttyUSB0", 38_400),
    )
    .await?;

    let frequency = radio.get_frequency().await?;
    println!("Current frequency: {frequency}");

    radio.set_frequency(Frequency::from_hz(14_074_000)).await?;
    radio.set_mode(Mode::Usb).await?;
    radio.set_cw_wpm(20).await?;
    radio.send_cw("CQ TEST DE N0CALL").await?;

    Ok(())
}
```

You can also connect over TCP:

```rust
use radio_cat_rs::{create_radio, ConnectionConfig, RadioKind};

let radio = create_radio(
    RadioKind::GenericElecraft,
    ConnectionConfig::tcp("127.0.0.1", 5002),
)
.await?;
```

## Example CLI

The repository includes a small interactive CLI in [`examples/cli.rs`](/home/greg/src/radio-cat-rs/examples/cli.rs).

Run it over serial:

```bash
cargo run --example cli -- --radio generic-elecraft --serial /dev/ttyUSB0 --baud 38400
```

Run it over TCP:

```bash
cargo run --example cli -- --radio generic-elecraft --tcp 127.0.0.1:5002
```

Supported `--radio` names:

- `generic-elecraft`
- `elecraft`
- `k4`

Interactive commands:

- `get-freq`
- `set-freq <hz>`
- `get-mode`
- `set-mode <mode>`
- `send-cw <text>`
- `stop-cw`
- `get-cw-wpm`
- `set-cw-wpm <wpm>`
- `help`
- `quit`

## API Overview

The main entry points are:

- `create_radio(kind, connection)` to build a concrete radio behind a trait object
- `supported_radio_kinds()` to enumerate supported backends
- `ControllableRadio` for the common async control surface
- `ConnectionConfig::serial(...)` and `ConnectionConfig::tcp(...)` for transport selection
- `Frequency` and the `khz!` / `mhz!` macros for frequency values

## Current Behavior And Limits

For the current `GenericElecraft` backend:

- Frequency range is `100_000` to `54_000_000` Hz
- CW speed range is `8` to `100` WPM
- CW text must be ASCII, at most 60 bytes, and may not contain `;`, carriage returns, or line feeds
- Mode support is limited to `CW`, `USB`, `LSB`, and `FM`
- `stop_cw()` sends `KY @;RX;` to abort queued CW and return to receive

## Development

Build:

```bash
cargo build
```

Run tests:

```bash
cargo test
```

## License

MIT. See [`LICENSE`](/home/greg/src/radio-cat-rs/LICENSE).
