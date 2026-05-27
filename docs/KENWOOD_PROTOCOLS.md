# Kenwood-Style CAT Protocol Notes

This document describes the Kenwood-style CAT protocol families represented in `rigs/kenwood/`, with emphasis on the command surface needed for a Rust implementation.

It is source-derived documentation rather than a manufacturer protocol manual. The goal is to capture the practical protocol families, model groupings, mode maps, and command quirks that matter when implementing compatible clients.

## Scope and Legend

The feature tables below focus on these core capabilities:

- Frequency: get/set current frequency.
- Other VFO frequency:
  - `yes`: the protocol can address an alternate VFO/band directly or with dedicated split/current-VFO commands.
  - `partial`: multiple VFOs/bands exist, but targeting is incomplete or model-specific.
  - `no`: effectively single-VFO/current-VFO only.
- Mode: get/set mode.
- Bandwidth:
  - `rw`: explicit read/write passband or filter width support.
  - `ro`: width is returned or implied by the mode family, but there is no independent width-setting command family in scope here.
  - `no`: no meaningful passband handling for this protocol family.
- RIT: explicit get/set RIT offset.
- Keyer: explicit get/set CW keyer speed.
- NR toggle: noise reduction as a function toggle.
- NB toggle: noise blanker as a function toggle. `NB1/NB2` means two independently addressable blankers.
- Morse:
  - `send`: outbound `KY...` only.
  - `send+stop`: both send and stop are supported.

## Feature Summary

