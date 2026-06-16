# Kenwood Protocol Radio Implementation Plan

This document outlines how `radio-cat-rs` should implement the Kenwood
protocol family described in `/home/greg/cat.csv`.

In this document, "Kenwood protocol" means the shared ASCII,
semicolon-delimited CAT protocol style used by the listed Kenwood, Elecraft,
and Yaesu radios. It is not limited to Kenwood-branded radios.

The current frontend API in `src/api.rs`, `src/command.rs`, `src/state.rs`,
and `src/capabilities.rs` is a good fit for most of this protocol family:
applications operate on normalized main/sub receiver paths, transmitter state,
RIT/XIT state, keyer state, capabilities, and async state updates. The driver
should keep that semantic API and hide model-specific VFO, mode, and CAT syntax
behind profile-specific codecs.

## Source Inputs

- `/home/greg/cat.csv`: authoritative model/command matrix for this plan.
- `docs/radio-cat-rs-design.md`: current library architecture and frontend API
  intent.
- `docs/radio_cat_rs_kenwood_ascii_driver_design.md`: prior shared ASCII
  protocol driver design notes.
- `docs/KENWOOD_PROTOCOLS.md`: broader source-derived Kenwood-style protocol
  notes.

The CSV references external filter tables that were not present under
`/home/greg` when this document was drafted:

- `Kenwood-HiLoFiltering-ts590.txt`
- `Kenwood-HiLoFiltering-ts890.txt`
- `Kenwood-HiLoFiltering-ts990.txt`
- `Kenwood-HiLoFiltering-ts480.txt`
- `ftdx101-bandwidth.csv`
- `ftdx891-bandwidth.csv`
- `ft991-bandwidth.csv`

Those files are required before the filter conversion tables can be implemented
precisely.

## Covered Radios

The first implementation pass should cover every model column in the CSV:

| Profile | Brand | Notes |
| --- | --- | --- |
| `kenwood-ts590` | Kenwood | 11-digit VFO, `MD` plus `DA`, `IF` status, hi/lo cut conversion. |
| `kenwood-ts890` | Kenwood | 11-digit VFO, `SF` mode/frequency snapshots, hi/lo cut conversion, `NB1`/`NB2`. |
| `kenwood-ts990` | Kenwood | Main/sub receiver, `OM` mode, `SP` split, hi/lo cut conversion. |
| `kenwood-ts2000` | Kenwood | 11-digit VFO, `MD`, `IF` status, `FT` split. |
| `kenwood-ts480` | Kenwood | 11-digit VFO, `MD`, `IF` status, hi/lo cut conversion. |
| `kenwood-ts570` | Kenwood | 11-digit VFO, `MD`, limited filter support. |
| `kenwood-ts870` | Kenwood | 11-digit VFO, `MD`, limited filter support. |
| `kenwood-if232` | Kenwood | Limited async: `IF;` only, no filters/RF extras in this matrix. |
| `elecraft-k4` | Elecraft | Main/sub style commands using optional `$`; `AI2;AID250;`. |
| `elecraft-k3-family` | Elecraft | K3S/K3/KX3/KX2; optional `$` means sub VFO; `DT` data-mode composition. |
| `elecraft-k2` | Elecraft | More limited command surface; `FR` can cancel split. |
| `yaesu-ftdx101` | Yaesu | Main/sub style, 9-digit VFO, `MDX`, signed `IS`, bandwidth table. |
| `yaesu-ftdx10` | Yaesu | 9-digit VFO, `MD0`, `FT`, signed `IS`, bandwidth table. |
| `yaesu-ft710` | Yaesu | Same broad command style as FTDX-10. |
| `yaesu-ft891` | Yaesu | 9-digit VFO, `ST` split, narrower mode/filter set. |
| `yaesu-ft991` | Yaesu | 9-digit VFO, `MD0`, `FT`, C4FM mode, bandwidth table. |

All of these should be implemented by one shared protocol engine plus
per-profile codecs and capability metadata.

## Shared Protocol Rules

All commands and responses are ASCII text frames delimited by `;`.

Examples:

```text
FA;
FA00014074000;
MD;
MD2;
IF;
AI2;
```

The shared engine should:

- split incoming bytes into semicolon-terminated frames
- preserve frame order
- handle multiple frames in one read
- handle partial frames across reads
- route command responses and unsolicited async frames through the same decoder
- tolerate unknown non-error frames by logging and ignoring them
- never expose raw CAT syntax through normal public commands

## Error Frames

The protocol engine should recognize these error responses before profile
decoding:

| Frame | Meaning | Driver behavior |
| --- | --- | --- |
| `?;` | Command syntax error | Fail the active transaction with command-rejected/syntax error. |
| `E;` | Communications error | Fail or retry the active transaction based on retry policy. |
| `O;` | Processing not complete | Retry the active transaction after a short backoff, then fail if exhausted. |
| `XX?;` | Elecraft syntax error for command `XX` | Fail the matching transaction with command-rejected/syntax error. |

The existing `RadioError` should gain protocol-specific variants before driver
implementation, for example:

```rust
RadioError::ProtocolSyntax { command: Option<&'static str> }
RadioError::ProtocolCommunication
RadioError::ProtocolBusy
RadioError::Decode { command: &'static str, message: String }
RadioError::Timeout { command: &'static str }
```

`?;`, `E;`, `O;`, and `XX?;` are not ordinary unknown frames. They should be
attached to the transaction in flight when possible.

## Recommended Module Layout

Keep the public API modules where they are. Add the protocol implementation
under `src/protocol/kenwood_ascii` and register profiles through
`src/drivers`.

```text
src/
  protocol/
    mod.rs
    kenwood_ascii/
      mod.rs
      frame.rs
      driver.rs
      profile.rs
      transaction.rs
      poll.rs
      error.rs
      commands/
        mod.rs
        frequency.rs
        info.rs
        mode.rs
        split.rs
        rit_xit.rs
        filter.rs
        rf.rs
        tx.rs
        cw.rs
      profiles/
        mod.rs
        kenwood.rs
        elecraft.rs
        yaesu.rs
```

The public driver IDs should be model/profile IDs, not one generic ID that
requires users to pass a model string in `options`.

Examples:

```text
kenwood-ts590
kenwood-ts890
kenwood-ts990
kenwood-ts2000
kenwood-ts480
kenwood-ts570
kenwood-ts870
kenwood-if232
elecraft-k4
elecraft-k3
elecraft-k2
yaesu-ftdx101
yaesu-ftdx10
yaesu-ft710
yaesu-ft891
yaesu-ft991
```

