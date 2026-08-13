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
    radio.set_data_ptt(false).await?;
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

`supported_drivers()` is the only public driver-discovery API. Drivers are
built into this crate; registering a downstream driver is not currently
supported.

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
use radio_cat_rs::{Radio, RadioConfig, RadioRegion, TransportConfig};

# async fn example() -> radio_cat_rs::Result<()> {
let serial_config = RadioConfig::new("kenwood-ts590")
    .with_region(RadioRegion::IaruRegion2)
    .with_transport(TransportConfig::serial("/dev/ttyUSB0", 38_400))
    .with_options("driver.specific=true");

let tcp_config = RadioConfig::new("kenwood-ts590")
    .with_region(RadioRegion::IaruRegion2)
    .with_transport(TransportConfig::tcp("127.0.0.1:4532"))
    .with_options("driver.specific=true");

let tcp_config_with_host_port = RadioConfig::new("kenwood-ts590")
    .with_region(RadioRegion::IaruRegion2)
    .with_tcp_socket("127.0.0.1", 4532)
    .with_options("driver.specific=true");

let radio = Radio::connect(RadioConfig::dummy()).await?;
# let _ = (serial_config, tcp_config, tcp_config_with_host_port, radio);
# Ok(())
# }
```

The `options` string is driver-specific. For example, Kenwood TS-590/890/990/480/2000 profiles support `ptt_source=front|usb` for `set_ptt(...)` behavior; the default is `front`. `set_data_ptt(...)` is always data/USB PTT on those radios. Elecraft K3/K4 profiles support `rtty_data_submode=fsk|afsk` for RTTY and RTTY Reversed requests; the default `fsk` sends data submode 2, while `afsk` sends data submode 1.

Physical-radio profiles require `RadioRegion::IaruRegion1`, `IaruRegion2`, or
`IaruRegion3` so their capabilities report the appropriate documented hardware
coverage. These ranges do not grant transmit authority; callers remain
responsible for national regulations and operator-license limits.

Radio state snapshots are read-only. Use getters such as `state.main_rx()`,
`receiver.frequency()`, and `state.rit_xit().xit_offset(ReceiverPath::Main)`;
change state by submitting commands. Capability metadata exposes frequency
ranges, current supported modes, filter domains, RF indexes, power ranges,
keyer-speed ranges, and normal/data PTT behavior for building valid controls.

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

## Advanced protocol API

Protocol profile tables, frame splitters, and encoder/decoder helpers are not
part of the default API. Enable the `advanced-protocol-api` feature when
building protocol tooling or experiments:

```toml
radio-cat-rs = { version = "0.1", features = ["advanced-protocol-api"] }
```

This feature exposes built-in protocol details; it does not provide a custom
driver registration mechanism.

## Optional XML-RPC server

Enable the `xml-rpc` feature to expose a flrig-compatible XML-RPC server task.
The feature only makes the task available; it never opens a listener unless an
application explicitly binds and runs one.

```rust
use radio_cat_rs::{Radio, RadioConfig, xml_rpc::XmlRpcServerTask};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let radio = Radio::connect(RadioConfig::dummy()).await?;
let task = XmlRpcServerTask::bind(radio.clone(), "127.0.0.1:12345".parse()?).await?;
let shutdown = task.shutdown_handle();
let server = tokio::spawn(task.run());

// Use the radio and XML-RPC server until the application is ready to exit.
shutdown.shutdown();
server.await??;
# Ok(())
# }
```

The server maps flrig VFO A to the normalized Main receiver and VFO B to Sub;
the `rig.get_bw*` and `rig.set_bw*` methods read and set each receiver's filter
bandwidth in Hz.
PTT methods use data PTT. Calls for capabilities or state that the selected
radio does not provide return XML-RPC faults. The HTTP endpoint is `/RPC2`.
FSK text is not exposed because `radio-cat-rs` does not provide an FSK transmit
API.

## TUI example

Run the dummy radio TUI:

```bash
cargo run --example tui
```

Build with XML-RPC support and opt in at runtime by supplying a listen port:

```bash
cargo run --features xml-rpc --example tui -- --xml-rpc-port 12345
```

The TUI XML-RPC option binds `0.0.0.0:<port>` and is therefore reachable from
other hosts allowed by the system firewall. Omit the option to run without an
XML-RPC listener.

The TUI displays the latest state snapshot and applies live updates from the broadcast `StateUpdate` stream (`update.state`).

Interactive keys mutate state through API commands (`f` frequency, `m` mode, `p` PTT, `s` split, `r` RIT, `+/-` offset, `k` keyer speed, `n` noise reduction, `c/x` CW send/stop).

## Capabilities example

Print the normalized capability set for any supported radio id:

```bash
cargo run --example capabilities -- kenwood-ts590 --region 2
```

Use `--list-radios` to see available ids.

## Development

```bash
cargo fmt
cargo test
cargo check --examples
```
