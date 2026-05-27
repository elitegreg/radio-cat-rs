# Yaesu New-CAT Protocol Notes

This document describes the Yaesu-compatible rigs under `rigs/yaesu/` that use the newer text CAT protocol implemented by the shared `newcat` layer.

It is source-derived documentation rather than a manufacturer protocol manual. The goal is to capture the protocol families, command forms, mode maps, and model-specific quirks that matter when implementing a compatible Rust client.

## Scope and Legend

Included models:

- `FT-450`
- `FT-950`
- `FT-2000`
- `FTDX-1200`
- `FTDX-3000`
- `FTDX-5000`
- `FTDX-9000`
- `FTDX-9000 Old`
- `FT-991`
- `FT-891`
- `FT-710`
- `FTDX-10`
- `FTDX-101D`
- `FTDX-101MP`

The feature matrix focuses on these core capabilities:

- `Freq`: get/set current frequency.
- `Other VFO freq`:
  - `yes`: the protocol can address the alternate side directly.
  - `partial`: the alternate side exists, but targeting requires model-specific switching or split logic.
- `Mode`: get/set mode.
- `Bandwidth`:
  - `indexed`: explicit width selection via `SH...` plus mode-family width tables.
  - `coarse`: width is supported, but the implementation falls back to coarse or heuristic mappings.
  - `mixed`: explicit width exists, but some mode families are fixed-width or special-cased.
- `RIT/XIT`:
  - `rit/xit`: normal RIT/XIT command family.
  - `clarifier`: implemented through the clarifier-frequency command family instead.
- `NB`:
  - `NB`: one blanker toggle.
  - `NB+NB2`: two independently addressable blankers.
- `Morse`:
  - `send`: arbitrary outbound morse text is supported. No stop command is exposed for the covered rigs.

## Feature Summary

| Models | Protocol family | Freq | Other VFO freq | Mode | Bandwidth | RIT/XIT | Keyer speed | NR | NB | Morse | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `FT-450` | early New-CAT | yes | partial | yes | coarse | rit/xit | yes | yes | `NB` | send | Alternate-VFO access exists, but frequency targeting is not cleanly direct; current-VFO switching is part of normal operation. |
| `FT-950` | early New-CAT with roofing | yes | yes | yes | indexed | rit/xit | yes | yes | `NB+NB2` | send | Adds explicit roofing-filter selection and width tables. |
| `FT-2000` | dual-VFO New-CAT with broader targeting | yes | yes | yes | coarse | rit/xit | yes | yes | `NB+NB2` | send | Width support exists, but the source comments note missing authoritative width details and uses coarse mappings. |
| `FTDX-1200`, `FTDX-3000` | mid-generation indexed-width family | yes | yes | yes | indexed | rit/xit | yes | yes | `NB+NB2` | send | Shared CW/SSB width tables. Roofing filter selection tied to width choice. |
| `FTDX-5000` | expanded indexed-width family | yes | yes | yes | indexed | rit/xit | yes | yes | `NB+NB2` | send | Similar to `1200/3000`, but with a larger roofing-filter set. |
| `FTDX-9000`, `FTDX-9000 Old` | early flagship New-CAT | yes | yes | yes | coarse | rit/xit | yes | yes | `NB+NB2` | send | Width support is present, but the source uses the same coarse fallback strategy as `FT-450`. |
| `FT-991` | hybrid HF/VHF/UHF family | yes | yes | yes | mixed | rit/xit | yes | yes | `NB` | send | Adds `AMN`, `FMN`, and `C4FM`. C4FM width handling is special-cased. |
| `FT-891` | compact HF/6 m family | yes | partial | yes | indexed | rit/xit | yes | yes | `NB` | send | Split/VFO behavior is more specialized than the generic New-CAT path. |
| `FT-710` | modern indexed-width family | yes | yes | yes | indexed | clarifier | yes | yes | `NB` | send | Uses clarifier-frequency commands for both RIT and XIT. Shares width tables with `FTDX-101`. |
| `FTDX-10` | modern indexed-width + roofing family | yes | yes | yes | indexed | rit/xit | yes | yes | `NB` | send | Shares most width semantics with `FTDX-101`, but roofing options differ. |
| `FTDX-101D`, `FTDX-101MP` | modern flagship family | yes | yes | yes | indexed | rit/xit | yes | yes | `NB` | send | Main/sub targeted mode and roofing control. `101MP` has optional extra roofing filters. |