The K3 family driver can initially share one profile ID if the CAT behavior is
identical for `K3S`, `K3`, `KX3`, and `KX2`; otherwise split those into separate
profile IDs later.

## Engine/Profile Split

The engine owns transport mechanics:

- frame splitting
- command serialization
- transaction matching
- timeout and retry policy
- read/write loop integration with `RadioTask`
- poll scheduling
- unsolicited frame dispatch
- state patch emission

Profiles own radio semantics:

- supported capabilities
- startup query plan
- auto-info command
- polling plan
- frequency width and formatting
- mode-code mapping
- data-mode composition
- `IF;` parser shape
- VFO/main/sub mapping
- split implementation
- RIT/XIT implementation
- filter implementation
- RF/DSP feature implementation
- PTT command shape
- CW buffer rules

This avoids duplicated serial/TCP protocol code while keeping same-prefix
differences safe.

## Command-Family Codec Contracts

Each command family should be implemented as a small codec with profile
configuration. A codec should not own transport I/O or mutate `RadioState`
directly. It should:

- encode a supported `RadioCommand` or startup/poll query into one or more
  semicolon-terminated frames
- decode matching command responses and unsolicited frames
- return `StatePatch` values for the actor/reducer
- report unsupported combinations before anything is written to the radio
- expose enough metadata for transaction matching and polling

Recommended shared types:

```rust
struct EncodedCommand {
    frames: Vec<AsciiFrame>,
    matcher: ResponseMatcher,
    optimistic: Vec<StatePatch>,
    priority: CommandPriority,
}

struct DecodedFrame {
    patches: Vec<StatePatch>,
    source_hint: Option<UpdateSource>,
}
```

The actual structs can differ, but the separation should remain: command codecs
translate protocol, while `StateReducer` owns state changes and change flags.

### Frequency Codec

Inputs:

- `SetReceiverFrequency { receiver, frequency }`
- `SetTxFrequency(frequency)`
- startup/poll queries for `FA;` and `FB;`

Responsibilities:

- format 11-digit or 9-digit Hz values based on profile
- map main/sub receiver paths to `FA`, `FB`, or profile-specific main/sub
  targets
- update `MainRxFrequency`, `SubRxFrequency`, or `TxFrequency` patches based on
  profile VFO mapping
- avoid treating VFO B as `sub_rx` on radios without a true sub receiver

### Info/Status Codec

Inputs:

- startup/poll query for `IF;` where supported
- unsolicited `IF...;` frames after auto-info is enabled

Responsibilities:

- parse Kenwood 35-character and Yaesu 25-character `IF` layouts separately
- emit all state patches present in the status frame as one decoded snapshot
- update transmitting and split state when present
- use profile mode maps when decoding mode characters from `IF`
- ignore documented padding and unknown fields

### Mode Codec

Inputs:

- `SetReceiverMode { receiver, mode }`
- `SetTxMode(mode)`
- startup/poll queries such as `MD;`, `MD0;`, `MD1;`, `SF0;`, `OM0;`, `DT;`,
  and `$` variants

Responsibilities:

- map normalized `Mode` values into profile-specific mode codes
- compose or split data-mode state for TS-590 and Elecraft profiles
- decode TS-890 `SF` frames into both frequency and mode patches
- decode TS-990 `OM` frames by target
- handle Yaesu `MDX` target prefixes
- reject modes unsupported by the selected profile with
  `RadioError::UnsupportedCapability` or `RadioError::InvalidValue`

### Split and Routing Codec

Inputs:

- `SetSplit(bool)`
- startup/poll queries for `FR;`, `FT;`, `SP;`, or `ST;`

Responsibilities:

- track RX VFO and TX VFO routing where the radio models split as VFO
  selection
- implement `SetSplit(true)` by selecting the opposite TX VFO only after the RX
  VFO is known
- implement direct split toggles for `SP` and `ST` profiles
- avoid losing split on Elecraft K3-family `FR` operations by re-enabling split
  when required
- emit `Split`, `TxFrequency`, and receiver-frequency patches when routing
  changes imply them

### RIT/XIT Codec

Inputs:

- `SetRitEnabled(bool)`
- `SetXitEnabled(bool)`
- `SetRitXitOffset(offset)`
- startup/poll queries for `RT;`, `XT;`, `RF;`, `RO;`, or status-derived offset

Responsibilities:

- encode common `RT`/`XT` commands and Elecraft `$` variants
- decode offset from `IF`, `RF`, or `RO` based on profile
- implement absolute offset setting over relative `RU`/`RD` commands by querying
  current offset, sending the delta, and confirming the result
- treat K2 fixed-step `RU;`/`RD;` as non-exact until an iterative strategy is
  explicitly implemented
- keep the normalized offset inside `RitXitOffsetHz`

### Filter Codec

Inputs:

- `SetReceiverFilterBandwidth { receiver, bandwidth_hz }`
- `SetReceiverFilterShift { receiver, shift_hz }`
- startup/poll filter queries

Responsibilities:

- route CW/FSK direct `FW`/`IS` profiles separately from phone/data hi/lo cut
  conversion profiles
- implement Elecraft `BW`/`IS` and `$` variants
- implement Yaesu `SH` table lookup after bandwidth CSV files are available
- preserve signed Yaesu `IS` semantics once the public filter-shift API is
  changed
- emit bandwidth and shift patches only for values the current API can
  represent honestly

### RF/DSP Codec

Inputs:

- `SetReceiverPreamp`
- `SetReceiverAttenuator`
- `SetReceiverNoiseBlanker`
- `SetReceiverNoiseReduction`
- `SetReceiverAutoNotch`
- startup/poll RF and DSP queries

Responsibilities:

- map profile values to `LeveledSetting` or boolean auto-notch state
- keep physical dB levels where the profile exposes them
- handle target-prefixed main/sub commands for TS-990 and FTDX-101
- handle Elecraft `$` variants for sub receiver/VFO commands
- avoid claiming complete support for multiple independent controls when the
  public API has only one field
- implement cycling controls such as K2 `NB1;` only with query/confirm loops

### TX Codec

Inputs:

- `SetTxPowerDeciMw`
- `SetPtt(bool)`
- startup/poll `PC;` and transmitting-state queries where available

Responsibilities:

- convert `power_deci_mw` to profile power units and validate range before send
- encode K4 `PCNNNR` ranges without losing low-power precision
- map boolean PTT to each profile's normal send/receive commands
- track local transmitting state for profiles without a getter
- prefer confirmed `IF`, `TX`, or `RX` frames over optimistic local state

### CW/Keyer Codec

Inputs:

- `SetKeyerSpeed(wpm)`
- `SendCw(text)`
- `StopCw`
- startup/poll `KS;` and optional `KY;`