| Models | Protocol family | Freq | Other VFO freq | Mode | Bandwidth | RIT | Keyer | NR toggle | NB toggle | Morse | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `TS-140S`, `TS-680S`, `TS-711`, `TS-790`, `TS-811` | classic Kenwood | yes | yes | yes | no | yes | no | no | no | no | Default Kenwood `MD` numbering. Older subset with a simpler control surface. |
| `TS-690S` | classic Kenwood | yes | yes | yes | no | yes | no | no | no | no | Default Kenwood `MD` numbering, more general than the group above. |
| `TS-50S` | classic Kenwood | yes | yes | yes | no | yes | yes | yes | yes | no | Adds `KS` and `NR`/`NB`, but no explicit width control. |
| `TS-930` | classic Kenwood | yes | yes | yes | no | yes | yes | yes | yes | no | HF-only mode set on the default Kenwood numbering. |
| `TS-940S` | classic Kenwood, limited mode map | yes | yes | yes | no | yes | no | no | no | no | Limited mode table: `LSB/USB/CW/FM/AM`. |
| `TS-950S`, `TS-950SDX` | classic Kenwood with `FL` filters | yes | yes | yes | rw | yes | no | no | no | send | Uses generic `MD` plus `FL` width handling. Also has model-specific DATA-mode behavior. |
| `TS-440S` | IC-10-derived mode/VFO family | yes | yes | yes | no | yes | no | no | no | no | Uses an IC-10-style mode and VFO model rather than normal Kenwood `MD`. |
| `R-5000` | IC-10-derived receive-oriented family | yes | yes | yes | no | no | no | no | no | no | Receive-oriented IC-10-style family. No RIT path in scope here. |
| `TS-450S` | classic Kenwood with `FL` filters | yes | yes | yes | rw | yes | no | no | no | no | Default Kenwood table; explicit width via `FL`. |
| `TS-850` | classic Kenwood with `FL` filters | yes | yes | yes | rw | yes | no | no | no | no | Default Kenwood table; explicit width via `FL`. |
| `TS-870S` | extended classic Kenwood | yes | yes | yes | rw | yes | no | yes | yes | send | Custom `MD` + `FW`/`IS` width logic. |
| `TS-570S`, `TS-570D` | TS-570 family | yes | yes | yes | rw | yes | yes | yes | yes | send | Width is `FW` for `CW/CWR/RTTY/RTTYR` and `SL` for `USB/LSB/FM/AM`. |
| `TS-2000` | extended classic Kenwood | yes | yes | yes | no | yes | yes | yes | yes | send | Default Kenwood table. Broad control surface, but no independent passband command family in this document. |
| `SDRConsole` | TS-2000-style emulation subset | yes | yes | yes | no | no | no | no | no | no | TS-2000 emulation with a reduced feature surface. |
| `TS-480` | TS-480 family | yes | yes | yes | rw | yes | yes | yes | yes | send | Default Kenwood table. Explicit width via `FW` mapping tables. |
| `(tr)uSDX` | TS-480-style emulation | yes | yes | yes | rw | yes | yes | yes | yes | send | TS-480-compatible emulation path. |
| `QCX/QDX` | TS-480-style emulation | yes | yes | yes | rw | yes | yes | yes | yes | send | TS-480-compatible emulation path. Also accepts `NR2`. |
| `QMX` | minimal TS-480-style subset | yes | yes | yes | rw | no | no | no | no | no | Minimal QRP Labs subset on top of the TS-480 mode and frequency path. |
| `PT-8000A` | TS-480-style variant | yes | yes | yes | rw | yes | no | yes | yes | no | TS-480 identifier compatibility but a different level/function surface. |
| `SDRUno` | TS-480-style emulation | yes | yes | yes | rw | yes | yes | yes | yes | send | TS-480-compatible with extra `PKTUSB` mapped onto mode code `8`. |
| `DSP` (Malachite) | reduced TS-480-style subset | yes | partial | yes | no | no | no | no | no | no | Frequency and mode only. Effectively VFO A only. |
| `TS-590S`, `TS-590SG`, `FX4/C/CR/L` | TS-590 family | yes | yes | yes | rw | yes | yes | yes | yes | send+stop | Extended Kenwood family with DATA submodes and `SF0`/`SF1` mode snapshots. |
| `TS-890S` | TS-890 family | yes | yes | yes | rw | yes | yes | yes | `NB1/NB2` | send+stop | Uses `SF0;`/`SF1;` for per-VFO mode reads and dedicated `NB1`/`NB2`. |
| `TS-990S` | TS-990 main/sub family | yes | yes | yes | rw | yes | yes | yes | yes | send+stop | Main/sub architecture, custom extended mode map, targeted mode read via `OM0`/`OM1`. |
| `TRC-80` | classic Kenwood variant | yes | partial | yes | no | yes | no | no | yes | no | Classic Kenwood-like HF set with tuner/AIP/TONE/NB/VOX, but not the broader modern feature set. |
| `K2` | Elecraft K2 family | yes | yes | yes | rw | yes | yes | no | yes | send | Custom K2 mode table. Width uses K2 filter-slot commands, not generic Kenwood passband commands. |
| `K3`, `K3S` | Elecraft K3 family | yes | yes | yes | rw | yes | yes | no | yes | send+stop | Uses `BW` plus VFO-B-specific `MD$`/`BW$`. NR is a level, not a toggle. |
| `K4` | Elecraft K4 family | yes | yes | yes | rw | yes | yes | no | yes | send+stop | K3-family semantics with K4-specific stop-morse and VFO query behavior. |
| `KX3`, `KX2` | Elecraft portable family | yes | yes | yes | rw | yes | yes | no | yes | send+stop | Same K3-family passband logic; NR is a level, not a toggle. |
| `XG3` | Elecraft signal-generator profile | yes | no | no | no | no | no | no | no | no | Signal generator profile, not a full transceiver protocol. |
| `6xxx` | Flex 6xxx emulation | yes | yes | yes | rw | yes | yes | no | no | send+stop | Flex SmartSDR emulation using a Flex-specific mode table. |
| `PowerSDR`, `Thetis` | PowerSDR/Thetis emulation | yes | yes | yes | rw | yes | yes | no | yes | send+stop | Mode numbering differs from both classic Kenwood and Flex 6xxx. |
| `PiHPSDR` | Kenwood-style SDR emulation | yes | yes | yes | no | yes | yes | yes | yes | send | Broad function surface but no independent width family. |
| `uSDX` (Hamgeek) | reduced Kenwood-style subset | yes | partial | yes | no | no | no | no | no | no | Uses the default Kenwood mode table, but remaps some requested packet and reverse-CW/data modes before sending. |
| `TX-500` | Kenwood-style transceiver subset | yes | yes | yes | no | yes | yes | yes | yes | send | Broad function surface, but no independent passband command family in this document. |
| `Transfox` | USB-only SDR profile | yes | no | no | no | no | no | no | no | no | USB-only SDR profile. |

## Shared Protocol Structure

### Command framing

- All rigs covered by this document use `;` as the command terminator.
- Commands are ASCII, typically two uppercase letters plus fixed-width decimal payloads.
- Replies are also ASCII and are usually parsed positionally.

For a Rust implementation, the first protocol discriminator should be:

1. mode-numbering family
2. VFO-addressing strategy
3. passband strategy

### Frequency control

The generic Kenwood family uses:

- `FA###########` for VFO A or main frequency
- `FB###########` for VFO B or sub frequency
- `FC###########` for VFO C where supported

Frequency is generally read with `FA`, `FB`, `FC`, or by falling back to `IF` for memory or current-VFO cases.

Important special cases:

- `TS-590S` has a firmware workaround when setting the non-current split VFO.
- `Malachite DSP` is effectively VFO-A-only even though it sits in a TS-480-style family.

### VFO and split handling

Common patterns:

- Classic Kenwood: a current-VFO selection command changes the active VFO, but `FA` and `FB` can still target A and B directly on many rigs.
- Split enable is usually `FT...`, `SP...`, or model-specific logic around the current TX VFO.
- `TS-990S` is special:
  - mode reads are targeted with `OM0` for main and `OM1` for sub
  - mode writes still require current-VFO switching internally
- `TS-890S` is special:
  - per-VFO mode snapshots come from `SF0;` and `SF1;`
- Elecraft K3/K4 family is special:
  - VFO-B mode, bandwidth, and data-mode use `$` suffixed commands such as `MD$`, `BW$`, `DT$`

## Mode Enumeration Families

These are the meaningful mode-code families in the directory. Make the mode map part of the per-model descriptor.

### 1. Default Kenwood mode table

Used by most classic Kenwood families and many emulations.

| Code | Mode |
| --- | --- |
| `0` | none |
| `1` | `LSB` |
| `2` | `USB` |
| `3` | `CW` |
| `4` | `FM` |
| `5` | `AM` |
| `6` | `RTTY` |
| `7` | `CWR` |
| `8` | none, tune, or model-specific override |
| `9` | `RTTYR` |
| `A` | `PSK` |
| `B` | `PSKR` |
| `C` | `PKTLSB` |
| `D` | `PKTUSB` |
| `E` | `PKTFM` |
| `F` | `PKTAM` |
| `G` | `LSBD2` |
| `H` | `USBD2` |
| `K` | `LSBD3` |
| `L` | `USBD3` |

Notes:

- `SDRUno` overrides code `8` to mean `PKTUSB`.
- `TS-590S` and `TS-590SG`, plus `TS-950S` and `TS-950SDX`, layer DATA-mode state on top of this numbering.
- `Hamgeek uSDX` accepts the default table but remaps some requested packet and reverse data modes before sending.

### 2. TS-940S limited table

Used by `TS-940S`.

| Code | Mode |
| --- | --- |
| `1` | `LSB` |
| `2` | `USB` |
| `3` | `CW` |
| `4` | `FM` |
| `5` | `AM` |

Everything else is effectively unsupported.

### 3. K2 table

Used by `K2`.

| Code | Mode |
| --- | --- |
| `1` | `LSB` |
| `2` | `USB` |
| `3` | `CW` |
| `6` | `PKTLSB` |
| `7` | `CWR` |
| `9` | `PKTUSB` |

K2 bandwidth is not driven by generic Kenwood passband commands. Implementations enter K2 filter setup mode and program filter slots explicitly.

### 4. K3 / K4 family

Used by `K3`, `K3S`, `K4`, `KX3`, `KX2`.

- Base mode numbering is close to default Kenwood.
- Data submodes are not represented by a simple static table alone.
- Practical implementations use `DT` or `DT$` plus `MD` or `MD$` to distinguish:
  - `PKTUSB`
  - `PKTLSB`
  - `RTTY`
  - `RTTYR`
  - `PSK`

Bandwidth is read and written with `BW` or `BW$`.

### 5. TS-990S extended table

Used by `TS-990S`.

| Code | Mode |
| --- | --- |
| `1` | `LSB` |
| `2` | `USB` |
| `3` | `CW` |
| `4` | `FM` |
| `5` | `AM` |
| `6` | `RTTY` |
| `7` | `CWR` |
| `9` | `RTTYR` |
| `C` | `LSBD1` |
| `D` | `USBD1` |
| `E` | `PKTFM` |
| `G` | `LSBD2` |
| `H` | `USBD2` |
| `I` | `PKTFM` |
| `K` | `LSBD3` |
| `L` | `USBD3` |
| `M` | `PKTFM` |

This is the biggest mode-map deviation from the standard Kenwood table.

### 6. Flex 6xxx table

Used by `6xxx`.

| Code | Mode |
| --- | --- |
| `1` | `LSB` |
| `2` | `USB` |
| `3` | `CW` |
| `4` | `FM` |
| `5` | `AM` |
| `6` | `PKTLSB` |
| `9` | `PKTUSB` |

### 7. PowerSDR / Thetis table

Used by `PowerSDR` and `Thetis`.

| Code | Mode |
| --- | --- |
| `0` | `LSB` |
| `1` | `USB` |
| `2` | `DSB` |
| `3` | `CWR` |
| `4` | `CW` |
| `5` | `FM` |
| `6` | `AM` |
| `7` | `PKTUSB` |
| `9` | `PKTLSB` |
| `10` | `SAM` |

### 8. IC-10-derived family

Used by `TS-440S` and `R-5000`.

This family does not use the normal Kenwood `MD` table. Supported modes are:

- `CW`
- `USB`
- `LSB`
- `FM`
- `AM`
- `RTTY`

