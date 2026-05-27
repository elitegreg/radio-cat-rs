# Icom CI-V Protocol Notes

This document describes the Icom CI-V protocol families represented under `rigs/icom/`, with emphasis on the command surface needed for a Rust implementation.

It is source-derived documentation rather than a manufacturer protocol manual. The goal is to capture the practical protocol families, frame structure, mode maps, VFO/split behavior, bandwidth handling, and model-specific exceptions that matter when implementing compatible clients.

## Scope and Legend

The main sections below cover Icom-branded models first. CI-V-compatible non-Icom profiles that also live in this directory are summarized separately near the end.

The feature matrix focuses on these core capabilities:

- `Freq`: get/set current frequency.
- `Other VFO freq`:
  - `yes`: the protocol can directly target the alternate VFO/receiver side.
  - `partial`: the alternate VFO exists, but targeting is usually implemented by VFO swaps or split helpers rather than direct addressing.
  - `no`: effectively single-VFO or current-VFO only.
- `Mode`: get/set mode.
- `Bandwidth`:
  - `preset`: passband is selected through 2- or 3-slot filter presets.
  - `dsp`: richer DSP or numeric-width handling is available.
  - `mixed`: width support exists, but set/get behavior is asymmetric or model-specific.
  - `fixed`: no meaningful variable-width handling in scope here.
- `RIT/XIT`:
  - `none`: no usable CI-V RIT/XIT path.
  - `new`: newer `0x21` RIT/XIT command family.
  - `custom`: non-generic custom implementation.
- `Morse`:
  - `send`: arbitrary text can be sent as CW.
  - `send+stop`: arbitrary text plus explicit stop.

## Feature Summary

