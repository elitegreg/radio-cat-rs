use crate::{
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::StatePatch,
    Mode, RadioState, Result,
};

use super::{DecodedFrame, EncodedCommand};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiProfile, ResponseMatcher,
};

pub fn encode(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
    state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetReceiverFilterBandwidth {
            receiver,
            bandwidth_hz,
        } => {
            require_filter_capability(profile, *receiver, true)?;
            Ok(Some(encode_bandwidth(
                profile,
                *receiver,
                *bandwidth_hz,
                state,
            )?))
        }
        RadioCommand::SetReceiverFilterShift { receiver, shift_hz } => {
            require_filter_capability(profile, *receiver, false)?;
            Ok(Some(encode_shift(profile, *receiver, *shift_hz, state)?))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    let (frames, matcher) = match semantic {
        "filter-state" => filter_state_queries(profile)?,
        "filter-hi-lo" => filter_hi_lo_queries(profile, ReceiverPath::Main)?,
        "filter-hi-lo-main" => filter_hi_lo_queries(profile, ReceiverPath::Main)?,
        "filter-hi-lo-sub" => filter_hi_lo_queries(profile, ReceiverPath::Sub)?,
        "FW" | "BW" | "BW$" | "IS" | "IS$" | "SH0" | "SH1" | "NA0" => {
            let frame = AsciiFrame::new(format!("{semantic};"))?;
            let matcher = match semantic {
                "BW$" => ResponseMatcher::Prefix("BW$"),
                "IS$" => ResponseMatcher::Prefix("IS$"),
                _ => ResponseMatcher::Prefix(frame.command_static_hint()),
            };
            (vec![frame], matcher)
        }
        _ => return Ok(None),
    };

    Ok(Some(EncodedCommand::new(
        frames,
        matcher,
        Vec::new(),
        CommandPriority::Normal,
    )))
}

pub fn decode(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
) -> Result<Option<DecodedFrame>> {
    let patches = match frame.command() {
        "FW" => decode_fw(frame)?,
        "BW" | "BW$" => decode_bw(frame)?,
        "IS" | "IS$" => decode_is(profile, frame)?,
        "SH" => decode_sh(profile, frame, state)?,
        "SL" => decode_sl(profile, frame, state)?,
        "NA" if is_ft891_or_ft991(profile) => Vec::new(),
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

fn encode_bandwidth(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    bandwidth_hz: u16,
    state: &RadioState,
) -> Result<EncodedCommand> {
    if is_unsupported_filter_profile(profile) {
        return Err(RadioError::UnsupportedCapability {
            capability: "receiver.filter_bandwidth",
        });
    }

    if is_elecraft_bw_is(profile) {
        let suffix = if matches!(receiver, ReceiverPath::Sub) {
            "$"
        } else {
            ""
        };
        let value = bandwidth_hz.div_ceil(10);
        let frame = AsciiFrame::new(format!("BW{suffix}{value};"))?;
        let matcher = if suffix.is_empty() {
            ResponseMatcher::Prefix("BW")
        } else {
            ResponseMatcher::Prefix("BW$")
        };
        return Ok(EncodedCommand::new(
            vec![frame],
            matcher,
            vec![bandwidth_patch(receiver, bandwidth_hz)],
            CommandPriority::Normal,
        ));
    }

    if profile.id() == "elecraft-k2" {
        return Ok(EncodedCommand::new(
            vec![AsciiFrame::new(format!("FW{bandwidth_hz:04};"))?],
            ResponseMatcher::Prefix("FW"),
            vec![bandwidth_patch(receiver, bandwidth_hz)],
            CommandPriority::Normal,
        ));
    }

    if is_yaesu(profile) {
        let mode = receiver_mode(state, receiver).unwrap_or(Mode::Usb);
        let selection = if is_ft891_or_ft991(profile) {
            let selection = select_yaesu_891_991_bandwidth(mode, bandwidth_hz)?;
            TableSelection {
                id: selection.id,
                value_hz: selection.value_hz,
            }
        } else {
            select_yaesu_bandwidth(mode, bandwidth_hz)
        };
        let frame = AsciiFrame::new(encode_yaesu_bandwidth_payload(profile, receiver, selection.id))?;
        return Ok(EncodedCommand::new(
            vec![frame],
            ResponseMatcher::Prefix("SH"),
            vec![bandwidth_patch(receiver, selection.value_hz)],
            CommandPriority::Normal,
        ));
    }

    if uses_direct_fw_for_mode(profile, state, receiver) {
        return Ok(EncodedCommand::new(
            vec![AsciiFrame::new(format!("FW{bandwidth_hz:04};"))?],
            ResponseMatcher::Prefix("FW"),
            vec![bandwidth_patch(receiver, bandwidth_hz)],
            CommandPriority::Normal,
        ));
    }

    let shift_hz = receiver_shift(state, receiver).unwrap_or(0);
    let mode = receiver_mode(state, receiver).unwrap_or(Mode::Usb);
    let (high_id, low_id, actual_bw, actual_shift) =
        select_hi_lo_for_bandwidth(profile, receiver, mode, bandwidth_hz, shift_hz)?;
    let frames = encode_hi_lo_set(profile, receiver, high_id, low_id)?;

    Ok(EncodedCommand::new(
        frames,
        ResponseMatcher::OneOf(&["SH", "SL"]),
        vec![
            bandwidth_patch(receiver, actual_bw),
            shift_patch(receiver, actual_shift),
        ],
        CommandPriority::Normal,
    ))
}

fn encode_shift(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    shift_hz: i16,
    state: &RadioState,
) -> Result<EncodedCommand> {
    if is_shift_unsupported_profile(profile) {
        return Err(RadioError::UnsupportedCapability {
            capability: "receiver.filter_shift",
        });
    }

    if is_elecraft_bw_is(profile) {
        let suffix = if matches!(receiver, ReceiverPath::Sub) {
            "$"
        } else {
            ""
        };
        let encoded_shift = if profile.id() == "elecraft-k4" {
            shift_hz / 10
        } else {
            shift_hz
        };
        let optimistic_shift = if profile.id() == "elecraft-k4" {
            encoded_shift * 10
        } else {
            shift_hz
        };

        let (sign, abs) = signed_parts_space_positive(encoded_shift);
        let frame = AsciiFrame::new(format!("IS{suffix}{sign}{abs:04};"))?;
        let matcher = if suffix.is_empty() {
            ResponseMatcher::Prefix("IS")
        } else {
            ResponseMatcher::Prefix("IS$")
        };

        return Ok(EncodedCommand::new(
            vec![frame],
            matcher,
            vec![shift_patch(receiver, optimistic_shift)],
            CommandPriority::Normal,
        ));
    }

    if is_yaesu(profile) {
        let frame = AsciiFrame::new(encode_yaesu_shift_payload(profile, receiver, shift_hz))?;
        return Ok(EncodedCommand::new(
            vec![frame],
            ResponseMatcher::Prefix("IS"),
            vec![shift_patch(receiver, shift_hz)],
            CommandPriority::Normal,
        ));
    }

    if uses_direct_is_for_mode(profile, state, receiver) {
        let (sign, abs) = signed_parts_space_positive(shift_hz);
        let frame = AsciiFrame::new(format!("IS{sign}{abs:04};"))?;
        return Ok(EncodedCommand::new(
            vec![frame],
            ResponseMatcher::Prefix("IS"),
            vec![shift_patch(receiver, shift_hz)],
            CommandPriority::Normal,
        ));
    }

    let bandwidth_hz = receiver_bandwidth(state, receiver).unwrap_or(2_400);
    let mode = receiver_mode(state, receiver).unwrap_or(Mode::Usb);
    let (high_id, low_id, actual_bw, actual_shift) =
        select_hi_lo_for_shift(profile, receiver, mode, bandwidth_hz, shift_hz)?;
    let frames = encode_hi_lo_set(profile, receiver, high_id, low_id)?;

    Ok(EncodedCommand::new(
        frames,
        ResponseMatcher::OneOf(&["SH", "SL"]),
        vec![
            bandwidth_patch(receiver, actual_bw),
            shift_patch(receiver, actual_shift),
        ],
        CommandPriority::Normal,
    ))
}

fn decode_fw(frame: &AsciiFrame) -> Result<Vec<StatePatch>> {
    let bandwidth_hz = parse_u16(frame.command_static_hint(), frame.payload())?;
    Ok(vec![StatePatch::MainRxFilterBandwidth(bandwidth_hz)])
}

fn decode_bw(frame: &AsciiFrame) -> Result<Vec<StatePatch>> {
    let (receiver, payload) = if frame.command() == "BW$" {
        (ReceiverPath::Sub, frame.payload())
    } else {
        (ReceiverPath::Main, frame.payload())
    };

    let raw = parse_u16(frame.command_static_hint(), payload)?;
    Ok(vec![bandwidth_patch(receiver, raw.saturating_mul(10))])
}

fn decode_is(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<Vec<StatePatch>> {
    if frame.command() == "IS$" {
        let mut shift = parse_signed(frame.command_static_hint(), frame.payload())?;
        if profile.id() == "elecraft-k4" {
            shift *= 10;
        }
        return Ok(vec![shift_patch(ReceiverPath::Sub, shift)]);
    }

    if is_yaesu(profile) {
        let (receiver, signed) = decode_yaesu_is_parts(profile, frame.payload())?;
        let shift = parse_signed("IS", signed)?;
        return Ok(vec![shift_patch(receiver, shift)]);
    }

    let mut shift = parse_signed(frame.command_static_hint(), frame.payload())?;
    if profile.id() == "elecraft-k4" {
        shift *= 10;
    }
    Ok(vec![shift_patch(ReceiverPath::Main, shift)])
}

fn decode_sh(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    if is_yaesu(profile) {
        return decode_yaesu_sh(profile, frame, state);
    }

    if !supports_hi_lo(profile) {
        return Ok(Vec::new());
    }

    let (receiver, high_id) = parse_hi_lo_id(profile, frame.payload(), true)?;
    let mode = receiver_mode(state, receiver).unwrap_or(Mode::Usb);
    let table = hi_lo_table(profile, mode)?;
    let high = table
        .high_values
        .get(high_id as usize)
        .copied()
        .ok_or(RadioError::Decode {
            command: "SH",
            message: format!("unknown high-cut id {high_id} for {}", profile.id()),
        })?;

    let (bandwidth, shift) = reconstruct_from_single_edge(
        state,
        receiver,
        table.neutral_center_hz,
        Some(high),
        None,
        &table.low_values,
    );

    Ok(vec![
        bandwidth_patch(receiver, bandwidth),
        shift_patch(receiver, shift),
    ])
}

fn decode_sl(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    if !supports_hi_lo(profile) {
        return Ok(Vec::new());
    }

    let (receiver, low_id) = parse_hi_lo_id(profile, frame.payload(), false)?;
    let mode = receiver_mode(state, receiver).unwrap_or(Mode::Usb);
    let table = hi_lo_table(profile, mode)?;
    let low = table
        .low_values
        .get(low_id as usize)
        .copied()
        .ok_or(RadioError::Decode {
            command: "SL",
            message: format!("unknown low-cut id {low_id} for {}", profile.id()),
        })?;

    let (bandwidth, shift) = reconstruct_from_single_edge(
        state,
        receiver,
        table.neutral_center_hz,
        None,
        Some(low),
        &table.high_values,
    );

    Ok(vec![
        bandwidth_patch(receiver, bandwidth),
        shift_patch(receiver, shift),
    ])
}

fn decode_yaesu_sh(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    let (receiver, id) = decode_yaesu_sh_parts(profile, frame.payload())?;
    let mode = receiver_mode(state, receiver).unwrap_or(Mode::Usb);

    let value_hz = if is_ft891_or_ft991(profile) {
        // No private NA0 state in public model yet; choose the closest value to current bandwidth.
        let current = receiver_bandwidth(state, receiver).unwrap_or(2_400);
        decode_yaesu_891_991_bandwidth(mode, id, current)?
    } else {
        decode_yaesu_bandwidth(mode, id)
    };

    Ok(vec![bandwidth_patch(receiver, value_hz)])
}

fn filter_state_queries(
    profile: &KenwoodAsciiProfile,
) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    if is_unsupported_filter_profile(profile) {
        return Err(RadioError::UnsupportedCapability {
            capability: "receiver.filter_bandwidth",
        });
    }

    if is_elecraft_bw_is(profile) {
        let mut frames = vec![AsciiFrame::new("BW;")?, AsciiFrame::new("IS;")?];
        if profile.id() == "elecraft-k4" || profile.id() == "elecraft-k3" {
            frames.push(AsciiFrame::new("BW$;")?);
            frames.push(AsciiFrame::new("IS$;")?);
        }
        return Ok((frames, ResponseMatcher::OneOf(&["BW", "BW$", "IS", "IS$"])));
    }

    if profile.id() == "elecraft-k2" {
        return Ok((vec![AsciiFrame::new("FW;")?], ResponseMatcher::Prefix("FW")));
    }

    if is_yaesu(profile) {
        let mut frames = Vec::new();
        if profile.id() == "yaesu-ftdx101" {
            frames.push(AsciiFrame::new("SH0;")?);
            frames.push(AsciiFrame::new("SH1;")?);
            frames.push(AsciiFrame::new("IS0;")?);
            frames.push(AsciiFrame::new("IS1;")?);
        } else {
            frames.push(AsciiFrame::new("SH0;")?);
            frames.push(AsciiFrame::new("IS0;")?);
        }
        return Ok((frames, ResponseMatcher::OneOf(&["SH", "IS"])));
    }

    if supports_hi_lo(profile) {
        let (frames, _) = filter_hi_lo_queries(profile, ReceiverPath::Main)?;
        return Ok((frames, ResponseMatcher::OneOf(&["SH", "SL", "FW", "IS"])));
    }

    Err(RadioError::UnsupportedCapability {
        capability: "receiver.filter_bandwidth",
    })
}

fn filter_hi_lo_queries(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    if !supports_hi_lo(profile) {
        return Err(RadioError::UnsupportedCapability {
            capability: "receiver.filter_bandwidth",
        });
    }

    let frames = if profile.id() == "kenwood-ts890" {
        vec![AsciiFrame::new("SH0;")?, AsciiFrame::new("SL0;")?]
    } else if profile.id() == "kenwood-ts990" {
        let t = match receiver {
            ReceiverPath::Main => '0',
            ReceiverPath::Sub => '1',
        };
        vec![
            AsciiFrame::new(format!("SH{t};"))?,
            AsciiFrame::new(format!("SL{t};"))?,
        ]
    } else {
        vec![AsciiFrame::new("SH;")?, AsciiFrame::new("SL;")?]
    };

    Ok((frames, ResponseMatcher::OneOf(&["SH", "SL"])))
}

fn select_hi_lo_for_bandwidth(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    mode: Mode,
    requested_bw: u16,
    desired_shift: i16,
) -> Result<(u16, u16, u16, i16)> {
    let table = hi_lo_table(profile, mode)?;
    let candidate = table
        .best_for_bandwidth(requested_bw, desired_shift)
        .ok_or(RadioError::Decode {
            command: "filter",
            message: format!("no hi/lo combinations available for {}", profile.id()),
        })?;

    let _ = receiver;
    Ok((
        candidate.high_id,
        candidate.low_id,
        candidate.bandwidth_hz,
        candidate.shift_hz,
    ))
}

fn select_hi_lo_for_shift(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    mode: Mode,
    desired_bw: u16,
    requested_shift: i16,
) -> Result<(u16, u16, u16, i16)> {
    let table = hi_lo_table(profile, mode)?;
    let candidate =
        table
            .best_for_shift(desired_bw, requested_shift)
            .ok_or(RadioError::Decode {
                command: "filter",
                message: format!("no hi/lo combinations available for {}", profile.id()),
            })?;

    let _ = receiver;
    Ok((
        candidate.high_id,
        candidate.low_id,
        candidate.bandwidth_hz,
        candidate.shift_hz,
    ))
}

fn encode_hi_lo_set(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    high_id: u16,
    low_id: u16,
) -> Result<Vec<AsciiFrame>> {
    if profile.id() == "kenwood-ts890" {
        return Ok(vec![
            AsciiFrame::new(format!("SH0{high_id:03};"))?,
            AsciiFrame::new(format!("SL0{low_id:02};"))?,
        ]);
    }

    if profile.id() == "kenwood-ts990" {
        let target = match receiver {
            ReceiverPath::Main => '0',
            ReceiverPath::Sub => '1',
        };
        return Ok(vec![
            AsciiFrame::new(format!("SH{target}{high_id:03};"))?,
            AsciiFrame::new(format!("SL{target}{low_id:02};"))?,
        ]);
    }

    Ok(vec![
        AsciiFrame::new(format!("SH{high_id:02};"))?,
        AsciiFrame::new(format!("SL{low_id:02};"))?,
    ])
}

fn parse_hi_lo_id(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    high: bool,
) -> Result<(ReceiverPath, u8)> {
    if profile.id() == "kenwood-ts890" {
        if payload.len() != if high { 4 } else { 3 } {
            return Err(RadioError::Decode {
                command: if high { "SH" } else { "SL" },
                message: format!("unexpected TS-890 payload {payload:?}"),
            });
        }
        let receiver = decode_target(payload.as_bytes()[0])?;
        let id = parse_u8(if high { "SH" } else { "SL" }, &payload[1..])?;
        return Ok((receiver, id));
    }

    if profile.id() == "kenwood-ts990" {
        if payload.len() != if high { 4 } else { 3 } {
            return Err(RadioError::Decode {
                command: if high { "SH" } else { "SL" },
                message: format!("unexpected TS-990 payload {payload:?}"),
            });
        }
        let receiver = decode_target(payload.as_bytes()[0])?;
        let id = parse_u8(if high { "SH" } else { "SL" }, &payload[1..])?;
        return Ok((receiver, id));
    }

    let id = parse_u8(if high { "SH" } else { "SL" }, payload)?;
    Ok((ReceiverPath::Main, id))
}

fn decode_target(byte: u8) -> Result<ReceiverPath> {
    match byte {
        b'0' => Ok(ReceiverPath::Main),
        b'1' => Ok(ReceiverPath::Sub),
        _ => Err(RadioError::Decode {
            command: "filter",
            message: format!("expected target 0/1, got {:?}", byte as char),
        }),
    }
}

fn parse_u16(command: &'static str, payload: &str) -> Result<u16> {
    payload.parse::<u16>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })
}

fn parse_u8(command: &'static str, payload: &str) -> Result<u8> {
    payload.parse::<u8>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })
}

