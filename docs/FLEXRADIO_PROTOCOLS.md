# FlexRadio Protocol Notes

This document describes the FlexRadio-compatible SmartSDR control implementation under `rigs/flexradio/`, with emphasis on the command surface that matters when building a Rust client.

It is source-derived documentation rather than a manufacturer protocol manual. This document intentionally covers only the SmartSDR slice-control family and excludes the non-CAT-style `DttSP` and `SDR-1000` backends.

## Scope and Legend

Included models:

- `SmartSDR Slice A`
- `SmartSDR Slice B`
- `SmartSDR Slice C`
- `SmartSDR Slice D`
- `SmartSDR Slice E`
- `SmartSDR Slice F`
- `SmartSDR Slice G`
- `SmartSDR Slice H`

The feature matrix focuses on these core capabilities:

- `Freq`: get/set current frequency.
- `Other VFO freq`:
  - `yes`: a distinct alternate side can be targeted directly.
  - `slice-based`: the protocol can control multiple slices, but each profile in this directory binds to exactly one slice.
  - `no`: effectively one active tuning target only.
- `Mode`: get/set mode.
- `Bandwidth`:
  - `numeric`: explicit numeric passband/filter limits.
  - `filter-pair`: low/high filter edges are set as a pair.
  - `fixed`: no useful variable-width control in scope here.
- `RIT`: whether an actual set/get RIT path is implemented in this source tree.
- `Morse`:
  - `send+stop`: arbitrary outbound text plus explicit stop/clear.
  - `no`: no arbitrary-text CW send path.

## Feature Summary

| Models | Protocol family | Transport | Terminator / framing | Freq | Other VFO freq | Mode | Bandwidth | RIT | Keyer speed | NR | NB | Morse | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `SmartSDR Slice A` through `SmartSDR Slice H` | SmartSDR slice control | TCP | newline-delimited ASCII inside `C<seq>|...` command envelope | yes | slice-based | yes | numeric | no usable set/get path here | no | no exposed toggle here | no exposed toggle here | `send+stop` | Each model is one fixed slice target, not a generic multi-slice controller. |

## SmartSDR Family

### Included models

- `SmartSDR Slice A`
- `SmartSDR Slice B`
- `SmartSDR Slice C`
- `SmartSDR Slice D`
- `SmartSDR Slice E`
- `SmartSDR Slice F`
- `SmartSDR Slice G`
- `SmartSDR Slice H`

These eight models are the same protocol with a different fixed slice number:

| Model | Slice number |
| --- | --- |
| `SmartSDR Slice A` | `0` |
| `SmartSDR Slice B` | `1` |
| `SmartSDR Slice C` | `2` |
| `SmartSDR Slice D` | `3` |
| `SmartSDR Slice E` | `4` |
| `SmartSDR Slice F` | `5` |
| `SmartSDR Slice G` | `6` |
| `SmartSDR Slice H` | `7` |

### Transport and framing

- Default endpoint in this source tree: `127.0.0.1:4992`.
- Commands are ASCII and newline-terminated.
- Outbound commands are wrapped as:

```text
C<seq>|<command>\n
```

Examples:

```text
C0|sub slice 0\n
C1|slice tune 0 14.074000 autopan=1\n
C2|slice set 0 mode=DIGU\n
```

- Replies are not simple request/response records. The code consumes an asynchronous mixed stream.
- The parser recognizes:
  - `S...` status/event packets
  - `R<number>|...|` result packets

Practical result handling:

- a nonzero `R<number>` is treated as an error
- status packets are important because they refresh cached state such as frequency, mode, width, and TX ownership

### Session bootstrap

On open, the client subscribes to one slice:

```text
sub slice <slice>
```

The implementation then waits for status messages until it has initial state, especially frequency.

### Frequency

Set current frequency:

```text
slice tune <slice> <MHz_float> autopan=1
```

Example:

```text
slice tune 0 10.137000 autopan=1
```