Responsibilities:

- validate profile WPM range before sending `KS`
- chunk or reject CW text based on profile buffer size
- validate the profile-specific CW character set
- encode the correct stop command: `KY0;`, `RX;`, or `KY @;`
- update `KeyerSpeed` and `KeyerSending` when the profile can report them

## Startup Sequence

Startup should explicitly load state for every supported frontend API field.
Async auto-info should not be treated as a substitute for initial state loading.

Recommended startup:

1. Open transport.
2. Set connection state to `Connecting`.
3. Flush stale input if the transport supports it.
4. Set connection state to `Identifying`.
5. Optionally query identity if the profile supports an identity command.
6. Enable auto-info with the profile's configured `AI` command.
7. Execute the profile's explicit startup query plan.
8. Apply decoded state patches through `StateReducer`.
9. Set connection state to `Ready`.
10. Start normal async receive handling and the poll scheduler.

Auto-info frames may arrive while the startup query plan is running. The driver
must decode them and apply them through the same reducer used for query
responses.

## Auto-Info Commands

| Profile | Auto-info command |
| --- | --- |
| Kenwood TS-590 | `AI2;` |
| Kenwood TS-890 | `AI2;` |
| Kenwood TS-990 | `AI2;` |
| Kenwood TS-2000 | `AI2;` |
| Kenwood TS-480 | `AI2;` |
| Kenwood TS-570 | `AI2;` |
| Kenwood TS-870 | `AI2;` |
| Kenwood IF-232 Protocol | `AI1;` |
| Elecraft K4 | `AI2;AID250;` |
| Elecraft K3S/K3/KX3/KX2 | `AI2;` |
| Elecraft K2 | `AI2;` |
| Yaesu FTDX-101 | `AI1;` |
| Yaesu FTDX-10 | `AI1;` |
| Yaesu FT-710 | `AI1;` |
| Yaesu FT-891 | `AI1;` |
| Yaesu FT-991 | `AI1;` |

`AI2;AID250;` should be represented as a startup command sequence, not as one
opaque frame.

## Async and Polling

The public API promise remains: applications subscribe to async state updates.
The implementation may use native unsolicited frames, polling, or a hybrid.

The CSV marks most profiles as full async, and `kenwood-if232` as limited
async. Even for full-async profiles, the driver should still poll fields that
are not represented in the profile's async frames.

Default polling policy:

| Category | Default interval | Examples |
| --- | --- | --- |
| Native async | no poll unless missing from async data | frames emitted after `AI` command |
| Limited async gap fill | 2 seconds, configurable | fields not available from `IF;` |
| Slow background confirmation | 10 to 30 seconds, configurable | keyer speed, RF power, filter settings |
| Manual refresh | immediate | explicit `RadioCommand::Refresh` |

The user requirement says limited-async radios need slow configurable polling
starting at once per 2 seconds for data not available in `IF;`. Implement this
as profile poll metadata rather than hard-coded loops.

Suggested config option syntax:

```text
poll_gap_ms=2000;poll_slow_ms=10000
```

If `RadioConfig::options` remains a raw string, parse it conservatively. A
typed options struct would be better before implementing several real drivers.

## Initial Query Plan

Each profile should list explicit query commands for all supported API fields:

| API state | Typical query commands |
| --- | --- |
| main frequency | `FA;`, sometimes also parsed from `IF;` |
| sub/VFO B frequency | `FB;` |
| main mode | `MD;`, `MD0;`, `SF0;`, `OM0;`, or parsed from `IF;` |
| sub mode | `MD1;`, `SF1;`, `OM1;`, or `$` variants |
| split | `FT;`, `SP;`, `ST;`, or parsed from `IF;` |
| RX/TX VFO routing | `FR;`, `FT;`, profile-specific |
| RIT enable | `RT;` or `RT$;` |
| XIT enable | `XT;` or `XT$;` |
| RIT/XIT offset | `IF;`, `RF;`, `RO;`, or profile-specific |
| filter bandwidth | `FW;`, `BW;`, `BW$;`, `SH...;`, or hi/lo conversion |
| filter shift | `IS;`, `IS$;`, signed `IS...;`, or hi/lo conversion |
| auto notch | `NT;`, `NTX;`, `NA$;`, `BC...;` |
| noise blanker | `NB;`, `NB1;`, `NB2;`, targeted variants |
| noise reduction | `NR;`, `NRX;`, `NR0;`, targeted variants |
| preamp | `PA;`, `PAX;`, `PA0;`, `$` variants |
| attenuator | `RA;`, `RAX;`, `RA0;`, `$` variants |
| RF power | `PC;` |
| PTT/transmitting | `IF;` if available, otherwise local tracking plus async frames |
| keyer speed | `KS;` |

If a profile cannot query a field but can set it, its capability should be
`WriteOnly`. If it can neither query nor set a field, use `Unsupported`.

## Profile Startup Plans

The startup plan should be profile metadata. Each listed command is a semantic
query step; the profile codec expands it to the exact CAT frame and target.

For full-async profiles, these queries still run once at startup so the state
cache is complete before relying on auto-info. For limited-async profiles, the
same query list seeds the poll plan for fields not carried by async `IF;`.