## Shared Protocol Structure

### Command framing

- All covered rigs use `;` as the command terminator.
- Commands and replies are ASCII.
- Most commands use a two-letter mnemonic followed by fixed-width decimal fields.
- Replies are parsed positionally rather than as delimited records.

### Error and retry behavior

- `?;` is not a clean error discriminator. In practice it can mean:
  - command rejected
  - rig busy
  - state-dependent refusal
  - general communication failure
- A robust client should treat `?;` as retryable first, then escalate to a real failure if the command is known to be valid but repeatedly rejected.
- Some commands are effectively verified by follow-up reads such as `AI;` or `ID;` rather than by a distinct ACK packet.

## Core Command Families

### Frequency

The shared frequency commands are:

- `FA###########;` for VFO A or main
- `FB###########;` for VFO B or sub

Important model-specific behavior:

- `FT-450` is the main outlier. It behaves more like a current-VFO rig and may require explicit VFO switching before `FA` or `FB` has the intended effect.
- `FT-991` uses extra band-selection logic when changing bands on VFO A. In practice the code path may issue `BS..;` before `FA...;`.
- `FTDX-1200`, `FTDX-3000`, and `FTDX-5000` may reject frequency changes while transmitting; retry logic matters.
- `FTDX-101D` and `FTDX-101MP` impose stricter VFO restrictions while transmitting.

### VFO and split

The protocol is fundamentally VFO-oriented rather than split-frequency-oriented.

- Split is represented by which side is TX and whether split is enabled.
- A dedicated "set split TX frequency without touching the TX VFO state" path is not generally available.
- The generic split-frequency helpers are effectively unavailable; a Rust implementation should model split as:
  - current RX VFO
  - current TX VFO
  - split on/off

Model-specific VFO notes:

- `FT-450` relies more heavily on explicit VFO switching.
- `FT-891` uses custom split-VFO logic rather than the plain generic path.
- `FT-991` has custom VFO get/set handling.
- `FTDX-101D` and `FTDX-101MP` support targeted mode and roofing operations on main/sub, but TX-time restrictions still apply.

### Mode

The shared mode command family is `MD`.

Common forms:

- current-side or simple-target rigs: `MDm;`
- main/sub-targeted rigs: `MD0m;` for main, `MD1m;` for sub

Mode reads follow the same pattern:

- `MD;`
- `MD0;`
- `MD1;`

Where `m` is a single mode code from the shared New-CAT mode table.

## Mode Enumeration

The shared New-CAT mode table is:

| Code | Mode |
| --- | --- |
| `1` | `LSB` |
| `2` | `USB` |
| `3` | `CW` |
| `4` | `FM` |
| `5` | `AM` |
| `6` | `RTTY` |
| `7` | `CWR` |
| `8` | `PKTLSB` |
| `9` | `RTTYR` |
| `A` | `PKTFM` |
| `B` | `FMN` |
| `C` | `PKTUSB` |
| `D` | `AMN` |
| `E` | `C4FM` |
| `F` | `PKTFMN` |

Not every model exposes every code meaningfully. The practical families are:

### 1. Early New-CAT set

Used by `FT-450`, `FT-950`, `FT-2000`, `FTDX-9000`, `FTDX-9000 Old`.

Commonly available:

- `LSB`
- `USB`
- `CW`
- `FM`
- `AM`
- `RTTY`
- `CWR`
- `PKTLSB`
- `RTTYR`
- `PKTFM`
- `PKTUSB`

Not part of the practical model surface here:

- `AMN`
- `FMN`
- `C4FM`
- `PKTFMN`

### 2. Mid-generation analog narrow-mode set

Used by `FTDX-1200`, `FTDX-3000`, `FTDX-5000`, `FT-891`.

Adds:

- `FMN`

Still not part of the practical model surface:

- `AMN`
- `C4FM`
- `PKTFMN`

### 3. `FT-991` hybrid set

Used by `FT-991`.

Adds:

- `AMN`
- `FMN`
- `C4FM`

Does not expose `PKTFMN` as part of the documented practical mode surface.

### 4. Modern flagship-style set

Used by `FT-710`, `FTDX-10`, `FTDX-101D`, `FTDX-101MP`.

Adds:

- `AMN`
- `FMN`
- `PKTFMN`

Notably, these models do not use `C4FM` as part of the practical mode set captured by this code.

## Bandwidth and Filter Strategy

Bandwidth handling is the main place where the protocol families diverge.

The shared primitives are:

- `NA...;` for narrow on/off state
- `SH...;` for width index selection
- `RF...;` for roofing filter selection on rigs that support it

### `NA` narrow command

The narrow command is VFO-sensitive on rigs with main/sub targeting:

- set: `NAx0;` or `NAx1;`
- get: `NAx;`

Where `x` is usually:

- `0` for main
- `1` for sub

This command is especially important for:

- AM vs `AMN`
- FM vs `FMN`
- `PKTFM` vs `PKTFMN`
- some older width families where narrow state partly determines the effective passband

### `SH` width-index command families

The width-setting command form is not uniform:

| Models | Set form | Notes |
| --- | --- | --- |
| `FT-950`, `FTDX-1200`, `FTDX-5000`, `FTDX-9000`, `FTDX-9000 Old`, most older rigs | `SHvww;` | `v` is target side where supported, `ww` is width index. |
| `FT-2000`, `FTDX-3000` | `SH0ww;` | Fixed leading `0` form. |
| `FTDX-10`, `FT-710` | `SH00ww;` | Longer fixed prefix form. |
| `FTDX-101D`, `FTDX-101MP` | `SHv0ww;` | Includes both target side and an enable field. |
| `FT-891` | `SHv1ww;` | Similar to `FTDX-101`, but the enable field is forced on. |

Width-read handling also varies by family. A Rust implementation should keep the `SH` formatter/parser in the per-family descriptor rather than hardcoding one global form.

### Width-table families

#### 1. Coarse fallback family

Used by `FT-450`, `FTDX-9000`, `FTDX-9000 Old`.

- SSB-like modes are effectively mapped into coarse buckets around:
  - `1800`
  - `2400`
  - `3000`
- CW/RTTY-like modes are mapped into coarse buckets around:
  - `500`
  - `1800`
  - `2400`
- AM/FM-like widths are mostly narrow-vs-normal decisions.

This family should be treated as supported but not authoritative. The source explicitly falls back to coarse mappings because detailed width behavior is not fully described there.

#### 2. `FT-950` family

Used by `FT-950`.

- CW table includes narrow defaults around `300`/`500` and extends to `2400`.
- SSB table runs from `200` through `3000`.
- Roofing filter selection is tied to requested width.

#### 3. `FT-2000` family

Used by `FT-2000`.

- Has separate CW, SSB, and RTTY tables.
- The implementation notes that authoritative width details are incomplete.
- Practical behavior is:
  - narrow mode is forced off for several DSP-width paths
  - width requests are quantized into a few important breakpoints
  - roofing filters still track requested width

Treat this as its own width family rather than assuming it behaves like `FT-950`.

#### 4. `FTDX-1200` / `FTDX-3000` family

Used by `FTDX-1200`, `FTDX-3000`.

- Shared CW table: `50` through `2400`
- Shared SSB table: `200` through `4000`
- Roofing filters:
  - `15 kHz`
  - `6 kHz`
  - `3 kHz`
  - `600 Hz`
  - `300 Hz`
- AM/FM-like modes still depend partly on narrow state.

#### 5. `FTDX-5000` family

Used by `FTDX-5000`.

- Similar to `1200/3000`, but with its own SSB table.
- Larger roofing-filter set than `FT-950`.
- Supports `600 Hz` and `300 Hz` roofing choices in addition to the wider filters.

#### 6. `FT-991` / `FT-891` family

Used by `FT-991`, `FT-891`.