Read behavior in this implementation:

- there is no dedicated synchronous "get frequency" command in the code path
- the client flushes pending messages and reads the most recent subscribed status state
- `RF_frequency=...` in status packets is the authoritative field

### Mode

Set mode:

```text
slice set <slice> mode=<mode>
```

Mode mappings used here:

| SmartSDR token | Meaning |
| --- | --- |
| `LSB` | `LSB` |
| `USB` | `USB` |
| `CW` | `CW` |
| `AM` | `AM` |
| `FM` | `FM` |
| `FMN` | `FMN` according to this implementation |
| `DIGL` | `PKTLSB` |
| `DIGU` | `PKTUSB` |
| `SAM` | `SAM` |
| `RTTY` | recognized on receive/status parse only |

Important mode note:

- The status example in the source advertises `NFM` in `mode_list`, while the parser/setter uses `FMN`.
- For a Rust implementation, treat narrow FM naming as a compatibility edge case and make the string table configurable.

### Bandwidth / filter width

This implementation treats bandwidth as numeric width derived from status:

- status parse reads `filter_hi=<value>`
- `get_mode` returns that numeric width

When setting a non-default width, the code formats:

```text
filt <slice> 0 <width>
```

However, this source path does not actually transmit that command after formatting it. So the documented protocol shape is present, but the implementation here only fully wires mode-setting, not explicit width-setting.

For a Rust client, model SmartSDR bandwidth as a numeric per-slice filter command family rather than a Yaesu/Icom-style preset table.

### PTT and TX ownership

PTT is not purely local to the subscribed slice. The code tracks both:

- `state=TRANSMITTING`
- `tx=<0|1>`

When enabling PTT, it first claims TX on the selected slice, then starts transmit:

```text
slice set <slice> tx=1
xmit 1
```

When disabling:

```text
xmit 0
```

If another slice already owns TX, the implementation refuses to key this one.

### Morse send / stop

This family supports arbitrary outbound text, not just stored messages.

Send arbitrary text:

```text
cwx send "<text>"
```

Implementation detail:

- spaces are rewritten to `0x7f` before transmission
- the outgoing command still uses a quoted string payload

Stop/clear current CW send:

```text
cwx clear
```

This is the only FlexRadio family in this directory with an arbitrary-text morse path.

### Status packet fields worth parsing

An example `sub slice` status line in the source includes fields such as:

- `RF_frequency=...`
- `mode=...`
- `filter_lo=...`
- `filter_hi=...`
- `rit_on=...`
- `rit_freq=...`
- `xit_on=...`
- `xit_freq=...`
- `nr=...`
- `nb=...`
- `tx=...`
- `active=...`
- `mode_list=...`

Important limitation:

- this source only turns a small subset of those fields into callable operations
- RIT, XIT, NR, and NB appear in status traffic, but no set/get command wrappers for them are exposed here

### Rust modeling guidance

A useful SmartSDR descriptor shape is:

```text
protocol_family = SmartSdr
transport = tcp
default_addr = "127.0.0.1:4992"
terminator = "\n"
command_envelope = "C{seq}|{body}\n"
status_prefixes = ["S", "R"]
addressing = fixed_slice
slice_number = 0..7
mode_map = string_based
morse_send = arbitrary_text
morse_stop = clear_command
```

Do not model these eight profiles as different protocol dialects. They are one dialect plus a fixed slice index.

## Practical Recommendations

For a clean implementation, model SmartSDR as:

1. a newline-delimited asynchronous TCP protocol
2. with sequenced outbound commands
3. and mixed status/result inbound packets
4. where one session is bound to one slice

The main trait surface can stay narrow:

- current frequency
- current mode
- PTT
- arbitrary-text morse send/stop

Do not assume support here for:

- alternate VFO targeting in the traditional A/B sense
- RIT set/get commands
- explicit NR/NB command wrappers from this source tree