| Models | Protocol family | Freq | Other VFO freq | Mode | Bandwidth | RIT/XIT | Keyer speed | NR | NB | Morse | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `IC-707`, `IC-725`, `IC-726`, `IC-728`, `IC-729`, `IC-735`, `IC-736`, `IC-737`, `IC-738`, `IC-751`, `IC-761`, `IC-765`, `IC-775`, `IC-781` | early HF CI-V | yes | partial | yes | preset | none | no | no | no | no | Older CI-V HF sets. VFO B and split are usually reachable only through current-VFO changes and split helpers. |
| `IC-271`, `IC-275`, `IC-375`, `IC-471`, `IC-475`, `IC-575`, `IC-820H`, `IC-821H`, `IC-970`, `IC-1275` | early VHF/UHF CI-V | yes | partial | yes | preset | none | no | no | no | no | Similar control surface to the early HF family, but with VHF/UHF mode sets. `IC-820H` and `IC-821H` use the old 731-style frequency format. |
| `IC-706`, `IC-706MkII`, `IC-706MkIIG` | `706` two-filter family | yes | partial | yes | preset | none | no | no | yes | no | Uses a special 0/1/2 passband encoding instead of the more common 1/2/3 scheme. |
| `IC-78` | minimal enhanced HF | yes | partial | yes | preset | none | no | no | yes | no | Adds NB and compressor-related controls, but still has a relatively small CI-V surface. |
| `IC-703` | compact DSP HF | yes | partial | yes | dsp | none | no | yes | yes | no | DSP-style controls are present, but no CW keyer-speed path is exposed here. |
| `IC-718` | entry HF with more controls | yes | partial | yes | preset | none | yes | yes | yes | no | Adds keyer speed and NR/NB, but still uses the older non-targetable VFO model. |
| `IC-746`, `IC-746PRO` | pre-direct-target DSP family | yes | partial | yes | mixed | none | yes | yes | yes | no | DSP passband handling exists, but direct alternate-VFO targeting is still absent. |
| `IC-756`, `IC-756PRO`, `IC-756PROII`, `IC-756PROIII` | `756` DSP family | yes | partial | yes | mixed | none | yes | yes | yes | no | Uses older DSP preset families. Packet/data handling is richer than the early rigs, but still not in the newer targetable-VFO class. |
| `IC-7000` | `7000` width-index family | yes | partial | yes | dsp | none | yes | yes | yes | no | Uses its own 0..40 width mapping rather than plain 2/3-slot filter presets. |
| `IC-7200` | data-mode + filter family | yes | partial | yes | dsp | none | yes | yes | yes | no | Supports data-mode overlay and filter-aware mode handling, but no direct alternate-VFO targeting and no morse path in scope here. |
| `IC-7410` | late non-targetable HF DSP | yes | partial | yes | dsp | none | yes | yes | yes | `send+stop` | Has data-mode support and arbitrary-text CW send, but still uses the non-targetable VFO model. |
| `IC-9100` | all-band DSP, no direct-target RIT | yes | partial | yes | dsp | none | yes | yes | yes | no | Supports data-mode plus filter handling, but the newer RIT/XIT CI-V path is not exposed here. |
| `IC-910` | satellite-era special family | yes | partial | yes | preset | none | yes | yes | yes | no | Custom mode mapper. Has more functions than the older VHF/UHF family, but not the newer direct-target mode/frequency commands. |
| `IC-7100` | transitional direct-target family | yes | yes | yes | dsp | none | yes | yes | yes | `send+stop` | Supports direct-target frequency/mode when `0x25/0x26` are available, but still lacks the newer RIT/XIT command family here. |
| `IC-7600`, `IC-7700`, `IC-7800` | first new-RIT / partial direct-target family | yes | yes | yes | dsp | new | yes | yes | yes | `send` on `7600`, `send+stop` on `7700/7800` | Uses the newer `0x21` RIT/XIT family. `0x25/0x26` may be firmware-dependent on some of these rigs. |
| `IC-7300`, `IC-7300MK2`, `IC-705`, `IC-7610`, `IC-7760`, `IC-7850/7851`, `IC-905`, `IC-9700` | modern direct-target CI-V | yes | yes | yes | dsp | new | yes | yes | yes | `send+stop` | Direct-target `0x25/0x26` family, newer data-mode handling, newer RIT family, and arbitrary-text CW send. `IC-7610`, `IC-7850/7851`, and `IC-9700` also expose spectrum targeting. `IC-9700` is narrower than the rest on XIT exposure. |
| `IC-R71`, `IC-R72`, `IC-R7000`, `IC-R7100`, `IC-R8500`, `IC-R9000` | older receiver CI-V | yes | no | yes | preset | none | no | little or none | little or none | no | Receive-only family. `IC-R7000` has a special SSB mode encoding. |
| `IC-R10`, `IC-R20`, `IC-R6`, `IC-RX7` | handheld receiver family | yes | no | yes | fixed | none | no | no | no | no | Narrow receive-oriented CI-V subset. |
| `IC-R30` | modern handheld receiver family | yes | no | yes | mixed | none | no | no | yes | no | `AMN` and `FMN` are represented through special width/filter handling rather than distinct base mode codes. |
| `IC-R75`, `IC-R8600`, `IC-R9500` | desktop receiver DSP family | yes | no | yes | dsp | none | no | yes | yes | no | Richer DSP receive-side controls, but still receiver-only. |
| `IC ID-1`, `ID-31`, `ID-51`, `ID-52A/E PLUS`, `IC-92D`, `ID-4100`, `ID-5100`, `IC-2730` | handheld/mobile digital or FM family | yes | partial | yes | fixed or preset | none | no | no | no | no | Mostly VHF/UHF FM or D-STAR-oriented profiles. `ID-5100` has the most custom VFO/frequency handling in this group. |
| `IC-F8101` | fixed-service special family | yes | partial | yes | fixed | none | no | yes | yes | no | Has custom mode codes for `USBD1/2/3` and `LSBD1/2/3`, plus custom split handling. |

## Shared CI-V Frame Structure

CI-V is a binary framed protocol, not a line-oriented text protocol.

### Frame layout

Most commands use:

1. `0xfe`
2. `0xfe`
3. destination CI-V address
4. controller address
5. command byte
6. optional subcommand byte or bytes
7. optional payload bytes
8. `0xfd`

Important control bytes:

- `0xfe`: preamble
- `0xfd`: end of frame
- `0xfb`: ACK
- `0xfa`: NAK
- `0xfc`: bus collision

### Transport behavior

- CI-V may echo the command frame back before the real response.
- On shared or half-duplex buses, collisions are real and are surfaced explicitly with `0xfc`.
- A robust implementation should be prepared to:
  - discard self-echo
  - retry after collision
  - ignore asynchronous frames that do not match the outstanding command