fn parse_signed(command: &'static str, payload: &str) -> Result<i16> {
    payload
        .trim()
        .parse::<i16>()
        .map_err(|error| RadioError::Decode {
            command,
            message: error.to_string(),
        })
}

fn require_filter_capability(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    bandwidth: bool,
) -> Result<()> {
    let rx = match receiver {
        ReceiverPath::Main => &profile.capabilities.main_rx,
        ReceiverPath::Sub => profile
            .capabilities
            .sub_rx
            .as_ref()
            .unwrap_or(&profile.capabilities.main_rx),
    };

    let cap = if bandwidth {
        rx.filter_bandwidth
    } else {
        rx.filter_shift
    };

    if cap.can_write() {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability {
            capability: if bandwidth {
                "receiver.filter_bandwidth"
            } else {
                "receiver.filter_shift"
            },
        })
    }
}

fn bandwidth_patch(receiver: ReceiverPath, bandwidth_hz: u16) -> StatePatch {
    match receiver {
        ReceiverPath::Main => StatePatch::MainRxFilterBandwidth(bandwidth_hz),
        ReceiverPath::Sub => StatePatch::SubRxFilterBandwidth(bandwidth_hz),
    }
}

fn shift_patch(receiver: ReceiverPath, shift_hz: i16) -> StatePatch {
    match receiver {
        ReceiverPath::Main => StatePatch::MainRxFilterShift(shift_hz),
        ReceiverPath::Sub => StatePatch::SubRxFilterShift(shift_hz),
    }
}