| Profile | Startup query set | Gap/slow polling after startup |
| --- | --- | --- |
| `kenwood-ts590` | `IF`, `FA`, `FB`, `FR`, `FT`, `MD`, `DA`, `RT`, `XT`, filter state, `NT`, `NB`, `NR`, `PA`, `RA`, `PC`, `KS`. | Poll filter/RF/keyer/power slowly unless auto-info proves those frames arrive. Poll inactive VFO if `IF` only reports active VFO changes. |
| `kenwood-ts890` | `FA`, `FB`, `FR`, `FT`, `SF0`, `SF1`, `RT`, `XT`, `RF`, hi/lo filter state, `NT`, `NB1`, `NB2`, `NR`, `PA`, `RA`, `PC`, `KS`. | Poll hi/lo filter state, `NB1`/`NB2`, RF power, and keyer speed slowly. |
| `kenwood-ts990` | `FA`, `FB`, `SP`, `OM0`, `OM1`, `RT`, `XT`, `RF`, hi/lo filter state for main/sub, `NT0`, `NT1`, `NB10`, `NB11`, `NB20`, `NB21`, `NR0`, `NR1`, `PA0`, `PA1`, `RA0`, `RA1`, `PC`, `KS`. | Poll main/sub filter and RF/DSP state slowly. This profile has no generic `IF` row in the CSV, so do not depend on `IF` snapshots. |
| `kenwood-ts2000` | `IF`, `FA`, `FB`, `FR`, `FT`, `MD`, `RT`, `XT`, filter state, `NT`, `NB`, `NR`, `PA`, `RA`, `PC`, `KS`. | Poll filter/RF/keyer/power slowly. Poll inactive VFO if needed. |
| `kenwood-ts480` | `IF`, `FA`, `FB`, `FR`, `FT`, `MD`, `RT`, `XT`, filter state, `NB`, `NR`, `PA`, `RA`, `PC`, `KS`. | Poll filter/RF/keyer/power slowly. Auto notch is unsupported by this matrix. |
| `kenwood-ts570` | `IF`, `FA`, `FB`, `FR`, `FT`, `MD`, `RT`, `XT`, `NB`, `NR`, `PA`, `RA`, `PC`, `KS`. | Poll RF/keyer/power slowly. Filter bandwidth/shift is unsupported by this matrix. |
| `kenwood-ts870` | `IF`, `FA`, `FB`, `FR`, `FT`, `MD`, `RT`, `XT`, `NB`, `NR`, `PA`, `RA`, `PC`, `KS`. | Poll RF/keyer/power slowly. Filter bandwidth/shift is unsupported by this matrix. |
| `kenwood-if232` | `IF`, `FA`, `FB`, `SP`, `MD`, `RT`, `XT`. | Limited async: use `AI1;` for `IF` events, then poll `FA`/`FB` and any supported non-`IF` fields every 2 seconds by default. |
| `elecraft-k4` | `FA`, `FB`, `FT`, `MD`, `DT`, `MD$`, `DT$`, `RT`, `XT`, `RO`, `RT$`, `XT$`, `RO$`, `BW`, `BW$`, `IS`, `IS$`, `NA`, `NA$`, `NB`, `NB$`, `NR`, `NR$`, `PA`, `PA$`, `RA`, `RA$`, `PC`, `KS`. | Poll RF/filter/keyer/power slowly if not emitted after `AI2;AID250;`. `$` variants are sub receiver/VFO queries. |
| `elecraft-k3-family` | `IF`, `FA`, `FB`, `FT`, `MD`, `DT`, `MD$`, `DT$`, `RT`, `XT`, `RO`, `BW`, `BW$`, `IS`, `IS$`, `NB`, `NB$`, `PA`, `PA$`, `RA`, `RA$`, `PC`, `KS`. | Poll filter/RF/keyer/power slowly. Re-assert split after `FR` operations if the operation cancels split. |
| `elecraft-k2` | `IF`, `FA`, `FB`, `FR`, `FT`, `MD`, `RT`, `XT`, `FW`, `NB`, `PA`, `RA`, `PC`, `KS`. | Poll filter/RF/keyer/power slowly. Treat fixed-step RIT and cycling `NB1;` behavior as non-exact until confirmed. |
| `yaesu-ftdx101` | `IF`, `FA`, `FB`, `FT`, `MD0`, `MD1`, `RT`, `XT`, `SH0`, `SH1`, `IS0`, `IS1`, `BC0`, `BC1`, `NB0`, `NB1`, `NR0`, `NR1`, `PA0`, `PA1`, `RA0`, `RA1`, `PC`, `KS`. | Poll table-backed filter state, RF/DSP state, RF power, and keyer speed slowly. |
| `yaesu-ftdx10` | `IF`, `FA`, `FB`, `FT`, `MD0`, `RT`, `XT`, `SH0`, `IS0`, `BC0`, `NB0`, `NR0`, `PA0`, `RA0`, `PC`, `KS`. | Poll table-backed filter state, RF/DSP state, RF power, and keyer speed slowly. |
| `yaesu-ft710` | `IF`, `FA`, `FB`, `FT`, `MD0`, `RT`, `XT`, `SH0`, `IS0`, `BC0`, `NB0`, `NR0`, `PA0`, `RA0`, `PC`, `KS`. | Same poll shape as `yaesu-ftdx10`. |
| `yaesu-ft891` | `IF`, `FA`, `FB`, `ST`, `MD0`, `RT`, `XT`, `SH0`, `IS0`, `BC0`, `NB0`, `NR0`, `PA0`, `RA0`, `PC`, `KS`. | Poll table-backed filter state, RF/DSP state, RF power, and keyer speed slowly. Use `ST`, not `FT`, for split. |
| `yaesu-ft991` | `IF`, `FA`, `FB`, `FT`, `MD0`, `RT`, `XT`, `SH0`, `IS0`, `BC0`, `NB0`, `NR0`, `PA0`, `RA0`, `PC`, `KS`. | Poll table-backed filter state, RF/DSP state, RF power, and keyer speed slowly. Mode code `E` is `C4FM`. |

The profile's startup plan should omit unsupported fields entirely rather than
query and ignore a predictable error response.

## Frequency Codec

Most Kenwood and Elecraft profiles use `FA`/`FB` with 11 decimal digits in Hz:

```text
FA00014074000;
FB00014074000;
```

Yaesu profiles in the CSV use 9 decimal digits in Hz:

```text
FA014074000;
FB014074000;
```

Implementation:

```rust
struct FrequencyFormat {
    digits: usize,
}
```

Profile mapping:

| Profiles | Digits |
| --- | --- |
| Kenwood TS-590/890/990/2000/480/570/870/IF-232 | 11 |
| Elecraft K4/K3-family/K2 | 11 |
| Yaesu FTDX-101/FTDX-10/FT-710/FT-891/FT-991 | 9 |

The normalized API continues to use `Frequency`.

## Receiver Path and VFO Mapping

The public API uses `ReceiverPath::Main` and `ReceiverPath::Sub`.

Profile mapping rules:

- Radios with VFO A/B but no sub receiver expose only `main_rx`.
- VFO B frequency is still useful for split and TX frequency, but should not
  imply `sub_rx: Some(...)` unless the radio has an independent sub receiver.
- Radios marked `Has Sub Rx = Yes` expose `sub_rx`.
- `tx.frequency` should be populated from the active TX VFO when known.
- `tx.split` should represent whether TX and RX are separated, not merely the
  raw value of `FT`, `SP`, or `ST`.

Profiles with sub receiver in the CSV:

| Profile | Sub receiver |
| --- | --- |
| Kenwood TS-990 | yes |
| Elecraft K4 | yes |
| Elecraft K3S/K3/KX3/KX2 | yes |
| Yaesu FTDX-101 | yes |

All other listed profiles expose VFO A/B without an independent `sub_rx`.

## Info and Status Frames

### Kenwood 35-character `IF`

The Kenwood-style `IF` response is listed as `IF` plus 35 characters and `;`.