## Core Command Families

### Frequency

Core frequency commands are:

- `0x03`: read current frequency
- `0x05`: set current frequency

Frequency payloads are usually packed BCD rather than ASCII.

Important format families:

- most rigs: 5-byte packed BCD frequency
- old `731`-style family: 4-byte packed BCD plus passband implications
- `ID-5100`: 3-byte frequency field in units of `10 kHz`
- some TX-frequency paths and newer direct-target reads may return different field lengths

Special notes:

- `IC-735`, `IC-820H`, `IC-821H`, and `Delta II` are the main built-in `731`-mode profiles.
- `IC-R7000` and `IC-R7100` use a custom set-frequency path.
- `IC-F8101` uses custom set/get frequency logic.

### Direct-target frequency and mode

Newer rigs can often bypass VFO swaps with:

- `0x25`: selected/unselected or main/sub frequency
- `0x26`: selected/unselected or main/sub mode/data/filter

Practical families:

- always modern direct-target:
  - `IC-7300`
  - `IC-7300MK2`
  - `IC-705`
  - `IC-7610`
  - `IC-7760`
  - `IC-7850/7851`
  - `IC-905`
  - `IC-9700`
- firmware-probed or model-dependent:
  - `IC-7100`
  - `IC-7600`
  - `IC-7700`
  - `IC-7800`

Fallback behavior on older rigs is usually:

- switch current VFO or band
- issue the normal current-VFO command
- switch back if needed

### Split and alternate VFO control

The shared split-control command is:

- `0x0f`

In practice, split handling falls into three families:

1. non-targetable legacy rigs:
   - split is controlled through the split command plus VFO changes
   - alternate-VFO frequency and mode are typically reached by swapping current VFO
2. direct-target rigs:
   - `0x25` and `0x26` can target selected/unselected or main/sub sides directly
3. custom split rigs:
   - `ID-5100`
   - `IC-F8101`
   - some Xiegu-compatible profiles

Important caveat:

- rigs with both main/sub receivers and A/B VFOs for each side still cannot always target every sub-side combination directly, even when `0x25/0x26` exist

## Mode Enumeration

### Generic CI-V mode table

The shared base mode map is:

| Code | Mode |
| --- | --- |
| `0x00` | `LSB` |
| `0x01` | `USB` |
| `0x02` | `AM` or `AMN` |
| `0x03` | `CW` |
| `0x04` | `RTTY` |
| `0x05` | `FM` or `FMN` |
| `0x06` | `WFM` |
| `0x07` | `CWR` |
| `0x08` | `RTTYR` |
| `0x11` | `AMS` |
| `0x12` | `PSK` |
| `0x13` | `PSKR` |
| `0x16` | `P25` |
| `0x17` | `D-STAR` |
| `0x18` | `dPMR` |
| `0x19` | `NXDN-VN` |
| `0x20` | `NXDN-N` |
| `0x21` | `DCR` |
| `0x22` | `DD` |

### Data-mode overlay

On many rigs, packet/data modes are not independent base mode bytes.

Instead they are represented as:

- base mode:
  - `USB`
  - `LSB`
  - `FM`
  - `AM`
- plus a separate data-mode state

That means:

- `PKTUSB` is usually `USB + data`
- `PKTLSB` is usually `LSB + data`
- `PKTFM` is usually `FM + data`
- `PKTAM` is usually `AM + data`

This matters for both set and get logic on:

- `IC-7200`
- `IC-7410`
- `IC-9100`
- `IC-7100`
- `IC-7600`
- `IC-7700`
- `IC-7800`
- `IC-7300` family
- `IC-7610`
- `IC-7760`
- `IC-7850/7851`

### Notable mode-family exceptions

#### `IC-706` family

- Uses a special passband encoding, not a different base mode table.
- The important difference is bandwidth coding, not the mode byte itself.

#### `IC-R7000`

- SSB is represented using the FM mode code plus a special passband value.

#### `IC-7800`

- `PKTUSB` and `PKTLSB` are mapped through the PSK/PSKR codes rather than the normal data-mode overlay.