fn receiver_mode(state: &RadioState, receiver: ReceiverPath) -> Option<Mode> {
    match receiver {
        ReceiverPath::Main => state.main_rx.mode,
        ReceiverPath::Sub => state.sub_rx.as_ref().and_then(|rx| rx.mode),
    }
}

fn receiver_bandwidth(state: &RadioState, receiver: ReceiverPath) -> Option<u16> {
    match receiver {
        ReceiverPath::Main => state.main_rx.filter.bandwidth_hz,
        ReceiverPath::Sub => state.sub_rx.as_ref().and_then(|rx| rx.filter.bandwidth_hz),
    }
}

fn receiver_shift(state: &RadioState, receiver: ReceiverPath) -> Option<i16> {
    match receiver {
        ReceiverPath::Main => state.main_rx.filter.shift_hz,
        ReceiverPath::Sub => state.sub_rx.as_ref().and_then(|rx| rx.filter.shift_hz),
    }
}

fn signed_parts(value: i16) -> (char, u16) {
    let sign = if value < 0 { '-' } else { '+' };
    (sign, value.unsigned_abs())
}

fn signed_parts_space_positive(value: i16) -> (char, u16) {
    let sign = if value < 0 { '-' } else { ' ' };
    (sign, value.unsigned_abs())
}

fn encode_yaesu_bandwidth_payload(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    id: u8,
) -> String {
    match profile.id() {
        "yaesu-ftdx101" => format!("SH{}0{id:02};", yaesu_target(profile, receiver)),
        "yaesu-ftdx10" | "yaesu-ft710" => format!("SH00{id:02};"),
        "yaesu-ft891" => format!("SH01{id:02};"),
        "yaesu-ft991" => format!("SH0{id:02};"),
        _ => format!("SH{}0{id:02};", yaesu_target(profile, receiver)),
    }
}

