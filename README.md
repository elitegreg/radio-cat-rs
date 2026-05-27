# radio-cat-rs

`radio-cat-rs` is an async CAT control library for profile-driven radio control.

The crate currently includes:

- Kenwood/Kenwood-like CAT profiles (text/`;` protocol)
- Icom CI-V profiles (binary framed protocol)
- CI-V-compatible Xiegu profiles: `X108G`, `X6100`, `X6200`, `G90`, `X5105`
- Yaesu New-CAT profiles: `FT-450`, `FT-950`, `FT-2000`, `FTDX-1200`, `FTDX-3000`, `FTDX-5000`, `FTDX-9000`, `FTDX-9000 Old`, `FT-991`, `FT-891`, `FT-710`, `FTDX-10`, `FTDX-101D`, `FTDX-101MP`

## ControllableRadio scope

This crate currently focuses on the `ControllableRadio` interface:

- get/set frequency
- get/set mode
- send/stop CW text
- get/set CW keyer speed

Unsupported operations for a given model return `RadioError::UnsupportedOperation`.

## Create a radio

```rust
use std::time::Duration;

use radio_cat_rs::{create_radio, ConnectionConfig, RadioKind};

let radio = create_radio(
    RadioKind::KenwoodTs590,
    ConnectionConfig::serial("/dev/ttyUSB0", 38_400).with_timeout(Duration::from_secs(5)),
)
.await?;
```

For Icom CI-V radios, use per-model kinds:

```rust
use radio_cat_rs::{create_radio, ConnectionConfig, IcomModel, RadioKind};

let radio = create_radio(
    RadioKind::Icom(IcomModel::Ic7300),
    ConnectionConfig::serial("/dev/ttyUSB0", 115_200),
)
.await?;
```

## Generic options string

Use `create_radio_with_options(...)` for backend/runtime options:

```rust
use radio_cat_rs::{create_radio_with_options, ConnectionConfig, IcomModel, RadioKind};

let radio = create_radio_with_options(
    RadioKind::Icom(IcomModel::Ic7300),
    ConnectionConfig::serial("/dev/ttyUSB0", 115_200),
    "civ.rig_addr=0x94,civ.controller_addr=0xE0,civ.retry_max=5,civ.retry_backoff_ms=30",
)
.await?;
```

Unknown option keys are ignored.

Yaesu-specific optional keys:

- `yaesu.retry_max`
- `yaesu.retry_backoff_ms`
- `yaesu.stop_cw_cmd` (if unset, `stop_cw()` is unsupported for Yaesu New-CAT profiles)

## Radio names

- Call `supported_radio_kinds()` for canonical names.
- `FromStr` parsing also accepts many model aliases (e.g. `ic-7300`, `ic7610`, `ic-706mkiig`, `x6100`, `g90`, `ft-991`, `ftdx101mp`).

## Development

```bash
cargo test
```