## Core Command Notes

### Get/set mode and bandwidth

There is no single bandwidth strategy across this directory.

- Generic classic Kenwood:
  - mode is `MDx`
  - width may be absent, inferred, or model-specific
- `TS-450S`, `TS-850`, `TS-950S`, `TS-950SDX`:
  - mode is generic `MDx`
  - bandwidth uses `FL`
- `TS-480` family:
  - mode is generic `MDx`
  - bandwidth is read and written with `FW` through model-specific width tables
- `TS-570S` and `TS-570D`:
  - width is `FW` for `CW/CWR/RTTY/RTTYR`
  - width is `SL` for `USB/LSB/FM/AM`
- `TS-590S`, `TS-590SG`, and `FX4`:
  - mode can be read from `SF0` and `SF1`
  - `FW`, `SL`, and `SH` combine to describe passband
  - DATA submodes are managed separately
- `TS-870S`:
  - width uses `FW`
  - for SSB and AM, `IS` participates in effective passband calculation
- `TS-890S`:
  - targeted mode read comes from `SF0;` and `SF1;`
  - width is part of the newer Kenwood extended command set
- `K3` family:
  - width uses `BW` and `BW$`
- `K2`:
  - width is controlled through K2 filter-slot commands rather than a generic Kenwood width command

### RIT

There are two major RIT styles.

Classic RIT:

- enable or disable via `RT0` and `RT1`
- read via positional fields in `IF`
- set by stepping with `RU` and `RD`
- some rigs support the newer absolute-delta form `RUxxxxx` and `RDxxxxx`

Newer dedicated-RIT rigs:

- read via `RF`
- write via `RUxxxxx;` and `RDxxxxx;`

The dedicated path appears in:

- `TS-890S`
- `TS-990S`

Custom wrappers around the older scheme appear in:

- `TS-480`
- `TS-590S`
- `TS-2000`
- `TS-570`
- `TS-850`
- Elecraft K3 family

### Keyer speed

Where present, keyer speed uses:

- `KSnnn` to set
- `KS` to get

Models with explicit keyer-speed support in this directory include:

- `TS-50S`
- `TS-570S` and `TS-570D`
- `TS-590S`, `TS-590SG`, `FX4`
- `TS-890S`
- `TS-990S`
- `TS-2000`
- `TS-480`, `(tr)uSDX`, `QCX/QDX`, `SDRUno`
- `K2`
- `K3`, `K3S`, `K4`, `KX3`, `KX2`
- `PiHPSDR`
- `TX-500`
- `6xxx`, `PowerSDR`, `Thetis`

### Noise reduction and noise blanker

Generic function commands:

- `NR0` and `NR1` for simple NR off and on
- some rigs also accept `NR2`
- `NB0` and `NB1` for simple NB off and on

Important exceptions:

- `TS-890S` has two blankers:
  - `NB10` and `NB11`
  - `NB20` and `NB21`
- `K3` family treats NR as a level rather than a boolean toggle
- `Flex 6xxx` also does not expose a simple Kenwood-style NR toggle in the same way

`NR2` is accepted for:

- `TS-890S`
- `TS-590S`
- `TS-590SG`
- `TS-480`
- `TS-2000`
- `QCX/QDX`

### Morse send/stop

Baseline transmit command is `KY`.

Long messages typically need to be fragmented into chunks, and formatting varies by model:

- Classic or default form:
  - `KY <left-padded or space-padded 24 character payload>`
- `K3`, `K3S`, `KX2`, `KX3`, `QCX/QDX`:
  - `KY <message>`
- `TS-590S`:
  - right-aligned 24-character payload
- `TS-890S`:
  - `KY2<message>`
- `TS-990S`:
  - `KY2<message>` on newer firmware, otherwise falls back to padded classic behavior

Stop behavior also differs:

- generic stop: `KY0`
- `K3` family stop: `KY <0x04>;`
- `K4` stop: `KY @;`

For a Rust design, treat morse sending as its own strategy object:

- buffer-availability probe
- payload formatter
- stop command formatter

## Rust Implementation Guidance

A practical model descriptor should contain at least:

- frequency strategy
- VFO addressing strategy
- mode map
- bandwidth strategy
- RIT strategy
- function map for NR and NB
- morse formatter and stop strategy

The minimum useful protocol families for this directory are:

1. classic Kenwood family
2. TS-480 width family
3. TS-570 width family
4. TS-590, TS-890, and TS-990 extended family
5. Elecraft K2
6. Elecraft K3 and K4 family
7. IC-10-derived family
8. Flex and PowerSDR family

That decomposition matches the real protocol-family boundaries in these sources much better than a per-model rewrite.