fn encode_yaesu_shift_payload(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    shift_hz: i16,
) -> String {
    let (sign, abs) = signed_parts(shift_hz);
    match profile.id() {
        "yaesu-ftdx101" => format!("IS{}0{sign}{abs:04};", yaesu_target(profile, receiver)),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft991" => format!("IS00{sign}{abs:04};"),
        "yaesu-ft891" => format!("IS01{sign}{abs:04};"),
        _ => format!("IS{}0{sign}{abs:04};", yaesu_target(profile, receiver)),
    }
}

fn decode_yaesu_is_parts<'a>(
    profile: &KenwoodAsciiProfile,
    payload: &'a str,
) -> Result<(ReceiverPath, &'a str)> {
    match profile.id() {
        "yaesu-ftdx101" => {
            if payload.len() != 7 {
                return Err(RadioError::Decode {
                    command: "IS",
                    message: format!("expected 7-char FTDX-101 IS payload, got {payload:?}"),
                });
            }
            Ok((decode_target(payload.as_bytes()[0])?, &payload[2..]))
        }
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => {
            if payload.len() != 7 {
                return Err(RadioError::Decode {
                    command: "IS",
                    message: format!("expected 7-char Yaesu IS payload, got {payload:?}"),
                });
            }
            Ok((ReceiverPath::Main, &payload[2..]))
        }
        _ => Err(RadioError::Decode {
            command: "IS",
            message: format!("unsupported Yaesu profile {}", profile.id()),
        }),
    }
}

fn decode_yaesu_sh_parts(profile: &KenwoodAsciiProfile, payload: &str) -> Result<(ReceiverPath, u8)> {
    match profile.id() {
        "yaesu-ftdx101" => {
            if payload.len() != 4 {
                return Err(RadioError::Decode {
                    command: "SH",
                    message: format!("expected 4-char FTDX-101 SH payload, got {payload:?}"),
                });
            }
            Ok((decode_target(payload.as_bytes()[0])?, parse_u8("SH", &payload[2..])?))
        }
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => {
            if payload.len() < 3 {
                return Err(RadioError::Decode {
                    command: "SH",
                    message: format!("expected Yaesu SH payload, got {payload:?}"),
                });
            }
            Ok((ReceiverPath::Main, parse_u8("SH", &payload[payload.len() - 2..])?))
        }
        _ => Err(RadioError::Decode {
            command: "SH",
            message: format!("unsupported Yaesu profile {}", profile.id()),
        }),
    }
}

fn reconstruct_from_single_edge(
    state: &RadioState,
    receiver: ReceiverPath,
    neutral_center_hz: i16,
    high: Option<u16>,
    low: Option<u16>,
    counterpart_candidates: &[u16],
) -> (u16, i16) {
    let current_bw = receiver_bandwidth(state, receiver);
    let current_shift = receiver_shift(state, receiver);

    let counterpart = if let (Some(bw), Some(shift)) = (current_bw, current_shift) {
        let mid = neutral_center_hz as i32 + shift as i32;
        let half = bw as i32 / 2;
        if high.is_some() {
            (mid - half).max(0) as u16
        } else {
            (mid + half).max(0) as u16
        }
    } else {
        counterpart_candidates
            .iter()
            .copied()
            .min_by_key(|candidate| {
                if let Some(high) = high {
                    ((high as i32 + *candidate as i32) / 2 - neutral_center_hz as i32).abs()
                } else if let Some(low) = low {
                    ((*candidate as i32 + low as i32) / 2 - neutral_center_hz as i32).abs()
                } else {
                    0
                }
            })
            .unwrap_or(0)
    };

    let (h, l) = match (high, low) {
        (Some(h), Some(l)) => (h, l),
        (Some(h), None) => (h, counterpart),
        (None, Some(l)) => (counterpart, l),
        (None, None) => (counterpart, counterpart),
    };

    let bandwidth = h.saturating_sub(l);
    let center = ((h as i32 + l as i32) / 2) as i16;
    let shift = center - neutral_center_hz;

    (bandwidth, shift)
}

fn uses_direct_fw_for_mode(
    profile: &KenwoodAsciiProfile,
    state: &RadioState,
    receiver: ReceiverPath,
) -> bool {
    matches!(
        profile.id(),
        "kenwood-ts590" | "kenwood-ts2000" | "kenwood-ts480"
    ) && matches!(
        receiver_mode(state, receiver),
        Some(Mode::Cw | Mode::CwReverse | Mode::Rtty | Mode::RttyReverse)
    )
}

fn uses_direct_is_for_mode(
    profile: &KenwoodAsciiProfile,
    state: &RadioState,
    receiver: ReceiverPath,
) -> bool {
    uses_direct_fw_for_mode(profile, state, receiver)
}

fn is_elecraft_bw_is(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "elecraft-k4" | "elecraft-k3")
}

fn is_unsupported_filter_profile(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "kenwood-ts570" | "kenwood-ts870" | "kenwood-if232"
    )
}

fn is_shift_unsupported_profile(profile: &KenwoodAsciiProfile) -> bool {
    is_unsupported_filter_profile(profile) || profile.id() == "elecraft-k2"
}

fn supports_hi_lo(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "kenwood-ts590" | "kenwood-ts2000" | "kenwood-ts480" | "kenwood-ts890" | "kenwood-ts990"
    )
}

fn is_yaesu(profile: &KenwoodAsciiProfile) -> bool {
    profile.id().starts_with("yaesu-")
}

fn yaesu_target(profile: &KenwoodAsciiProfile, receiver: ReceiverPath) -> char {
    if profile.id() == "yaesu-ftdx101" {
        match receiver {
            ReceiverPath::Main => '0',
            ReceiverPath::Sub => '1',
        }
    } else {
        '0'
    }
}

fn is_ft891_or_ft991(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "yaesu-ft891" | "yaesu-ft991")
}

fn decode_yaesu_bandwidth(mode: Mode, id: u8) -> u16 {
    let family = yaesu_family(mode);
    let entries = yaesu_table_entries(family);
    if id == 0 {
        return entries.first().map(|entry| entry.value_hz).unwrap_or(2_400);
    }
    entries
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.value_hz)
        .unwrap_or_else(|| entries.last().map(|entry| entry.value_hz).unwrap_or(2_400))
}

fn decode_yaesu_891_991_bandwidth(mode: Mode, id: u8, current: u16) -> Result<u16> {
    let family = yaesu_891_991_family(mode);
    let narrow = yaesu_891_991_entries(family, true);
    let wide = yaesu_891_991_entries(family, false);

    let narrow_value = narrow
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.value_hz);
    let wide_value = wide
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.value_hz);

    match (narrow_value, wide_value) {
        (Some(value), None) | (None, Some(value)) => Ok(value),
        (Some(a), Some(b)) => {
            let da = (a as i32 - current as i32).abs();
            let db = (b as i32 - current as i32).abs();
            if da <= db {
                Ok(a)
            } else {
                Ok(b)
            }
        }
        _ => Err(RadioError::Decode {
            command: "SH",
            message: format!("unknown SH id {id} for FT-891/FT-991"),
        }),
    }
}

fn select_yaesu_bandwidth(mode: Mode, requested_hz: u16) -> TableSelection {
    let family = yaesu_family(mode);
    select_upward(requested_hz, yaesu_table_entries(family))
}