Fields from the CSV:

| Position | Meaning |
| --- | --- |
| first 11 chars | active VFO frequency in Hz |
| next 5 chars | ignored |
| next 5 chars | RIT/XIT offset: sign plus four Hz digits |
| next 1 char | RIT enabled |
| next 1 char | XIT enabled |
| next 3 chars | ignored |
| next 1 char | transmitting, `0` no, `1` yes |
| next 1 char | operating mode |
| next 1 char | active VFO, `0` A, `1` B |
| next 1 char | split enabled |
| remaining chars | ignored |

Implement this parser once for the profiles that share it.

### Yaesu 25-character `IF`

The Yaesu profiles use `IF` plus 25 characters and `;`.

Fields from the CSV:

| Position | Meaning |
| --- | --- |
| first 3 chars | ignored |
| next 9 chars | VFO A frequency in Hz |
| next 5 chars | RIT/XIT offset with leading sign |
| next 1 char | RIT enabled |
| next 1 char | XIT enabled |
| next 1 char | mode |
| next 4 chars | ignored |
| next 1 char | split, `0` disabled, `1` or `2` enabled |

Implement this as a separate status parser from the Kenwood 35-character
parser.

### Profiles Without `IF` in the Matrix

The CSV lists `Get Info` as `n/a` for TS-890, TS-990, and K4. Those profiles
must load state from explicit field queries and their profile-specific async
frames instead of relying on a generic `IF` startup snapshot.

## Mode Codec

Mode handling must be profile-specific. The current `Mode` enum is too small
for the CSV because it lacks at least:

- `FSK` as a named public value, unless it intentionally remains normalized to
  `RTTY`
- `PSK`
- `PSK-R`
- `DATA-AM`
- `DATA-FM-N`
- `FM-N`
- `AM-N`
- `AFSK`
- `C4FM`

Decision needed: expand `Mode`, add a `DigitalMode`/submode structure, or map
unsupported variants to `Mode::Digital`. The least lossy implementation is to
expand `Mode` for all modes listed in the CSV.

Decision needed: the CSV uses `FSK`/`FSK-R`, while the current API exposes
`Rtty`/`RttyReverse`. Decide whether the public API should keep normalizing FSK
to RTTY, rename/add FSK variants, or support aliases while preserving the
current Rust enum names.

### Shared Kenwood `MD` Table

Common Kenwood-style `MD` values:

| Code | Mode |
| --- | --- |
| `1` | LSB |
| `2` | USB |
| `3` | CW |
| `4` | FM |
| `5` | AM |
| `6` | FSK/RTTY |
| `7` | CW-R |
| `8` | unused on many profiles |
| `9` | FSK-R/RTTY-R |

TS-590 composes the final mode from `MD` plus `DA`:

- `DA0` means normal mode.
- `DA1` means data mode for phone modes.
- CW and FSK modes do not use the data flag.

TS-2000, TS-480, TS-570, TS-870, and IF-232 use the simpler `MD` table in the
CSV.

### TS-890 `SF`

TS-890 uses:

```text
SF0;
SFXHHHHHHHHHHHY;
```

`X` is VFO A/B and `Y` is mode. The response also carries frequency, so the
decoder should emit both frequency and mode patches when both are present.

Mode values:

```text
1 LSB, 2 USB, 3 CW, 4 FM, 5 AM, 6 FSK, 7 CW-R, 9 FSK-R,
A PSK, B PSK-R, C LSB-D, D USB-D, E FM-D, F AM-D
```

### TS-990 `OM`

TS-990 uses:

```text
OM0;
OM1;
OMXY;
```

`X` is RX/TX mode target and `Y` is the mode code.

Mode values include the standard codes plus:

```text
A PSK, B PSK-R,
C LSB-D1, D USB-D1, E FM-D1, F AM-D1,
G LSB-D2, H USB-D2, I FM-D2, J AM-D2,
K LSB-D3, L USB-D3, M FM-D3, N AM-D3
```

The public API probably should normalize `D1`/`D2`/`D3` variants to the same
`DATA-*` mode unless a future API exposes data-mode profile details.

### Elecraft K4/K3 Family

K4 and K3-family modes are composed from `MD` plus `DT`:

- `MD1` LSB
- `MD2` USB
- `MD3` CW
- `MD4` FM
- `MD5` AM
- `MD6` Data
- `MD7` CW-R
- `MD9` DATA-R
- `DT0` DATA
- `DT1` AFSK
- `DT2` FSK
- `DT3` PSK

Commands with `$` target the sub VFO on K4/K3-family profiles. No `$` means
main VFO.

### Elecraft K2

K2 mode values:

```text
1 LSB, 2 USB, 3 CW, 6 FSK, 7 CW-R, 9 FSK-R
```

### Yaesu `MDX`

Yaesu profiles use target-prefixed mode commands:

```text
MDX;
MDXY;
```

`X` is `0` main or `1` sub on FTDX-101. Single-receiver Yaesu profiles use
`MD0`.

Common values:

```text
1 LSB, 2 USB, 3 CW, 4 FM, 5 AM, 6 FSK, 7 CW-R,
8 DATA-LSB, 9 FSK-R, A DATA-FM, B FM-N, C DATA-USB,
D AM-N, E PSK, F DATA-FM-N
```

FT-891 does not list `A`, `E`, or `F`. FT-991 uses `E` for C4FM instead of PSK.

## Split and TX/RX Routing

The normalized frontend API has:

```rust
RadioCommand::SetSplit(bool)
TransmitterState { split: Option<bool>, ... }
```

Profiles must translate this into model-specific VFO routing.

| Profiles | Split mechanism |
| --- | --- |
| TS-590/890/2000/480/570/870/K2 | `FT;`, `FT0;`, `FT1;` where `FT` selects TX VFO. Split means TX VFO differs from RX VFO. |
| TS-990 and Kenwood IF-232 | `SP;`, `SP0;`, `SP1;` |
| Elecraft K4 | `FT;`, `FT0;`, `FT1;` where CSV describes off/on. |
| Elecraft K3 family | enable with `FT1;`, disable with `FR0;`; `FR` cancels split. |
| Yaesu FTDX-101/10/710/991 | `FTX;`, set `FT2;` or `FT3;`, response `FT0;` or `FT1;`. |
| Yaesu FT-891 | `ST;`, `ST0;`, `ST1;` |

Where split is expressed as TX VFO selection, the driver must know the current
RX VFO before it can implement `SetSplit(true)`. If RX VFO is unknown, query it
first or return a structured error that asks the caller to refresh.

## RIT/XIT

Enable commands are broadly shared:

```text
RT;
RT0;
RT1;
XT;
XT0;
XT1;
```

Elecraft K4 supports `$` variants for sub VFO:

```text
RT$;
RT$0;
XT$;
XT$0;
```

Clear commands:

| Profiles | Clear |
| --- | --- |
| Most profiles | `RC;` |
| Elecraft K4 | `RC$;` for sub |

Offset handling differs:

| Profiles | Offset query/set |
| --- | --- |
| TS-590/2000/480/570/870/IF-232/Yaesu/K2 | Query from `IF;`; set relative with `RU...;` and `RD...;`. |
| TS-890/TS-990 | Query `RF;`; response `RFSXXXX;`. |
| Elecraft K4 | `RO$;`, `RO$SNNNN;`. |
| Elecraft K3 family | `RO;`, `ROSNNNN;`. |

Current `SetRitXitOffset` is absolute, but many profiles only support relative
up/down commands. The driver should:

1. read the current offset first
2. compute delta to target
3. send `RU` or `RD`
4. confirm with `IF;`, `RF;`, or `RO;`

If current offset is unknown and the profile only supports relative set,
absolute set should query before sending.

K2 is a special case: the CSV lists `RU;` and `RD;` with a fixed radio-defined
step instead of an explicit Hz argument. Absolute `SetRitXitOffset` cannot be
implemented exactly for K2 unless the driver iterates fixed steps and confirms
the result. Until that behavior is proven, K2 offset setting should be exposed
as unsupported or as a profile-specific extension rather than pretending to be
an exact absolute setter.

## Filters

The public API exposes:

```rust
ReceiverFilterState {
    bandwidth_hz: Option<u16>,
    shift_hz: Option<u16>,
}
```

This matches Kenwood and Elecraft absolute-width style reasonably well, but it
does not fully match Yaesu signed IF shift. Decision needed: change
`shift_hz` and `SetReceiverFilterShift` from `u16` to `i16`, or introduce a
new signed `FilterShiftHz` newtype.

### Bandwidth

| Profiles | Bandwidth strategy |
| --- | --- |
| TS-590/TS-2000/TS-480 | CW/FSK use `FW`; phone/data use hi/lo cut conversion. |
| TS-890/TS-990 | Convert API bandwidth/shift to hi/lo cut. |
| TS-570/TS-870/IF-232 | Unsupported in CSV. |
| Elecraft K4/K3 family | `BW` or `BW$`; value is Hz divided by 10. |
| Elecraft K2 | `FW`; direct Hz. |
| Yaesu FTDX-101/10/710 | `SH...` with table lookup from `ftdx101-bandwidth.csv`. |
| Yaesu FT-891 | `SH...` with table lookup from `ftdx891-bandwidth.csv`. |
| Yaesu FT-991 | `SH...` with table lookup from `ft991-bandwidth.csv`. |

### Shift

| Profiles | Shift strategy |
| --- | --- |
| TS-590/TS-2000/TS-480 | CW/FSK use `IS`; phone/data use hi/lo cut conversion. |
| TS-890/TS-990 | Convert API bandwidth/shift to hi/lo cut. |
| TS-570/TS-870/IF-232/K2 | Unsupported in CSV. |
| Elecraft K4/K3 family | `IS` or `IS$`; direct Hz. |
| Yaesu profiles | Signed `IS` offset from center, not absolute frequency. |

The filter implementation should be table-driven by mode family. Do not put the
hi/lo conversion math in the command router.

## RF and DSP Controls

The existing `LeveledSetting` type works for most preamp, attenuator, noise
blanker, and noise reduction rows:

```rust
LeveledSetting {
    enabled: Option<bool>,
    level: Option<u8>,
}
```

Mapping examples:

- on/off preamp: `enabled=true`, `level=1`
- off: `enabled=false`, `level=0`
- TS-890 preamp 2: `enabled=true`, `level=2`
- TS-890 attenuator 18 dB: `enabled=true`, `level=18` or profile code `3`

Decision needed: choose whether attenuator `level` is a physical dB value or a
profile-specific ordinal. Physical dB is better for the public API, but some
profiles only expose ordinal on/off state.

Decision needed: choose how to expose multiple independent noise blankers such
as TS-890 `NB1`/`NB2` and TS-990 `NB1`/`NB2`. Options:

- map the first or active blanker into the existing single `noise_blanker`
  field
- add indexed/multiple RF controls to the frontend API
- expose profile-specific extension commands later

Until decided, profile docs should record both blankers but the normalized API
should only claim support for the one it can represent honestly.

### RF/DSP Command Families

These command families are similar enough to share parsing helpers, but the
profile must still define the exact shape, target, and value map.

| Feature | Profiles | Commands |
| --- | --- | --- |
| Auto notch | TS-590 | `NT;`; set `NT10;`; response `NTXX`, first digit selects disabled/auto/manual. |
| Auto notch | TS-890, TS-2000 | `NT;`; set `NT0;`/`NT1;`. |
| Auto notch | TS-990 | `NTX;`; set `NTX0;`/`NTX1;`/`NTX2;`/`NTX3;`, where `X` is main/sub. |
| Auto notch | Elecraft K4 | `NA$;`; set `NA$0;`/`NA$1;`; `$` selects sub. |
| Auto notch | Yaesu profiles | `BC...;`; target is main/sub or `0` depending on model. |
| Noise blanker | TS-590 | `NB;`; set `NB1;` or `NB2;` for blanker selection. |
| Noise blanker | TS-890 | `NB1;`, `NB2;`; set `NB10;`, `NB11;`, `NB20;`, `NB21;`. |
| Noise blanker | TS-990 | `NB1X;`, `NB2X;`; set `NB1X0;`/`NB1X1;`, `NB2X0;`/`NB2X1;`. |
| Noise blanker | TS-2000/480/570/870 | `NB;`; set `NB0;`/`NB1;`. |
| Noise blanker | Elecraft K4/K3 family | `NB$;`; set `NB$0;`/`NB$1;`; `$` selects sub. |
| Noise blanker | Elecraft K2 | `NB;`; `NB1;` toggles/cycles, so setting a target value requires query and repeat. |
| Noise blanker | Yaesu profiles | `NBX;` or `NB0;`; set with target plus `0`/`1`. |
| Noise reduction | TS-590/890/2000/480/570/870 | `NR;`; set `NR0;`, `NR1;`, or `NR2;`. |
| Noise reduction | TS-990 | `NRX;`; set `NRX0;`, `NRX1;`, or `NRX2;`. |
| Noise reduction | Elecraft K4 | `NR$;`; set `NR$NNM`, where `NN` is 0-10 level and `M` is off/on. |
| Noise reduction | Yaesu profiles | `NRX;` or `NR0;`; set with target plus `0`/`1`. |
| Preamp | TS-590/2000/570/870/K2 | `PA;`; set `PA0;`/`PA1;`. |
| Preamp | TS-890 | `PA;`; set `PA0;`, `PA1;`, or `PA2;`. |
| Preamp | TS-990 | `PAX;`; set `PAX0;`/`PAX1;`. |
| Preamp | TS-480 | `PA;`; set `PA0;`/`PA1;`; response shape may be `PA00;` or `PA10;`. |
| Preamp | Elecraft K4/K3 family | `PA$;`; set `PA$0;` through the profile's max preamp level. |
| Preamp | Yaesu profiles | `PAX;` or `PA0;`; values are off/preamp1/preamp2 where supported. |
| Attenuator | TS-590/2000/480/570/870 | `RA;`; set `RA0;`/`RA1;`. |
| Attenuator | TS-890 | `RA;`; set `RA0;`, `RA1;`, `RA2;`, `RA3;` for off/6/12/18 dB. |
| Attenuator | TS-990 | `RAX;`; set `RAX0;`, `RAX1;`, `RAX2;`, `RAX3;`. |
| Attenuator | Elecraft K4 | `RA$;`; set `RA$NNM`, where `NN` is dB and `M` is off/on. |
| Attenuator | Elecraft K3 family | `RA$;`; set `RA$NN`, where `NN` is off/on or dB code by model. |
| Attenuator | Elecraft K2 | `RA;`; set `RA00;`/`RA01;`. |
| Attenuator | Yaesu profiles | `RAX;` or `RA0;`; values are off/6/12/18 dB on larger models, off/on on FT-891/FT-991. |