#### `IC-R30`

- `AMN` and `FMN` are represented as `AM` or `FM` plus a special narrow filter value.

#### `IC-F8101`

This is the main Icom-family outlier in the directory. It has explicit extra mode codes for:

- `LSBD1` = `0x18`
- `USBD1` = `0x19`
- `LSBD2` = `0x20`
- `USBD2` = `0x21`
- `LSBD3` = `0x22`
- `USBD3` = `0x23`

For a Rust implementation, `IC-F8101` should have its own mode family.

## Bandwidth and Filter Handling

Bandwidth is one of the biggest CI-V differences across models.

### 1. Two-filter preset family

Used by:

- `IC-706`
- `IC-706MkII`
- `IC-706MkIIG`
- parts of the older non-DSP families

Practical behavior:

- only wide/normal/narrow-style presets are available
- `IC-706` family uses a shifted encoding:
  - `0` = wide
  - `1` = normal
  - `2` = narrow

### 2. Three-preset family

Common on many older CI-V rigs:

- `1` = wide
- `2` = medium or normal
- `3` = narrow

This is the fallback interpretation used by the generic CI-V mapper.

### 3. DSP-width family

Used by the DSP-oriented transceivers and receivers:

- `IC-7000`
- `IC-7200`
- `IC-7410`
- `IC-7600`
- `IC-7700`
- `IC-7800`
- `IC-9100`
- `IC-7300` family
- `IC-7610`
- `IC-7760`
- `IC-7850/7851`
- `IC-R75`
- `IC-R8600`
- `IC-R9500`

Important details:

- some rigs expose only preset numbers in the mode command, but allow richer width selection via other DSP/filter commands
- some rigs can set width but cannot reliably report it back through normal mode reads

### 4. Known set/get asymmetry

The source explicitly treats these rigs as not supporting normal width query in `get_mode`:

- `IC-910`
- `Omni VI Plus`
- `IC-706`
- `IC-706MkII`
- `IC-706MkIIG`
- `IC-756`
- `IC-756PROII`
- `IC-756PROIII`
- `IC-R30`

The source also suppresses passband bytes on some sets, including:

- `IC-375`
- `IC-726`
- `IC-475`
- `IC-746`
- `IC-746PRO`
- `IC-756`
- `IC-756PROII`
- `IC-756PROIII`
- `IC-910`
- `IC-7000`
- any active `731`-mode profile

For a Rust implementation, it is best to model width support as a family enum rather than as a single bool.

## RIT and XIT

### Older Icom family

Most older Icom-branded rigs in this directory do not expose a usable CI-V RIT/XIT path.

That includes:

- early HF family
- early VHF/UHF family
- `IC-706`
- `IC-7000`
- `IC-7200`
- `IC-7410`
- `IC-7100`
- `IC-9100`
- nearly all receive-only profiles

### Newer `0x21` RIT/XIT family

Newer rigs use:

- command `0x21`
- newer subcommands for reading and writing the offset directly

This family is present on:

- `IC-7600`
- `IC-7700`
- `IC-7800`
- `IC-7610`
- `IC-7760`
- `IC-7850/7851`
- `IC-7300`
- `IC-7300MK2`
- `IC-705`
- `IC-905`
- `IC-9700`

Important practical note:

- the newer implementation uses one underlying offset register for both RIT and XIT
- enabling one or both keeps them effectively synchronized

`IC-9700` is slightly narrower than the rest of this family in the current source:

- RIT is present
- XIT is not exposed through the rig-caps surface

### Custom non-Icom-compatible exception

- `Omni VI Plus` has a custom RIT/XIT implementation in this directory, but it is not generic Icom CI-V behavior

## Keyer Speed, NR, and NB

### Keyer speed

Broadly:

- no keyer-speed path on the oldest HF/VHF/UHF and receiver families
- present on most DSP transceivers:
  - `IC-718`
  - `IC-746` family
  - `IC-756` family
  - `IC-7000`
  - `IC-7200`
  - `IC-7410`
  - `IC-7100`
  - `IC-910`
  - `IC-9100`
  - `IC-7600`
  - `IC-7700`
  - `IC-7800`
  - `IC-7300` family
  - `IC-7610`
  - `IC-7760`
  - `IC-7850/7851`