fn select_yaesu_891_991_bandwidth(
    mode: Mode,
    requested_hz: u16,
) -> Result<TableSelectionWithWidth> {
    let family = yaesu_891_991_family(mode);
    let narrow = select_upward(requested_hz, yaesu_891_991_entries(family, true));
    let wide = select_upward(requested_hz, yaesu_891_991_entries(family, false));

    let pick_narrow = if narrow.value_hz >= requested_hz && wide.value_hz >= requested_hz {
        narrow.value_hz <= wide.value_hz
    } else if narrow.value_hz >= requested_hz {
        true
    } else if wide.value_hz >= requested_hz {
        false
    } else {
        let dn = (narrow.value_hz as i32 - requested_hz as i32).abs();
        let dw = (wide.value_hz as i32 - requested_hz as i32).abs();
        dn <= dw
    };

    let selected = if pick_narrow { narrow } else { wide };
    Ok(TableSelectionWithWidth {
        id: selected.id,
        value_hz: selected.value_hz,
        narrow: pick_narrow,
    })
}

fn select_upward(requested_hz: u16, entries: &'static [TableSelection]) -> TableSelection {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| entry.value_hz);

    sorted
        .iter()
        .find(|entry| entry.value_hz >= requested_hz)
        .copied()
        .unwrap_or_else(|| *sorted.last().expect("tables are non-empty"))
}

fn yaesu_family(mode: Mode) -> YaesuFamily {
    match mode {
        Mode::Cw | Mode::CwReverse => YaesuFamily::Cw,
        Mode::Rtty | Mode::RttyReverse => YaesuFamily::Fsk,
        Mode::Psk => YaesuFamily::Psk,
        _ => YaesuFamily::Ssb,
    }
}