Profiles with `n/a` in the CSV for these rows should set the corresponding
capability to `Unsupported` even if the shared command family exists for other
profiles.

## RF Power

The current API uses:

```rust
SetTxPowerDeciMw(u32)
TransmitterState::power_deci_mw
```

Most profiles use `PCXXX;` where `XXX` is watts:

```text
PC005;
PC100;
```

Convert between watts and deci-milliwatts:

```text
watts = power_deci_mw / 10_000
power_deci_mw = watts * 10_000
```

Profile limits from the CSV:

| Profiles | Range |
| --- | --- |
| TS-590/TS-2000 | 5 to 100 W |
| TS-890 | 5 to 100 W |
| TS-990 | 5 to 200 W |
| TS-480/TS-570/TS-870 | 5 to 200 W |
| Elecraft K3 family | 0 to 110 W |
| Elecraft K2 | 0 to 150 W |
| Yaesu FTDX-101 | 5 to 200 W |
| Yaesu FTDX-10/FT-710/FT-891/FT-991 | 5 to 100 W |

Elecraft K4 uses `PCNNNR` with range suffix `L`, `H`, or `X`. The normalized
API can still use `power_deci_mw`; the K4 profile chooses the suffix.

## PTT

The current API exposes `set_ptt(bool)`.

Profile mappings:

| Profiles | Transmit | Receive |
| --- | --- | --- |
| TS-590/890/990/480 | `TX0;` plus optional data/tune variants | `RX;` |
| TS-2000 | `TX0;` | `RX;` |
| TS-570/870/IF-232/K4/K3/K2 | `TX;` | `RX;` |
| Yaesu profiles | `TX1;` data send, `TX2;` send | `TX0;` |

Decision needed: keep boolean PTT only, or add a transmit mode API for data
send and tune. Boolean PTT can map to the normal send form, but it cannot
express tune or data-send variants.

Because some profiles have no PTT getter, the driver should track local PTT
state after successful command sends and correct it from `IF;` or async `TX`/`RX`
frames when available.

## CW and Keyer

Current API:

```rust
set_keyer_speed(wpm)
send_cw(text)
stop_cw()
```

This maps well to the CSV.

CW send:

| Profiles | Command | Buffer |
| --- | --- | --- |
| Kenwood TS-590/890/990/2000/480/570/870 | `KY text;` | 24 chars or fewer, padded where required. |
| Elecraft K4 | `KY text;` | 60 chars. |
| Elecraft K3 family/K2 | `KY text;` | 24 chars or fewer. |
| Kenwood IF-232 and Yaesu profiles | unsupported in CSV. |

CW stop:

| Profiles | Stop |
| --- | --- |
| TS-590/890/990 | `KY0;` |
| TS-2000/480/570/870 | `RX;` |
| Elecraft K4/K3 family/K2 | `KY @;` |

Keyer speed:

| Profiles | Range |
| --- | --- |
| TS-590/890/990 and Yaesu profiles | 4 to 60 WPM |
| TS-2000/480/570/870 | 10 to 60 WPM |
| Elecraft K4 | 8 to 100 WPM |
| Elecraft K3 family | 8 to 50 WPM |
| Elecraft K2 | 9 to 50 WPM |

The `KY;` query response should update `KeyerState::sending` if the profile can
report buffer availability.

## Capabilities

Capabilities should be generated from profile metadata, not inferred from
whether an encoder happens to return a frame.

Each profile should define:

```rust
struct KenwoodProfile {
    descriptor: DriverDescriptor,
    capabilities: RadioCapabilities,
    update_strategy: StateUpdateCapability,
    startup: StartupPlan,
    poll_plan: PollPlan,
    codecs: Codecs,
}
```

Rules:

- `sub_rx: Some(...)` only for true sub receivers.
- VFO B alone does not create `sub_rx`.
- Unsupported CSV cells map to `Capability::Unsupported`.
- Set-only commands map to `WriteOnly`.
- Query-only commands map to `ReadOnly`.
- Query and set map to `ReadWrite`.
- `state_updates` is `Native` for full async profiles, `Hybrid` for limited
  async plus polling, and `Polling` only if auto-info cannot be enabled.

### Capability Summary Matrix

This matrix summarizes the CSV in terms of the existing `RadioCapabilities`
shape. It is not a replacement for profile-specific command metadata; it is the
starting point for each profile's capabilities value.

Legend:

- `Sub`: whether `RadioCapabilities::sub_rx` should be `Some`.
- `Filter`: receiver filter bandwidth/shift support.
- `RIT/XIT`: enable state plus offset support.
- `RF/DSP`: `AN` auto notch, `NB` noise blanker, `NR` noise reduction, `PA`
  preamp, `RA` attenuator.
- `TX`: `PC` RF power and `PTT` transmit control.
- `Keyer/CW`: `KS` keyer speed, `KY` send CW, `Stop` stop CW.