### Noise reduction

Broadly:

- absent on the oldest transceiver and receiver families
- present on most DSP transceivers and on several DSP receivers
- absent on most handheld/mobile FM and D-STAR rigs in this directory

### Noise blanker

Broadly:

- absent on the oldest CI-V families
- present on most HF DSP transceivers
- present on several desktop receivers such as `IC-R75`, `IC-R8500`, `IC-R8600`, and `IC-R9500`

## Morse Send and Stop

For this document, “supports morse send” means arbitrary caller-provided text, not pre-programmed message playback.

The CI-V morse command family is:

- command `0x17` with up to 30 bytes of message payload
- stop by sending a single `0xff` payload byte to the same command

This is true arbitrary text send, not stored-memory playback.

Supported models in this directory:

- `IC-7410`: `send+stop`
- `IC-7100`: `send+stop`
- `IC-7600`: `send`
- `IC-7700`: `send+stop`
- `IC-7800`: `send+stop`
- `IC-7300`: `send+stop`
- `IC-7300MK2`: `send+stop`
- `IC-705`: `send+stop`
- `IC-7610`: `send+stop`
- `IC-7760`: `send+stop`
- `IC-7850/7851`: `send+stop`
- `IC-905`: `send+stop`
- `IC-9700`: `send+stop`

Everything else in the Icom-branded set should be treated as not supporting arbitrary-text morse send.

## Recommended Rust Family Model

The cleanest implementation strategy is to make the model descriptor carry CI-V family selectors rather than baking all decisions directly into per-model code.

Suggested descriptor fields:

- `civ_address`
- `freq_family`
- `vfo_family`
- `split_family`
- `mode_family`
- `width_family`
- `data_mode_family`
- `rit_family`
- `morse_family`
- `receiver_only`

Practical family values from this directory would look like:

| Models | Suggested family selectors |
| --- | --- |
| early HF CI-V | `freq=bcd5_or_731`, `vfo=swap_only`, `mode=generic`, `width=preset_legacy`, `rit=none`, `morse=none` |
| early VHF/UHF CI-V | `freq=bcd5_or_731`, `vfo=swap_only`, `mode=generic_vhf`, `width=preset_legacy`, `rit=none`, `morse=none` |
| `IC-706` family | `width=icom706_preset2` |
| `IC-7000` | `width=icom7000_indexed`, `data_mode=legacy_overlay` |
| `IC-7200`, `IC-7410`, `IC-9100` | `width=dsp_filter`, `data_mode=ctl_mem_overlay` |
| `IC-7100` | `vfo=direct_if_available`, `mode=targetable_if_available`, `rit=none`, `morse=send_stop` |
| `IC-7600`, `IC-7700`, `IC-7800` | `vfo=direct_if_available`, `mode=targetable_if_available`, `rit=new_0x21` |
| modern `7300/7610/7760/785x/705/905/9700` family | `vfo=direct_always`, `mode=direct_always`, `data_mode=x26`, `rit=new_0x21`, `morse=send_stop_or_send_only` |
| `IC-R7000` | `mode=r7000_ssb_special` |
| `IC-R30` | `mode=icr30_narrow_special`, `width=mixed_receiver` |
| `IC-F8101` | `mode=icf8101_special`, `freq=icf8101_special`, `split=icf8101_special` |

## CI-V-Compatible Profiles in This Directory

These are not Icom-branded rigs, but they share the same backend directory because they emulate or extend CI-V-like behavior.

| Models | Notes |
| --- | --- |
| `Omni VI Plus` | CI-V-like profile with custom RIT/XIT behavior. Not a generic Icom mode/VFO implementation. |
| `Delta II` | Legacy profile using old `731`-style frequency behavior. |
| `OptoScan456`, `OptoScan535` | Receiver-control profiles layered onto CI-V framing. |
| `Perseus` | Custom mode mapper on top of CI-V transport. |
| `X108G`, `X6100`, `X6200`, `G90`, `X5105` | CI-V-compatible Xiegu profiles, with custom split behavior on `X108G` and partial direct-target behavior on newer models. |

If the Rust project only needs actual Icom transceivers and receivers, treat this section as out of scope.