fn yaesu_891_991_family(mode: Mode) -> Yaesu891991Family {
    match mode {
        Mode::Cw | Mode::CwReverse => Yaesu891991Family::Cw,
        Mode::Rtty | Mode::RttyReverse | Mode::Psk => Yaesu891991Family::FskPsk,
        _ => Yaesu891991Family::Ssb,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YaesuFamily {
    Ssb,
    Cw,
    Fsk,
    Psk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Yaesu891991Family {
    Ssb,
    Cw,
    FskPsk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TableSelection {
    id: u8,
    value_hz: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TableSelectionWithWidth {
    id: u8,
    value_hz: u16,
    narrow: bool,
}

const YAESU_TABLE_SSB: &[TableSelection] = &[
    TableSelection {
        id: 1,
        value_hz: 300,
    },
    TableSelection {
        id: 2,
        value_hz: 400,
    },
    TableSelection {
        id: 3,
        value_hz: 600,
    },
    TableSelection {
        id: 4,
        value_hz: 850,
    },
    TableSelection {
        id: 5,
        value_hz: 850,
    },
    TableSelection {
        id: 6,
        value_hz: 1200,
    },
    TableSelection {
        id: 7,
        value_hz: 1500,
    },
    TableSelection {
        id: 8,
        value_hz: 1650,
    },
    TableSelection {
        id: 9,
        value_hz: 1800,
    },
    TableSelection {
        id: 10,
        value_hz: 1950,
    },
    TableSelection {
        id: 11,
        value_hz: 2100,
    },
    TableSelection {
        id: 12,
        value_hz: 2200,
    },
    TableSelection {
        id: 13,
        value_hz: 2300,
    },
    TableSelection {
        id: 14,
        value_hz: 2400,
    },
    TableSelection {
        id: 15,
        value_hz: 2500,
    },
    TableSelection {
        id: 16,
        value_hz: 2600,
    },
    TableSelection {
        id: 17,
        value_hz: 2700,
    },
    TableSelection {
        id: 18,
        value_hz: 2800,
    },
    TableSelection {
        id: 19,
        value_hz: 2900,
    },
    TableSelection {
        id: 20,
        value_hz: 3000,
    },
    TableSelection {
        id: 21,
        value_hz: 3200,
    },
];

const YAESU_TABLE_CW: &[TableSelection] = &[
    TableSelection {
        id: 1,
        value_hz: 50,
    },
    TableSelection {
        id: 2,
        value_hz: 100,
    },
    TableSelection {
        id: 3,
        value_hz: 150,
    },
    TableSelection {
        id: 4,
        value_hz: 200,
    },
    TableSelection {
        id: 5,
        value_hz: 250,
    },
    TableSelection {
        id: 6,
        value_hz: 300,
    },
    TableSelection {
        id: 7,
        value_hz: 350,
    },
    TableSelection {
        id: 8,
        value_hz: 400,
    },
    TableSelection {
        id: 9,
        value_hz: 450,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
    TableSelection {
        id: 11,
        value_hz: 600,
    },
    TableSelection {
        id: 12,
        value_hz: 800,
    },
    TableSelection {
        id: 13,
        value_hz: 1200,
    },
    TableSelection {
        id: 14,
        value_hz: 1400,
    },
    TableSelection {
        id: 15,
        value_hz: 1700,
    },
    TableSelection {
        id: 16,
        value_hz: 2000,
    },
    TableSelection {
        id: 17,
        value_hz: 2400,
    },
    TableSelection {
        id: 18,
        value_hz: 3000,
    },
];

const YAESU_TABLE_FSK: &[TableSelection] = &[
    TableSelection {
        id: 1,
        value_hz: 50,
    },
    TableSelection {
        id: 2,
        value_hz: 100,
    },
    TableSelection {
        id: 3,
        value_hz: 150,
    },
    TableSelection {
        id: 4,
        value_hz: 200,
    },
    TableSelection {
        id: 5,
        value_hz: 250,
    },
    TableSelection {
        id: 6,
        value_hz: 300,
    },
    TableSelection {
        id: 7,
        value_hz: 350,
    },
    TableSelection {
        id: 8,
        value_hz: 400,
    },
    TableSelection {
        id: 9,
        value_hz: 450,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
    TableSelection {
        id: 11,
        value_hz: 600,
    },
    TableSelection {
        id: 12,
        value_hz: 800,
    },
    TableSelection {
        id: 13,
        value_hz: 1200,
    },
    TableSelection {
        id: 14,
        value_hz: 1400,
    },
    TableSelection {
        id: 15,
        value_hz: 1700,
    },
    TableSelection {
        id: 16,
        value_hz: 2000,
    },
    TableSelection {
        id: 17,
        value_hz: 2400,
    },
    TableSelection {
        id: 18,
        value_hz: 3000,
    },
];

const YAESU_TABLE_PSK: &[TableSelection] = &[
    TableSelection {
        id: 1,
        value_hz: 50,
    },
    TableSelection {
        id: 2,
        value_hz: 100,
    },
    TableSelection {
        id: 3,
        value_hz: 150,
    },
    TableSelection {
        id: 4,
        value_hz: 200,
    },
    TableSelection {
        id: 5,
        value_hz: 250,
    },
    TableSelection {
        id: 6,
        value_hz: 300,
    },
    TableSelection {
        id: 7,
        value_hz: 350,
    },
    TableSelection {
        id: 8,
        value_hz: 400,
    },
    TableSelection {
        id: 9,
        value_hz: 450,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
    TableSelection {
        id: 11,
        value_hz: 600,
    },
    TableSelection {
        id: 12,
        value_hz: 800,
    },
    TableSelection {
        id: 13,
        value_hz: 1200,
    },
    TableSelection {
        id: 14,
        value_hz: 1400,
    },
    TableSelection {
        id: 15,
        value_hz: 1700,
    },
    TableSelection {
        id: 16,
        value_hz: 2000,
    },
    TableSelection {
        id: 17,
        value_hz: 2400,
    },
    TableSelection {
        id: 18,
        value_hz: 3000,
    },
];

fn yaesu_table_entries(family: YaesuFamily) -> &'static [TableSelection] {
    match family {
        YaesuFamily::Ssb => YAESU_TABLE_SSB,
        YaesuFamily::Cw => YAESU_TABLE_CW,
        YaesuFamily::Fsk => YAESU_TABLE_FSK,
        YaesuFamily::Psk => YAESU_TABLE_PSK,
    }
}

const YAESU_891_991_SSB_NARROW: &[TableSelection] = &[
    TableSelection {
        id: 0,
        value_hz: 1500,
    },
    TableSelection {
        id: 1,
        value_hz: 200,
    },
    TableSelection {
        id: 2,
        value_hz: 400,
    },
    TableSelection {
        id: 3,
        value_hz: 600,
    },
    TableSelection {
        id: 4,
        value_hz: 850,
    },
    TableSelection {
        id: 5,
        value_hz: 1100,
    },
    TableSelection {
        id: 6,
        value_hz: 1350,
    },
    TableSelection {
        id: 7,
        value_hz: 1500,
    },
    TableSelection {
        id: 8,
        value_hz: 1650,
    },
    TableSelection {
        id: 9,
        value_hz: 1800,
    },
];

const YAESU_891_991_SSB_WIDE: &[TableSelection] = &[
    TableSelection {
        id: 0,
        value_hz: 2400,
    },
    TableSelection {
        id: 10,
        value_hz: 1950,
    },
    TableSelection {
        id: 11,
        value_hz: 2100,
    },
    TableSelection {
        id: 12,
        value_hz: 2200,
    },
    TableSelection {
        id: 13,
        value_hz: 2300,
    },
    TableSelection {
        id: 14,
        value_hz: 2400,
    },
    TableSelection {
        id: 15,
        value_hz: 2500,
    },
    TableSelection {
        id: 16,
        value_hz: 2600,
    },
    TableSelection {
        id: 17,
        value_hz: 2700,
    },
    TableSelection {
        id: 18,
        value_hz: 2800,
    },
    TableSelection {
        id: 19,
        value_hz: 2900,
    },
    TableSelection {
        id: 20,
        value_hz: 3000,
    },
    TableSelection {
        id: 21,
        value_hz: 3200,
    },
];

const YAESU_891_991_CW_NARROW: &[TableSelection] = &[
    TableSelection {
        id: 0,
        value_hz: 500,
    },
    TableSelection {
        id: 1,
        value_hz: 50,
    },
    TableSelection {
        id: 2,
        value_hz: 100,
    },
    TableSelection {
        id: 3,
        value_hz: 150,
    },
    TableSelection {
        id: 4,
        value_hz: 200,
    },
    TableSelection {
        id: 5,
        value_hz: 250,
    },
    TableSelection {
        id: 6,
        value_hz: 300,
    },
    TableSelection {
        id: 7,
        value_hz: 350,
    },
    TableSelection {
        id: 8,
        value_hz: 400,
    },
    TableSelection {
        id: 9,
        value_hz: 450,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
];

const YAESU_891_991_CW_WIDE: &[TableSelection] = &[
    TableSelection {
        id: 0,
        value_hz: 2400,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
    TableSelection {
        id: 11,
        value_hz: 800,
    },
    TableSelection {
        id: 12,
        value_hz: 1200,
    },
    TableSelection {
        id: 13,
        value_hz: 1400,
    },
    TableSelection {
        id: 14,
        value_hz: 1700,
    },
    TableSelection {
        id: 15,
        value_hz: 2000,
    },
    TableSelection {
        id: 16,
        value_hz: 2400,
    },
    TableSelection {
        id: 17,
        value_hz: 3000,
    },
];

const YAESU_891_991_FSK_PSK_NARROW: &[TableSelection] = &[
    TableSelection {
        id: 0,
        value_hz: 300,
    },
    TableSelection {
        id: 1,
        value_hz: 50,
    },
    TableSelection {
        id: 2,
        value_hz: 100,
    },
    TableSelection {
        id: 3,
        value_hz: 150,
    },
    TableSelection {
        id: 4,
        value_hz: 200,
    },
    TableSelection {
        id: 5,
        value_hz: 250,
    },
    TableSelection {
        id: 6,
        value_hz: 300,
    },
    TableSelection {
        id: 7,
        value_hz: 350,
    },
    TableSelection {
        id: 8,
        value_hz: 400,
    },
    TableSelection {
        id: 9,
        value_hz: 450,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
];

const YAESU_891_991_FSK_PSK_WIDE: &[TableSelection] = &[
    TableSelection {
        id: 0,
        value_hz: 500,
    },
    TableSelection {
        id: 10,
        value_hz: 500,
    },
    TableSelection {
        id: 11,
        value_hz: 800,
    },
    TableSelection {
        id: 12,
        value_hz: 1200,
    },
    TableSelection {
        id: 13,
        value_hz: 1400,
    },
    TableSelection {
        id: 14,
        value_hz: 1700,
    },
    TableSelection {
        id: 15,
        value_hz: 2000,
    },
    TableSelection {
        id: 16,
        value_hz: 2400,
    },
    TableSelection {
        id: 17,
        value_hz: 3000,
    },
];

fn yaesu_891_991_entries(family: Yaesu891991Family, narrow: bool) -> &'static [TableSelection] {
    match (family, narrow) {
        (Yaesu891991Family::Ssb, true) => YAESU_891_991_SSB_NARROW,
        (Yaesu891991Family::Ssb, false) => YAESU_891_991_SSB_WIDE,
        (Yaesu891991Family::Cw, true) => YAESU_891_991_CW_NARROW,
        (Yaesu891991Family::Cw, false) => YAESU_891_991_CW_WIDE,
        (Yaesu891991Family::FskPsk, true) => YAESU_891_991_FSK_PSK_NARROW,
        (Yaesu891991Family::FskPsk, false) => YAESU_891_991_FSK_PSK_WIDE,
    }
}

#[derive(Debug, Clone, Copy)]
struct HiLoTable {
    high_values: &'static [u16],
    low_values: &'static [u16],
    neutral_center_hz: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HiLoCandidate {
    high_id: u16,
    low_id: u16,
    bandwidth_hz: u16,
    shift_hz: i16,
}

impl HiLoTable {
    fn all_candidates(self) -> Vec<HiLoCandidate> {
        let mut out = Vec::new();
        for (high_id, high) in self.high_values.iter().copied().enumerate() {
            for (low_id, low) in self.low_values.iter().copied().enumerate() {
                if high < low {
                    continue;
                }

                let bandwidth_hz = high - low;
                let center = ((high as i32 + low as i32) / 2) as i16;
                let shift_hz = center - self.neutral_center_hz;

                out.push(HiLoCandidate {
                    high_id: high_id as u16,
                    low_id: low_id as u16,
                    bandwidth_hz,
                    shift_hz,
                });
            }
        }
        out
    }

    fn best_for_bandwidth(self, requested_bw: u16, desired_shift: i16) -> Option<HiLoCandidate> {
        let candidates = self.all_candidates();

        let mut above: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|candidate| candidate.bandwidth_hz >= requested_bw)
            .collect();

        if !above.is_empty() {
            above.sort_by_key(|candidate| {
                (
                    candidate.bandwidth_hz,
                    (candidate.shift_hz as i32 - desired_shift as i32).abs(),
                    candidate.shift_hz.unsigned_abs(),
                )
            });
            return above.first().copied();
        }

        let mut all = candidates;
        all.sort_by_key(|candidate| {
            (
                std::cmp::Reverse(candidate.bandwidth_hz),
                (candidate.shift_hz as i32 - desired_shift as i32).abs(),
                candidate.shift_hz.unsigned_abs(),
            )
        });
        all.first().copied()
    }

    fn best_for_shift(self, desired_bw: u16, requested_shift: i16) -> Option<HiLoCandidate> {
        let mut candidates = self.all_candidates();
        candidates.sort_by_key(|candidate| {
            (
                (candidate.shift_hz as i32 - requested_shift as i32).abs(),
                candidate.shift_hz.unsigned_abs(),
                (candidate.bandwidth_hz as i32 - desired_bw as i32).abs(),
            )
        });
        candidates.first().copied()
    }
}

fn hi_lo_table(profile: &KenwoodAsciiProfile, mode: Mode) -> Result<HiLoTable> {
    let family = hi_lo_family(mode);
    match profile.id() {
        "kenwood-ts590" | "kenwood-ts2000" => match family {
            HiLoFamily::Am => Ok(HiLoTable {
                high_values: TS590_2000_AM_HIGH,
                low_values: TS590_2000_AM_LOW,
                neutral_center_hz: 1_250,
            }),
            _ => Ok(HiLoTable {
                high_values: TS590_2000_SSB_HIGH,
                low_values: TS590_2000_SSB_LOW,
                neutral_center_hz: 1_500,
            }),
        },
        "kenwood-ts480" => match family {
            HiLoFamily::Am => Ok(HiLoTable {
                high_values: TS480_AM_HIGH,
                low_values: TS480_AM_LOW,
                neutral_center_hz: 1_250,
            }),
            _ => Ok(HiLoTable {
                high_values: TS480_SSB_HIGH,
                low_values: TS480_SSB_LOW,
                neutral_center_hz: 1_500,
            }),
        },
        "kenwood-ts890" | "kenwood-ts990" => match family {
            HiLoFamily::Am => Ok(HiLoTable {
                high_values: TS890_990_AM_HIGH,
                low_values: TS890_990_AM_LOW,
                neutral_center_hz: 1_250,
            }),
            HiLoFamily::Fm => Ok(HiLoTable {
                high_values: TS890_990_FM_HIGH,
                low_values: TS890_990_FM_LOW,
                neutral_center_hz: 1_500,
            }),
            _ => Ok(HiLoTable {
                high_values: TS890_990_SSB_HIGH,
                low_values: TS890_990_SSB_LOW,
                neutral_center_hz: 1_500,
            }),
        },
        _ => Err(RadioError::UnsupportedCapability {
            capability: "receiver.filter_bandwidth",
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HiLoFamily {
    Ssb,
    Am,
    Fm,
}

fn hi_lo_family(mode: Mode) -> HiLoFamily {
    match mode {
        Mode::Fm | Mode::DataFm => HiLoFamily::Fm,
        Mode::Am | Mode::DataAm => HiLoFamily::Am,
        _ => HiLoFamily::Ssb,
    }
}

const TS590_2000_SSB_HIGH: &[u16] = &[
    1000, 1200, 1400, 1600, 1800, 2000, 2200, 2400, 2600, 2800, 3000, 3400, 4000, 5000,
];
const TS590_2000_SSB_LOW: &[u16] = &[0, 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];
const TS590_2000_AM_HIGH: &[u16] = &[2500, 3000, 4000, 5000];
const TS590_2000_AM_LOW: &[u16] = &[0, 100, 200, 300];

const TS480_SSB_HIGH: &[u16] = TS590_2000_SSB_HIGH;
const TS480_SSB_LOW: &[u16] = TS590_2000_SSB_LOW;
const TS480_AM_HIGH: &[u16] = TS590_2000_AM_HIGH;
const TS480_AM_LOW: &[u16] = TS590_2000_AM_LOW;

const TS890_990_SSB_HIGH: &[u16] = &[
    600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400, 1500, 1600, 1700, 1800, 1900, 2000, 2100,
    2200, 2300, 2400, 2500, 2600, 2700, 2800, 2900, 3000, 3400, 4000, 5000,
];
const TS890_990_AM_HIGH: &[u16] = &[
    2000, 2100, 2200, 2300, 2400, 2500, 2600, 2700, 2800, 2900, 3000, 3500, 4000, 5000,
];
const TS890_990_FM_HIGH: &[u16] = &[
    1000, 1100, 1200, 1300, 1400, 1500, 1600, 1700, 1800, 1900, 2000, 2100, 2200, 2300, 2400, 2500,
    2600, 2700, 2800, 2900, 3000, 3400, 4000, 5000,
];

const TS890_990_SSB_LOW: &[u16] = &[
    0, 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400, 1500, 1600,
    1700, 1800, 1900, 2000,
];
const TS890_990_AM_LOW: &[u16] = &[0, 100, 200, 300];
const TS890_990_FM_LOW: &[u16] = &[0, 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];

trait FrameHint {
    fn command_static_hint(&self) -> &'static str;
}

impl FrameHint for AsciiFrame {
    fn command_static_hint(&self) -> &'static str {
        match self.command() {
            "FW" => "FW",
            "BW" | "BW$" => "BW",
            "IS" | "IS$" => "IS",
            "SH" => "SH",
            "SL" => "SL",
            _ => "filter",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::kenwood_ascii::profile_by_id;

    #[test]
    fn elecraft_bw_and_is_encode_and_decode() {
        let k4 = profile_by_id("elecraft-k4").unwrap();
        let state = RadioState::default();

        let bw = encode(
            k4,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Sub,
                bandwidth_hz: 2_450,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames[0].as_str(), "BW$245;");

        let is = encode(
            k4,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Sub,
                shift_hz: -125,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(is.frames[0].as_str(), "IS$-0012;");

        let is_main = encode(
            k4,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(is_main.frames[0].as_str(), "IS 0025;");

        let decoded_bw = decode(k4, &AsciiFrame::new("BW$240;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded_bw.patches,
            vec![StatePatch::SubRxFilterBandwidth(2_400)]
        );

        let decoded_is = decode(k4, &AsciiFrame::new("IS$ 0025;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(decoded_is.patches, vec![StatePatch::SubRxFilterShift(250)]);
    }

    #[test]
    fn k3_shift_stays_in_hz_units() {
        let k3 = profile_by_id("elecraft-k3").unwrap();
        let state = RadioState::default();

        let is = encode(
            k3,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(is.frames[0].as_str(), "IS 0250;");

        let decoded = decode(k3, &AsciiFrame::new("IS$ 0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(decoded.patches, vec![StatePatch::SubRxFilterShift(250)]);
    }

    #[test]
    fn yaesu_filter_commands_encode_with_model_specific_formats() {
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::Usb);
        state.sub_rx = Some(crate::state::ReceiverState::default());
        state.sub_rx.as_mut().unwrap().mode = Some(Mode::Usb);

        let ftdx101 = profile_by_id("yaesu-ftdx101").unwrap();
        let bw = encode(
            ftdx101,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_305,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames[0].as_str(), "SH0014;");
        let bw_sub = encode(
            ftdx101,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Sub,
                bandwidth_hz: 2_305,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw_sub.frames[0].as_str(), "SH1014;");
        let sh = encode(
            ftdx101,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: -250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh.frames[0].as_str(), "IS00-0250;");
        let sh_sub = encode(
            ftdx101,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Sub,
                shift_hz: 250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh_sub.frames[0].as_str(), "IS10+0250;");

        let ftdx10 = profile_by_id("yaesu-ftdx10").unwrap();
        let bw = encode(
            ftdx10,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_305,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames[0].as_str(), "SH0014;");
        let sh = encode(
            ftdx10,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh.frames[0].as_str(), "IS00+0250;");

        let ft710 = profile_by_id("yaesu-ft710").unwrap();
        let bw = encode(
            ft710,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_305,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames[0].as_str(), "SH0014;");
        let sh = encode(
            ft710,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh.frames[0].as_str(), "IS00+0250;");

        let ft891 = profile_by_id("yaesu-ft891").unwrap();
        let bw = encode(
            ft891,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_600,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames[0].as_str(), "SH0116;");
        let sh = encode(
            ft891,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: -250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh.frames[0].as_str(), "IS01-0250;");

        let ft991 = profile_by_id("yaesu-ft991").unwrap();
        let bw = encode(
            ft991,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_600,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames[0].as_str(), "SH016;");
        let sh = encode(
            ft991,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 250,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh.frames[0].as_str(), "IS00+0250;");
    }

    #[test]
    fn kenwood_hilo_conversion_for_bandwidth_and_shift() {
        let ts890 = profile_by_id("kenwood-ts890").unwrap();
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::Usb);
        state.main_rx.filter.shift_hz = Some(0);

        let bw = encode(
            ts890,
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_400,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bw.frames.len(), 2);
        assert!(bw.frames[0].as_str().starts_with("SH0"));
        assert!(bw.frames[1].as_str().starts_with("SL0"));

        state.main_rx.filter.bandwidth_hz = Some(2_400);
        let sh = encode(
            ts890,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 150,
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sh.frames.len(), 2);
    }

    #[test]
    fn k2_filter_shift_is_unsupported() {
        let k2 = profile_by_id("elecraft-k2").unwrap();
        let err = encode(
            k2,
            &RadioCommand::SetReceiverFilterShift {
                receiver: ReceiverPath::Main,
                shift_hz: 50,
            },
            &RadioState::default(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            RadioError::UnsupportedCapability {
                capability: "receiver.filter_shift"
            }
        ));
    }

    #[test]
    fn query_semantics_expand_to_profile_specific_filter_frames() {
        let ts990 = profile_by_id("kenwood-ts990").unwrap();
        let q = encode_query(ts990, "filter-hi-lo-sub").unwrap().unwrap();
        assert_eq!(q.frames[0].as_str(), "SH1;");
        assert_eq!(q.frames[1].as_str(), "SL1;");

        let k3 = profile_by_id("elecraft-k3").unwrap();
        let q = encode_query(k3, "filter-state").unwrap().unwrap();
        assert!(q.frames.iter().any(|f| f.as_str() == "BW$;"));
        assert!(q.frames.iter().any(|f| f.as_str() == "IS$;"));

        let ftdx101 = profile_by_id("yaesu-ftdx101").unwrap();
        let q = encode_query(ftdx101, "filter-state").unwrap().unwrap();
        assert_eq!(
            q.frames.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            vec!["SH0;", "SH1;", "IS0;", "IS1;"]
        );

        let ft891 = profile_by_id("yaesu-ft891").unwrap();
        let q = encode_query(ft891, "filter-state").unwrap().unwrap();
        assert_eq!(
            q.frames.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            vec!["SH0;", "IS0;"]
        );

        let ft991 = profile_by_id("yaesu-ft991").unwrap();
        let q = encode_query(ft991, "filter-state").unwrap().unwrap();
        assert_eq!(
            q.frames.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            vec!["SH0;", "IS0;"]
        );
    }

    #[test]
    fn decode_yaesu_and_hilo_frames_to_normalized_patches() {
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::Usb);
        state.main_rx.filter.bandwidth_hz = Some(2_400);
        state.sub_rx = Some(crate::state::ReceiverState::default());
        state.sub_rx.as_mut().unwrap().mode = Some(Mode::Usb);

        let ftdx101 = profile_by_id("yaesu-ftdx101").unwrap();
        let sh = decode(ftdx101, &AsciiFrame::new("SH0013;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(sh.patches, vec![StatePatch::MainRxFilterBandwidth(2_300)]);
        let sh_sub = decode(ftdx101, &AsciiFrame::new("SH1014;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(sh_sub.patches, vec![StatePatch::SubRxFilterBandwidth(2_400)]);
        let is = decode(ftdx101, &AsciiFrame::new("IS00-0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(is.patches, vec![StatePatch::MainRxFilterShift(-250)]);
        let is_sub = decode(ftdx101, &AsciiFrame::new("IS10+0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(is_sub.patches, vec![StatePatch::SubRxFilterShift(250)]);

        let ftdx10 = profile_by_id("yaesu-ftdx10").unwrap();
        let sh = decode(ftdx10, &AsciiFrame::new("SH0013;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(sh.patches, vec![StatePatch::MainRxFilterBandwidth(2_300)]);
        let is = decode(ftdx10, &AsciiFrame::new("IS00+0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(is.patches, vec![StatePatch::MainRxFilterShift(250)]);

        let ft710 = profile_by_id("yaesu-ft710").unwrap();
        let sh = decode(ft710, &AsciiFrame::new("SH0013;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(sh.patches, vec![StatePatch::MainRxFilterBandwidth(2_300)]);
        let is = decode(ft710, &AsciiFrame::new("IS00+0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(is.patches, vec![StatePatch::MainRxFilterShift(250)]);

        let ft891 = profile_by_id("yaesu-ft891").unwrap();
        let sh = decode(ft891, &AsciiFrame::new("SH0116;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(sh.patches, vec![StatePatch::MainRxFilterBandwidth(2_600)]);
        let is = decode(ft891, &AsciiFrame::new("IS01-0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(is.patches, vec![StatePatch::MainRxFilterShift(-250)]);

        let ft991 = profile_by_id("yaesu-ft991").unwrap();
        let sh = decode(ft991, &AsciiFrame::new("SH016;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(sh.patches, vec![StatePatch::MainRxFilterBandwidth(2_600)]);
        let is = decode(ft991, &AsciiFrame::new("IS00+0250;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(is.patches, vec![StatePatch::MainRxFilterShift(250)]);

        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::Usb);
        state.main_rx.filter.bandwidth_hz = Some(2_400);
        state.main_rx.filter.shift_hz = Some(0);

        let sh = decode(ts590, &AsciiFrame::new("SH09;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert!(matches!(
            sh.patches[0],
            StatePatch::MainRxFilterBandwidth(_)
        ));

        let sl = decode(ts590, &AsciiFrame::new("SL03;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert!(matches!(sl.patches[1], StatePatch::MainRxFilterShift(_)));
    }
}