| Profile | Sub | Filter | RIT/XIT | RF/DSP | TX | Keyer/CW |
| --- | --- | --- | --- | --- | --- | --- |
| `kenwood-ts590` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-ts890` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-ts990` | yes | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-ts2000` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-ts480` | no | BW, shift | RT, XT, offset | NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-ts570` | no | unsupported | RT, XT, offset | NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-ts870` | no | unsupported | RT, XT, offset | NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `kenwood-if232` | no | unsupported | RT, XT, offset | unsupported | PTT | unsupported |
| `elecraft-k4` | yes | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS, KY, Stop |
| `elecraft-k3-family` | yes | BW, shift | RT, XT, offset | NB, PA, RA | PC, PTT | KS, KY, Stop |
| `elecraft-k2` | no | BW only | RT, XT, offset | NB, PA, RA | PC, PTT | KS, KY, Stop |
| `yaesu-ftdx101` | yes | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS |
| `yaesu-ftdx10` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS |
| `yaesu-ft710` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS |
| `yaesu-ft891` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS |
| `yaesu-ft991` | no | BW, shift | RT, XT, offset | AN, NB, NR, PA, RA | PC, PTT | KS |

Capability caveats:

- `elecraft-k2` RIT/XIT offset is fixed-step for set operations, so expose
  absolute offset write only after the iterative behavior is implemented and
  tested.
- Yaesu `shift` is signed. The current unsigned API cannot fully represent it
  until the filter-shift decision is resolved.
- Multi-control features such as TS-890/TS-990 `NB1`/`NB2` cannot be fully
  represented by the current single `noise_blanker` field.
- Yaesu profiles expose keyer speed but not CAT CW send/stop in the CSV, so
  `keyer.send_cw` and `keyer.stop_cw` should be `Unsupported`.
- `kenwood-if232` has `PTT` but no RF power row, so `tx.power` should be
  `Unsupported`.

## Command Completion

The current API methods return `Result<()>`, so the driver must define what
success means.

Recommended semantics:

- Setter returns `Ok(())` after the command is accepted or no protocol error is
  observed before timeout.
- If the command has a confirming query/response path, the driver should prefer
  confirmation before returning.
- If confirmation is unavailable, emit an optimistic state patch after the send
  succeeds and mark the update source as `Optimistic`.
- Poll or async frames later correct the state if the radio disagrees.

`SetPtt(false)` and CW stop commands should use high command priority.

## Transaction Matching

The transaction layer must support unsolicited frames interleaved with command
responses.

Outgoing commands should carry:

```rust
struct OutgoingFrame {
    frame: AsciiFrame,
    expected: ResponseMatcher,
    priority: CommandPriority,
    timeout: Duration,
    retries: u8,
}
```

Examples:

| Command | Expected response |
| --- | --- |
| `FA;` | matching `FA...;` |
| `FA00014074000;` | no response, echo, or matching `FA...;` depending on profile |
| `IF;` | matching `IF...;` |
| `AI2;` | usually no response; unsolicited frames may follow |
| `KY text;` | no immediate response; `KY;` query can confirm buffer |
| `TX...;` | no getter on many profiles; confirm from `IF;` when possible |

If a frame does not match the active transaction, pass it to unsolicited decode.
If unsolicited decode yields state patches, apply them immediately.

## Testing

Testing should be table-driven and profile-specific.

Unit tests:

- frame splitting with partial and multiple frames
- error frame recognition
- frequency formatting for 11-digit and 9-digit profiles
- Kenwood `IF` parser
- Yaesu `IF` parser
- mode encode/decode per profile family
- split encode/decode per profile family
- RIT/XIT offset encode/decode including relative set
- filter lookup/conversion once external tables are available
- RF/DSP leveled setting mapping
- PTT command mapping
- CW chunking and stop command mapping

Integration tests with mock transport:

- startup sends auto-info and explicit query plan
- async frame updates state without a command in flight
- poll frame updates state and emits no event when unchanged
- command response and unsolicited frame interleaving
- `O;` retry behavior
- `?;`, `E;`, and `XX?;` error behavior
- unsupported command returns `RadioError::UnsupportedCapability`

Fixture layout:

```text
tests/fixtures/kenwood_ascii/
  ts590/
    startup.txt
    if_status.txt
    mode_data.txt
  ts890/
    sf_status.txt
  ts990/
    om_status.txt
  elecraft_k4/
    mode_data.txt
  yaesu_ftdx10/
    if_status.txt
    mode.txt
```

## Implementation Phases

1. Add `protocol::kenwood_ascii` frame, error, transaction, and profile
   scaffolding.
2. Add profile metadata for all CSV columns with capabilities and startup plans.
3. Implement shared frequency, error, PTT, and keyer codecs.
4. Implement Kenwood 35-character `IF` and Yaesu 25-character `IF` parsers.
5. Implement mode codecs for Kenwood, TS-890, TS-990, Elecraft, K2, and Yaesu.
6. Implement split and VFO routing.
7. Implement RIT/XIT.
8. Implement RF power and RF/DSP controls.
9. Implement filters after the missing external tables are available.
10. Register public driver IDs and add mock transport tests.

The first end-to-end profile should be TS-590 because it exercises common
Kenwood `IF`, 11-digit frequency, mode plus data-mode composition, RIT/XIT,
PTT, CW, and hi/lo filter conversion. The second should be a Yaesu profile
such as FTDX-10 to validate 9-digit frequency, Yaesu `IF`, signed IF shift, and
mode target syntax. The third should be Elecraft K4 or K3 family to validate
`$` sub-VFO commands and `DT` data-mode composition.

## Decisions Needed

These are the decisions I need from you before implementation details are
finalized:

1. Should this new document stay as
   `docs/KENWOOD_PROTOCOL_IMPLEMENTATION.md`, or should it be merged into
   `docs/radio_cat_rs_kenwood_ascii_driver_design.md`?
2. Where are the missing filter reference files listed in the Source Inputs
   section?
3. Should `Mode` be expanded to cover every mode in the CSV, or should
   uncommon digital modes collapse to `Mode::Digital`?
4. Should receiver filter shift become signed (`i16` or a newtype) to support
   Yaesu IF shift?
5. Should public PTT remain boolean, or should the API expose normal send,
   data send, and tune?
6. Should attenuator/preamp/noise-control `level` values use physical values
   where possible, such as dB, or profile-specific ordinals?
7. How should multiple independent noise blankers/noise reducers be exposed?
8. Should startup send `AI` before the explicit query plan, as recommended
   here, or after the explicit query plan?
9. Should driver selection be manual by explicit profile ID only, or should the
   driver also attempt model auto-detection later?
