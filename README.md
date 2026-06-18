# radio-cat-rs

`radio-cat-rs` is a stateful async CAT control library for amateur radios, transceivers, receivers, and similar devices.

The current implementation includes an in-memory `dummy` driver plus a profile-driven Kenwood-ASCII engine (Kenwood, Elecraft, and Yaesu profile IDs).

## Model

A connected radio is represented as:

```text
Radio = command sink + latest state source + update event source
```

Applications do not poll the radio directly. They subscribe to:

- `watch::Receiver<Arc<RadioState>>` for the latest state snapshot
- `broadcast::Receiver<StateUpdate>` for categorized updates

The public state model is signal-path oriented:

- main receiver
- optional sub receiver
- optional transmitter
- RIT/XIT state
- optional keyer state
- connection state

Frequencies use the existing `Frequency` type from `src/frequency.rs`.

## Dummy driver

The first driver is `dummy`. It does not open a real radio connection. It stores state in memory and supports every normalized v1 capability, including:

- main/sub frequency and mode
- filters and RF/DSP settings
- TX frequency, mode, power, PTT, split
- RIT/XIT enable and offset
- keyer speed
- CAT CW send/stop

## Basic usage

```rust
use radio_cat_rs::{Frequency, Mode, Power, Radio, RadioConfig, RitXitOffsetHz};

#[tokio::main]
async fn main() -> radio_cat_rs::Result<()> {
    let radio = Radio::connect(RadioConfig::dummy()).await?;

    let mut updates = radio.subscribe_updates();

    radio.set_main_frequency(Frequency::from_hz(14_074_000)).await?;
    radio.set_main_mode(Mode::Usb).await?;
    radio.set_tx_power(Power::from_watts(25)).await?;
    radio.set_ptt(true).await?;
    radio.send_cw("CQ TEST").await?;
    radio.stop_cw().await?;
    radio.set_rit_xit_offset(RitXitOffsetHz::new(250).unwrap()).await?;

    if let Ok(update) = updates.recv().await {
        println!("changed: {:?}", update.changes);
    }

    Ok(())
}
```

## Supported drivers

```rust
for driver in radio_cat_rs::supported_drivers() {
    println!("{} - {}", driver.id, driver.display_name);
}
```

This includes `dummy` and Kenwood-ASCII profile IDs such as:

- `kenwood-ts590`, `kenwood-ts890`, `kenwood-ts990`, `kenwood-ts2000`, `kenwood-ts480`, `kenwood-ts570`, `kenwood-ts870`, `kenwood-if232`
- `elecraft-k4`, `elecraft-k3`, `elecraft-k2`
- `yaesu-ftdx101`, `yaesu-ftdx10`, `yaesu-ft710`, `yaesu-ft891`, `yaesu-ft991`
- `icom-ic705`, `icom-ic7100`, `icom-ic7300`, `icom-ic7610`, `icom-ic7760`
- `flexradio-smartsdr`

## Serial/TCP connections and driver options

`RadioConfig` supports radios connected through either serial ports or TCP sockets:

```rust
use radio_cat_rs::{Radio, RadioConfig, TransportConfig};

# async fn example() -> radio_cat_rs::Result<()> {
let serial_config = RadioConfig::new("kenwood-ts590")
    .with_transport(TransportConfig::serial("/dev/ttyUSB0", 38_400))
    .with_options("driver.specific=true");

let tcp_config = RadioConfig::new("kenwood-ts590")
    .with_transport(TransportConfig::tcp("127.0.0.1:4532"))
    .with_options("driver.specific=true");

let tcp_config_with_host_port = RadioConfig::new("kenwood-ts590")
    .with_tcp_socket("127.0.0.1", 4532)
    .with_options("driver.specific=true");

let radio = Radio::connect(RadioConfig::dummy()).await?;
# let _ = (serial_config, tcp_config, tcp_config_with_host_port, radio);
# Ok(())
# }
```

The `options` string is passed through to the selected driver unchanged. The core API does not parse it, so future drivers can use driver-specific formats while keeping one common construction path.

`flexradio-smartsdr` is TCP-only. Use `with_tcp_transport(...)` or `with_tcp_socket(...)` when calling `Radio::connect`, for example:

```rust
use radio_cat_rs::RadioConfig;

# fn example() -> RadioConfig {
RadioConfig::new("flexradio-smartsdr").with_tcp_socket("127.0.0.1", 4992)
# }
```

## Provided transports

The library exposes a `CatTransport` trait and `Radio::connect_with_transport` / `Radio::build_with_transport` APIs so callers can provide an already-open bidirectional data channel. This is intended for shared serial-port setups where another library owns modem/control lines, such as CW keying, while CAT data passes through a separate async channel.

```rust
use radio_cat_rs::{AsyncIoTransport, Radio, RadioConfig};

# async fn example<T>(io: T) -> radio_cat_rs::Result<()>
# where
#     T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
# {
let transport = AsyncIoTransport::new(io);
let radio = Radio::connect_with_transport(RadioConfig::dummy(), transport).await?;
# Ok(())
# }
```

The dummy driver ignores the transport, but the API shape is in place for real drivers.

## TUI example

Run the dummy radio TUI:

```bash
cargo run --example tui
```

The TUI displays the latest state snapshot and applies live updates from the broadcast `StateUpdate` stream (`update.state`).

Interactive keys mutate state through API commands (`f` frequency, `m` mode, `p` PTT, `s` split, `r` RIT, `+/-` offset, `k` keyer speed, `n` noise reduction, `c/x` CW send/stop).

## Capabilities example

Print the normalized capability set for any supported radio id:

```bash
cargo run --example capabilities -- kenwood-ts590
```

Use `--list-radios` to see available ids.

## Development

```bash
cargo fmt
cargo test
cargo check --examples
```