- Shared CW table up to `3000`.
- Shared SSB table up to `3200`.
- `FT-991` special cases:
  - `AM` is effectively `9000`
  - `AMN` is effectively `6000`
  - `FM` is effectively `16000`
  - `FMN` is effectively `9000`
  - `C4FM` is special-cased because querying `NA0` while in `C4FM` is unsafe
- `FT-891` has no `C4FM`, but otherwise follows the same indexed-width style.

#### 7. `FTDX-101` modern family

Used by `FT-710`, `FTDX-10`, `FTDX-101D`, `FTDX-101MP`.

- Shared CW table extends to `4000`.
- Shared SSB table extends to `4000`.
- `FTDX-101D` and `FTDX-101MP` use full targeted roofing control.
- `FTDX-10` uses a reduced roofing set.
- `FT-710` shares the width tables but has no roofing-filter family in this implementation.

### Roofing filter command

On rigs that support roofing filters, the command family is:

- set: `RFvx;`
- get: `RFv;`

Where:

- `v` is main/sub target on rigs with targeted roofing support
- `x` is the model-specific roofing selector

Important practical rule:

- width selection and roofing selection are coupled
- the source chooses roofing filters automatically from requested bandwidth

If you want a faithful compatible client, model roofing filters as part of the passband family rather than as a completely separate abstraction.

## RIT, XIT, and Clarifier

### Most rigs

The normal RIT/XIT family is:

- clear RIT: `RC;`
- positive RIT: `RC;RUdddd;`
- negative RIT: `RC;RDdddd;`

RIT reads are commonly derived from:

- `IF;` for main
- `OI;` for sub

This applies to all covered rigs except `FT-710`.

### `FT-710`

`FT-710` uses the clarifier-frequency command family instead:

- set: `CF...;`
- get: `CF...;`

The same clarifier-frequency path is used for both RIT and XIT in the covered implementation.

For a Rust design, `FT-710` should have its own `rit_family = clarifier_frequency`.

## Keyer, NR, NB, and Morse

### Keyer speed

The shared CW keyer speed command family is:

- set: `KSnnn;`
- get: `KS;`

This is available across the covered New-CAT models.

### Noise reduction

- NR is exposed on all covered models.
- There is both a toggle concept and a numeric level concept in the protocol surface.
- A practical implementation should separate:
  - `nr_enabled`
  - `nr_level`

### Noise blanker

Two distinct families matter:

- `FT-950`, `FT-2000`, `FTDX-1200`, `FTDX-3000`, `FTDX-5000`, `FTDX-9000`, `FTDX-9000 Old`:
  - support `NB` and `NB2`
- `FT-450`, `FT-991`, `FT-891`, `FT-710`, `FTDX-10`, `FTDX-101D`, `FTDX-101MP`:
  - support one `NB` toggle only

### Morse send

The shared morse-send family is:

- load free-form text into a morse memory slot: `KM1text...;`
- key the loaded message or another stored slot: `KYn;`

For the rigs covered here:

- arbitrary text send is available
- an explicit "stop current morse send" command is not part of the exposed protocol surface

Practical note:

- for this document, "supports morse send" means it can send an arbitrary caller-provided string
- stored-message playback by itself would not count
- the covered New-CAT rigs do qualify, because the text path is `KM1...;` followed by `KY6;`

## APF and Contour Families

These are not part of the minimal CAT surface, but they matter if the Rust implementation wants feature parity with the better-supported rigs.

### APF enable and frequency

The APF control family uses `CO...;`, but the subcommand layout differs:

| Models | APF on/off | APF frequency read/write |
| --- | --- | --- |
| `FTDX-101D`, `FTDX-101MP` | `COv2...` | `COv3...` |
| `FTDX-10`, `FT-991`, `FT-891`, `FT-710` | `CO02...` | `CO03...` |
| `FTDX-3000`, `FTDX-1200` | older `CO` subset | `CO02...` |
| `FTDX-5000`, `FT-2000` | different older `CO` subset | model-specific |

### APF width

The APF-width family uses `EX...;`, grouped as:

| Models | APF width command |
| --- | --- |
| `FTDX-101D`, `FTDX-101MP`, `FTDX-10` | `EX030201...;` |
| `FT-710` | `EX030204...;` |
| `FT-991` | `EX111...;` |
| `FT-891` | `EX1201...;` |
| `FTDX-5000` | `EX112...;` |
| `FTDX-3000`, `FTDX-1200` | `EX107...;` |

### Contour

Contour uses both `CO...;` and `EX...;`, with family splits:

| Models | Contour on/off | Contour freq | Contour level | Contour width |
| --- | --- | --- | --- | --- |
| `FTDX-101D`, `FTDX-101MP`, `FTDX-10` | `CO...` modern | `CO...` modern | `EX030202...;` | `EX030203...;` |
| `FT-710` | `CO...` modern-like | `CO...` modern-like | `EX030205...;` | `EX030206...;` |
| `FT-991` | `CO00...` | `CO01...` | `EX112...;` | `EX113...;` |
| `FT-891` | `CO00...` | `CO01...` | `EX1202...;` | `EX1203...;` |
| `FTDX-5000` | `CO...` older targeted | `CO...` older targeted | `EX113...;` | `EX114...;` |
| `FTDX-3000`, `FTDX-1200` | `CO00...` | `CO01...` | `EX108...;` | `EX109...;` |
| `FT-2000` | `CO00...` | `CO01...` | not in this family table | not in this family table |

The exact `CO` formatting differs enough that this should be its own per-family formatter in Rust rather than a single shared implementation.

## Other Useful Commands

- Repeater shift: `OS...;`
- Repeater offset: `EX...;`
  - the exact `EX` selector depends on both model and band
  - `FT-991` is the broadest here, covering `10 m`, `6 m`, `2 m`, and `70 cm`
- Voice memory playback: `PB...;`
- Clock:
  - date: `DT0...;`
  - time: `DT1...;`
  - UTC offset: `DT2...;`

## Recommended Rust Descriptor Shape

The cleanest implementation strategy is to make the model descriptor carry protocol-family selectors rather than hardcoding by model name all over the command layer.

Suggested descriptor fields:

- `mode_family`
- `freq_family`
- `vfo_family`
- `split_family`
- `width_family`
- `roofing_family`
- `rit_family`
- `apf_family`
- `contour_family`
- `nb_family`

Practical family values from this directory would look like:

| Models | Suggested family selectors |
| --- | --- |
| `FT-450` | `freq=ft450`, `width=coarse_early`, `rit=standard`, `nb=single` |
| `FT-950` | `freq=generic`, `width=ft950`, `roofing=ft950`, `rit=standard`, `nb=dual` |
| `FT-2000` | `freq=generic`, `width=ft2000`, `roofing=ft950`, `rit=standard`, `nb=dual` |
| `FTDX-1200`, `FTDX-3000` | `freq=generic`, `width=ftdx1200`, `roofing=ftdx1200`, `contour=ftdx1200`, `nb=dual` |
| `FTDX-5000` | `freq=generic`, `width=ftdx5000`, `roofing=ftdx5000`, `contour=ftdx5000`, `nb=dual` |
| `FTDX-9000`, `FTDX-9000 Old` | `freq=generic`, `width=coarse_early`, `rit=standard`, `nb=dual` |
| `FT-991` | `freq=ft991`, `width=ft991`, `mode=ft991`, `contour=ft991`, `nb=single` |
| `FT-891` | `freq=ft891`, `split=ft891`, `width=ft991`, `mode=ft891`, `contour=ft891`, `nb=single` |
| `FT-710` | `freq=generic`, `width=ftdx101`, `rit=clarifier`, `apf=ft710`, `contour=ft710`, `nb=single` |
| `FTDX-10` | `freq=generic`, `width=ftdx101`, `roofing=ftdx10`, `apf=ftdx101`, `contour=ftdx101`, `nb=single` |
| `FTDX-101D`, `FTDX-101MP` | `freq=generic`, `width=ftdx101`, `roofing=ftdx101`, `apf=ftdx101`, `contour=ftdx101`, `nb=single` |

This keeps the command layer mostly table-driven while still acknowledging the real protocol splits present in the source.
