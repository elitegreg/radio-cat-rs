use super::VfoRouting;
use crate::{
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::StatePatch,
    Mode, RadioState, Result,
};

use super::{DecodedFrame, EncodedCommand};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiProfile, OutgoingStep, ResponseMatcher,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ModeTarget {
    Main,
    Sub,
}

pub fn encode(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
    state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    encode_with_routing(profile, command, state, VfoRouting::for_profile(profile))
}

pub fn encode_with_routing(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetReceiverMode { receiver, mode } => encode_mode_for_target(
            profile,
            receiver_target(*receiver),
            *mode,
            state,
            vfo_routing,
        )
        .map(Some),
        RadioCommand::SetTxMode(mode) => encode_mode_for_target(
            profile,
            receiver_target(vfo_routing.receiver_for_vfo(vfo_routing.tx_vfo())),
            *mode,
            state,
            vfo_routing,
        )
        .map(Some),
        _ => Ok(None),
    }
}

pub fn encode_query(
    _profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    let matcher = match semantic {
        "MD" | "MD0" | "MD1" => ResponseMatcher::Prefix("MD"),
        "MD$" => ResponseMatcher::Prefix("MD$"),
        "DA" => ResponseMatcher::Prefix("DA"),
        "DT" => ResponseMatcher::Prefix("DT"),
        "DT$" => ResponseMatcher::Prefix("DT$"),
        "SF0" | "SF1" => ResponseMatcher::Prefix("SF"),
        "OM0" | "OM1" => ResponseMatcher::Prefix("OM"),
        _ => return Ok(None),
    };

    Ok(Some(EncodedCommand::new(
        vec![AsciiFrame::new(format!("{semantic};"))?],
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
    decode_with_routing(profile, frame, state, VfoRouting::for_profile(profile))
}

pub fn decode_with_routing(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Option<DecodedFrame>> {
    let patches = match frame.command() {
        "MD" => decode_md(profile, frame.payload(), state, vfo_routing)?,
        "MD$" => decode_elecraft_md(
            profile,
            ModeTarget::Sub,
            frame.payload(),
            state,
            vfo_routing,
        )?,
        "DA" => decode_ts590_da(profile, frame.payload(), state, vfo_routing)?,
        "DT" => decode_elecraft_dt(
            profile,
            ModeTarget::Main,
            frame.payload(),
            state,
            vfo_routing,
        )?,
        "DT$" => decode_elecraft_dt(
            profile,
            ModeTarget::Sub,
            frame.payload(),
            state,
            vfo_routing,
        )?,
        "SF" => decode_ts890_sf(profile, frame.payload(), state, vfo_routing)?,
        "OM" => decode_ts990_om(profile, frame.payload(), state, vfo_routing)?,
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

pub(crate) fn decode_kenwood_if_mode(profile: &KenwoodAsciiProfile, code: char) -> Result<Mode> {
    match code {
        '1' => Ok(Mode::Lsb),
        '2' => Ok(Mode::Usb),
        '3' => Ok(Mode::Cw),
        '4' => Ok(Mode::Fm),
        '5' => Ok(Mode::Am),
        '6' => Ok(Mode::Rtty),
        '7' => Ok(Mode::CwReverse),
        '9' => Ok(Mode::RttyReverse),
        'C' if profile.id() == "kenwood-ts590" => Ok(Mode::DataLsb),
        'D' if profile.id() == "kenwood-ts590" => Ok(Mode::DataUsb),
        'E' if profile.id() == "kenwood-ts590" => Ok(Mode::DataFm),
        other => Err(RadioError::Decode {
            command: "IF",
            message: format!(
                "unsupported Kenwood mode code {other:?} for {}",
                profile.id()
            ),
        }),
    }
}

pub(crate) fn decode_yaesu_if_mode(profile: &KenwoodAsciiProfile, code: char) -> Result<Mode> {
    decode_yaesu_code(profile, code, "IF")
}

fn encode_mode_for_target(
    profile: &KenwoodAsciiProfile,
    target: ModeTarget,
    mode: Mode,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<EncodedCommand> {
    let (frames, matcher) = if profile.id() == "kenwood-ts590" {
        encode_ts590(mode)?
    } else if profile.id() == "kenwood-ts890" {
        encode_standard_md(mode)?
    } else if profile.id() == "kenwood-ts990" {
        encode_ts990(target, mode)?
    } else if is_standard_kenwood(profile) {
        encode_standard_md(mode)?
    } else if is_elecraft_family(profile) {
        encode_elecraft_mode(target, mode)?
    } else if profile.id() == "elecraft-k2" {
        encode_k2(mode)?
    } else if is_yaesu(profile) {
        encode_yaesu_mode(profile, target, mode, vfo_routing)?
    } else {
        return Err(RadioError::UnsupportedCapability { capability: "mode" });
    };

    let patches = mode_patches(profile, target, mode, state, vfo_routing);
    if frames.len() == 1 {
        return Ok(EncodedCommand::new(
            frames,
            matcher,
            patches,
            CommandPriority::Normal,
        ));
    }

    let steps = frames
        .into_iter()
        .map(|frame| {
            let expected = match frame.command() {
                "MD" => ResponseMatcher::Prefix("MD"),
                "DA" => ResponseMatcher::Prefix("DA"),
                "MD$" => ResponseMatcher::Prefix("MD$"),
                "DT$" => ResponseMatcher::Prefix("DT$"),
                command => unreachable!("unexpected multi-frame mode command {command}"),
            };
            OutgoingStep::decoded(frame, expected, CommandPriority::Normal)
        })
        .collect();
    Ok(EncodedCommand::with_steps(steps, patches))
}

fn decode_md(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if is_yaesu(profile) {
        return decode_yaesu_md(profile, payload, state, vfo_routing);
    }
    if profile.id() == "kenwood-ts590" {
        return decode_ts590_md(profile, payload, state, vfo_routing);
    }
    if is_elecraft_family(profile) {
        return decode_elecraft_md(profile, ModeTarget::Main, payload, state, vfo_routing);
    }
    if profile.id() == "elecraft-k2" {
        return decode_k2_md(profile, payload, state, vfo_routing);
    }

    let code = single_code("MD", payload)?;
    let mode = decode_standard_kenwood_code(code, "MD")?;
    Ok(mode_patches(
        profile,
        ModeTarget::Main,
        mode,
        state,
        vfo_routing,
    ))
}

fn encode_standard_md(mode: Mode) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    let code = encode_standard_kenwood_code(mode)?;
    Ok((
        vec![AsciiFrame::new(format!("MD{code};"))?],
        ResponseMatcher::Prefix("MD"),
    ))
}

fn encode_ts590(mode: Mode) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    let (md_code, da_flag) = encode_ts590_mode(mode)?;
    let mut frames = vec![AsciiFrame::new(format!("MD{md_code};"))?];
    if let Some(flag) = da_flag {
        frames.push(AsciiFrame::new(format!("DA{};", if flag { 1 } else { 0 }))?);
    }
    Ok((frames, ResponseMatcher::Prefix("MD")))
}

fn encode_ts990(target: ModeTarget, mode: Mode) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    let target_code = match target {
        ModeTarget::Main => '0',
        ModeTarget::Sub => '1',
    };
    let mode_code = encode_ts990_code(mode)?;
    Ok((
        vec![AsciiFrame::new(format!("OM{target_code}{mode_code};"))?],
        ResponseMatcher::Prefix("OM"),
    ))
}

fn encode_elecraft_mode(
    target: ModeTarget,
    mode: Mode,
) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    let suffix = match target {
        ModeTarget::Main => "",
        ModeTarget::Sub => "$",
    };
    let (md_code, dt_code) = encode_elecraft_codes(mode)?;
    let mut frames = vec![AsciiFrame::new(format!("MD{suffix}{md_code};"))?];
    if let Some(dt_code) = dt_code {
        frames.push(AsciiFrame::new(format!("DT{suffix}{dt_code};"))?);
    }
    Ok((
        frames,
        ResponseMatcher::Prefix(if suffix.is_empty() { "MD" } else { "MD$" }),
    ))
}

fn encode_k2(mode: Mode) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    let code = match mode {
        Mode::Lsb => '1',
        Mode::Usb => '2',
        Mode::Cw => '3',
        Mode::Rtty => '6',
        Mode::CwReverse => '7',
        Mode::RttyReverse => '9',
        _ => {
            return Err(RadioError::InvalidValue {
                field: "mode",
                message: format!("mode {mode} is not supported by elecraft-k2"),
            })
        }
    };
    Ok((
        vec![AsciiFrame::new(format!("MD{code};"))?],
        ResponseMatcher::Prefix("MD"),
    ))
}

fn encode_yaesu_mode(
    profile: &KenwoodAsciiProfile,
    target: ModeTarget,
    mode: Mode,
    _vfo_routing: VfoRouting,
) -> Result<(Vec<AsciiFrame>, ResponseMatcher)> {
    let target_code = match profile.id() {
        "yaesu-ftdx101" | "yaesu-ftdx10" | "yaesu-ft710" => {
            if matches!(target, ModeTarget::Main) {
                '0'
            } else {
                '1'
            }
        }
        _ => '0',
    };
    let code = encode_yaesu_code(profile, mode)?;
    Ok((
        vec![AsciiFrame::new(format!("MD{target_code}{code};"))?],
        ResponseMatcher::Prefix("MD"),
    ))
}

fn decode_ts590_md(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    let code = single_code("MD", payload)?;
    let data_flag = current_ts590_data_flag(state.main_rx.mode);
    let mode = compose_ts590_mode(code, data_flag, "MD")?;
    Ok(mode_patches(
        profile,
        ModeTarget::Main,
        mode,
        state,
        vfo_routing,
    ))
}

fn decode_ts590_da(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    let flag = match single_code("DA", payload)? {
        '0' => false,
        '1' => true,
        other => {
            return Err(RadioError::Decode {
                command: "DA",
                message: format!("unsupported DA flag {other:?}"),
            })
        }
    };

    let base = current_ts590_base_code(state.main_rx.mode).ok_or(RadioError::Decode {
        command: "DA",
        message: "cannot compose TS-590 mode without current MD state".to_string(),
    })?;
    let mode = compose_ts590_mode(base, flag, "DA")?;
    Ok(mode_patches(
        profile,
        ModeTarget::Main,
        mode,
        state,
        vfo_routing,
    ))
}

fn decode_ts890_sf(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if payload.len() != 13 {
        return Err(RadioError::Decode {
            command: "SF",
            message: format!("expected 13-character SF payload, got {}", payload.len()),
        });
    }
    let target = match payload.as_bytes()[0] {
        b'0' => ModeTarget::Main,
        b'1' => ModeTarget::Sub,
        other => {
            return Err(RadioError::Decode {
                command: "SF",
                message: format!("unsupported SF target {:?}", other as char),
            })
        }
    };
    let frequency = payload[1..12]
        .parse::<u64>()
        .map_err(|error| RadioError::Decode {
            command: "SF",
            message: error.to_string(),
        })?;
    let mode = decode_ts890_code(payload.as_bytes()[12] as char)?;

    let mut patches = match target {
        ModeTarget::Main => vec![StatePatch::MainRxFrequency(crate::Frequency::from_hz(
            frequency,
        ))],
        ModeTarget::Sub => vec![
            StatePatch::SubRxPresent(true),
            StatePatch::SubRxFrequency(crate::Frequency::from_hz(frequency)),
        ],
    };
    patches.extend(mode_patches(profile, target, mode, state, vfo_routing));
    Ok(patches)
}

fn decode_ts990_om(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if profile.id() != "kenwood-ts990" {
        return Err(RadioError::Decode {
            command: "OM",
            message: format!("OM frames are not valid for {}", profile.id()),
        });
    }
    if payload.len() != 2 {
        return Err(RadioError::Decode {
            command: "OM",
            message: format!("expected 2-character OM payload, got {}", payload.len()),
        });
    }
    let target = match payload.as_bytes()[0] {
        b'0' => ModeTarget::Main,
        b'1' => ModeTarget::Sub,
        other => {
            return Err(RadioError::Decode {
                command: "OM",
                message: format!("unsupported OM target {:?}", other as char),
            })
        }
    };
    let mode = decode_ts990_code(payload.as_bytes()[1] as char)?;
    Ok(mode_patches(profile, target, mode, state, vfo_routing))
}

fn decode_elecraft_md(
    profile: &KenwoodAsciiProfile,
    target: ModeTarget,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if !is_elecraft_family(profile) {
        return Err(RadioError::Decode {
            command: "MD",
            message: format!("MD/MD$ composition is not valid for {}", profile.id()),
        });
    }
    let code = single_code("MD", payload)?;
    let dt_code = current_elecraft_dt_code(target, state);
    let mode = compose_elecraft_mode(code, dt_code, "MD")?;
    Ok(mode_patches(profile, target, mode, state, vfo_routing))
}

fn decode_elecraft_dt(
    profile: &KenwoodAsciiProfile,
    target: ModeTarget,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if !is_elecraft_family(profile) {
        return Err(RadioError::Decode {
            command: "DT",
            message: format!("DT/DT$ composition is not valid for {}", profile.id()),
        });
    }
    let dt_code = single_code("DT", payload)?;
    let md_code = current_elecraft_md_code(target, state).ok_or(RadioError::Decode {
        command: "DT",
        message: "cannot compose Elecraft mode without current MD state".to_string(),
    })?;
    let mode = compose_elecraft_mode(md_code, Some(dt_code), "DT")?;
    Ok(mode_patches(profile, target, mode, state, vfo_routing))
}

fn decode_k2_md(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    let code = single_code("MD", payload)?;
    let mode = match code {
        '1' => Mode::Lsb,
        '2' => Mode::Usb,
        '3' => Mode::Cw,
        '6' => Mode::Rtty,
        '7' => Mode::CwReverse,
        '9' => Mode::RttyReverse,
        other => {
            return Err(RadioError::Decode {
                command: "MD",
                message: format!("unsupported K2 mode code {other:?}"),
            })
        }
    };
    Ok(mode_patches(
        profile,
        ModeTarget::Main,
        mode,
        state,
        vfo_routing,
    ))
}

fn decode_yaesu_md(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    let (target, code) = match payload.len() {
        1 => (ModeTarget::Main, payload.as_bytes()[0] as char),
        2 => {
            let target = match payload.as_bytes()[0] {
                b'0' => ModeTarget::Main,
                b'1' => ModeTarget::Sub,
                other => {
                    return Err(RadioError::Decode {
                        command: "MD",
                        message: format!("unsupported Yaesu mode target {:?}", other as char),
                    })
                }
            };
            (target, payload.as_bytes()[1] as char)
        }
        len => {
            return Err(RadioError::Decode {
                command: "MD",
                message: format!("expected 1 or 2 character MD payload, got {len}"),
            })
        }
    };

    let mode = decode_yaesu_code(profile, code, "MD")?;
    Ok(mode_patches(profile, target, mode, state, vfo_routing))
}

fn mode_patches(
    _profile: &KenwoodAsciiProfile,
    target: ModeTarget,
    mode: Mode,
    _state: &RadioState,
    vfo_routing: VfoRouting,
) -> Vec<StatePatch> {
    let mut patches = vec![match target {
        ModeTarget::Main => StatePatch::MainRxMode(mode),
        ModeTarget::Sub => StatePatch::SubRxMode(mode),
    }];

    if matches!(target, ModeTarget::Sub) {
        patches.insert(0, StatePatch::SubRxPresent(true));
    }

    if vfo_routing.receiver_for_vfo(vfo_routing.tx_vfo())
        == match target {
            ModeTarget::Main => ReceiverPath::Main,
            ModeTarget::Sub => ReceiverPath::Sub,
        }
    {
        patches.push(StatePatch::TxMode(mode));
    }

    patches
}

fn receiver_target(receiver: ReceiverPath) -> ModeTarget {
    match receiver {
        ReceiverPath::Main => ModeTarget::Main,
        ReceiverPath::Sub => ModeTarget::Sub,
    }
}

fn single_code(command: &'static str, payload: &str) -> Result<char> {
    if payload.len() != 1 {
        return Err(RadioError::Decode {
            command,
            message: format!("expected 1 character payload, got {}", payload.len()),
        });
    }
    Ok(payload.as_bytes()[0] as char)
}

fn decode_standard_kenwood_code(code: char, command: &'static str) -> Result<Mode> {
    match code {
        '1' => Ok(Mode::Lsb),
        '2' => Ok(Mode::Usb),
        '3' => Ok(Mode::Cw),
        '4' => Ok(Mode::Fm),
        '5' => Ok(Mode::Am),
        '6' => Ok(Mode::Rtty),
        '7' => Ok(Mode::CwReverse),
        '9' => Ok(Mode::RttyReverse),
        other => Err(RadioError::Decode {
            command,
            message: format!("unsupported Kenwood mode code {other:?}"),
        }),
    }
}

fn encode_standard_kenwood_code(mode: Mode) -> Result<char> {
    match mode {
        Mode::Lsb => Ok('1'),
        Mode::Usb => Ok('2'),
        Mode::Cw => Ok('3'),
        Mode::Fm => Ok('4'),
        Mode::Am => Ok('5'),
        Mode::Rtty => Ok('6'),
        Mode::CwReverse => Ok('7'),
        Mode::RttyReverse => Ok('9'),
        _ => Err(RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not valid for shared Kenwood MD"),
        }),
    }
}

fn encode_ts590_mode(mode: Mode) -> Result<(char, Option<bool>)> {
    match mode {
        Mode::Lsb => Ok(('1', Some(false))),
        Mode::Usb => Ok(('2', Some(false))),
        Mode::Fm => Ok(('4', Some(false))),
        Mode::Am => Ok(('5', Some(false))),
        Mode::DataLsb => Ok(('1', Some(true))),
        Mode::DataUsb => Ok(('2', Some(true))),
        Mode::DataFm => Ok(('4', Some(true))),
        Mode::DataAm => Ok(('5', Some(true))),
        Mode::Cw => Ok(('3', None)),
        Mode::Rtty => Ok(('6', None)),
        Mode::CwReverse => Ok(('7', None)),
        Mode::RttyReverse => Ok(('9', None)),
        _ => Err(RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not supported by kenwood-ts590"),
        }),
    }
}

fn compose_ts590_mode(code: char, data_flag: bool, command: &'static str) -> Result<Mode> {
    match (code, data_flag) {
        ('1', false) => Ok(Mode::Lsb),
        ('2', false) => Ok(Mode::Usb),
        ('4', false) => Ok(Mode::Fm),
        ('5', false) => Ok(Mode::Am),
        ('1', true) => Ok(Mode::DataLsb),
        ('2', true) => Ok(Mode::DataUsb),
        ('4', true) => Ok(Mode::DataFm),
        ('5', true) => Ok(Mode::DataAm),
        ('3', _) => Ok(Mode::Cw),
        ('6', _) => Ok(Mode::Rtty),
        ('7', _) => Ok(Mode::CwReverse),
        ('9', _) => Ok(Mode::RttyReverse),
        other => Err(RadioError::Decode {
            command,
            message: format!("unsupported TS-590 mode tuple {:?}", other),
        }),
    }
}

fn current_ts590_data_flag(mode: Option<Mode>) -> bool {
    matches!(
        mode,
        Some(Mode::DataLsb | Mode::DataUsb | Mode::DataFm | Mode::DataAm)
    )
}

fn current_ts590_base_code(mode: Option<Mode>) -> Option<char> {
    match mode? {
        Mode::Lsb | Mode::DataLsb => Some('1'),
        Mode::Usb | Mode::DataUsb => Some('2'),
        Mode::Fm | Mode::DataFm => Some('4'),
        Mode::Am | Mode::DataAm => Some('5'),
        Mode::Cw => Some('3'),
        Mode::Rtty => Some('6'),
        Mode::CwReverse => Some('7'),
        Mode::RttyReverse => Some('9'),
        _ => None,
    }
}

fn decode_ts890_code(code: char) -> Result<Mode> {
    match code {
        '1' => Ok(Mode::Lsb),
        '2' => Ok(Mode::Usb),
        '3' => Ok(Mode::Cw),
        '4' => Ok(Mode::Fm),
        '5' => Ok(Mode::Am),
        '6' => Ok(Mode::Rtty),
        '7' => Ok(Mode::CwReverse),
        '9' => Ok(Mode::RttyReverse),
        'A' => Ok(Mode::Psk),
        'B' => Ok(Mode::PskReverse),
        'C' => Ok(Mode::DataLsb),
        'D' => Ok(Mode::DataUsb),
        'E' => Ok(Mode::DataFm),
        'F' => Ok(Mode::DataAm),
        other => Err(RadioError::Decode {
            command: "SF",
            message: format!("unsupported TS-890 mode code {other:?}"),
        }),
    }
}

fn decode_ts990_code(code: char) -> Result<Mode> {
    match code {
        '1' => Ok(Mode::Lsb),
        '2' => Ok(Mode::Usb),
        '3' => Ok(Mode::Cw),
        '4' => Ok(Mode::Fm),
        '5' => Ok(Mode::Am),
        '6' => Ok(Mode::Rtty),
        '7' => Ok(Mode::CwReverse),
        '9' => Ok(Mode::RttyReverse),
        'A' => Ok(Mode::Psk),
        'B' => Ok(Mode::PskReverse),
        'C' | 'G' | 'K' => Ok(Mode::DataLsb),
        'D' | 'H' | 'L' => Ok(Mode::DataUsb),
        'E' | 'I' | 'M' => Ok(Mode::DataFm),
        'F' | 'J' | 'N' => Ok(Mode::DataAm),
        other => Err(RadioError::Decode {
            command: "OM",
            message: format!("unsupported TS-990 mode code {other:?}"),
        }),
    }
}

fn encode_ts990_code(mode: Mode) -> Result<char> {
    match mode {
        Mode::Lsb => Ok('1'),
        Mode::Usb => Ok('2'),
        Mode::Cw => Ok('3'),
        Mode::Fm => Ok('4'),
        Mode::Am => Ok('5'),
        Mode::Rtty => Ok('6'),
        Mode::CwReverse => Ok('7'),
        Mode::RttyReverse => Ok('9'),
        Mode::Psk => Ok('A'),
        Mode::PskReverse => Ok('B'),
        Mode::DataLsb => Ok('C'),
        Mode::DataUsb => Ok('D'),
        Mode::DataFm => Ok('E'),
        Mode::DataAm => Ok('F'),
        _ => Err(RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not supported by kenwood-ts990"),
        }),
    }
}

fn encode_elecraft_codes(mode: Mode) -> Result<(char, Option<char>)> {
    match mode {
        Mode::Lsb => Ok(('1', None)),
        Mode::Usb => Ok(('2', None)),
        Mode::Cw => Ok(('3', None)),
        Mode::Fm => Ok(('4', None)),
        Mode::Am => Ok(('5', None)),
        Mode::Rtty => Ok(('6', Some('2'))),
        Mode::CwReverse => Ok(('7', None)),
        Mode::RttyReverse => Ok(('9', Some('2'))),
        Mode::DataUsb => Ok(('6', Some('0'))),
        Mode::DataLsb => Ok(('9', Some('0'))),
        Mode::Psk => Ok(('6', Some('3'))),
        Mode::PskReverse => Ok(('9', Some('3'))),
        _ => Err(RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not supported by Elecraft MD/DT"),
        }),
    }
}

fn compose_elecraft_mode(
    md_code: char,
    dt_code: Option<char>,
    command: &'static str,
) -> Result<Mode> {
    match md_code {
        '1' => Ok(Mode::Lsb),
        '2' => Ok(Mode::Usb),
        '3' => Ok(Mode::Cw),
        '4' => Ok(Mode::Fm),
        '5' => Ok(Mode::Am),
        '7' => Ok(Mode::CwReverse),
        '6' => match dt_code {
            Some('1' | '2') => Ok(Mode::Rtty),
            Some('0') => Ok(Mode::DataUsb),
            Some('3') | None => Ok(Mode::Psk),
            Some(other) => Err(RadioError::Decode {
                command,
                message: format!("unsupported Elecraft DT code {other:?}"),
            }),
        },
        '9' => match dt_code {
            Some('1' | '2') => Ok(Mode::RttyReverse),
            Some('0') => Ok(Mode::DataLsb),
            Some('3') | None => Ok(Mode::PskReverse),
            Some(other) => Err(RadioError::Decode {
                command,
                message: format!("unsupported Elecraft DT code {other:?}"),
            }),
        },
        other => Err(RadioError::Decode {
            command,
            message: format!("unsupported Elecraft MD code {other:?}"),
        }),
    }
}

fn current_elecraft_dt_code(target: ModeTarget, state: &RadioState) -> Option<char> {
    match current_mode(target, state)? {
        Mode::Rtty | Mode::RttyReverse => Some('2'),
        Mode::Psk | Mode::PskReverse => Some('3'),
        Mode::DataUsb | Mode::DataLsb => Some('0'),
        _ => None,
    }
}

fn current_elecraft_md_code(target: ModeTarget, state: &RadioState) -> Option<char> {
    match current_mode(target, state)? {
        Mode::Lsb => Some('1'),
        Mode::Usb => Some('2'),
        Mode::Cw => Some('3'),
        Mode::Fm => Some('4'),
        Mode::Am => Some('5'),
        Mode::CwReverse => Some('7'),
        Mode::Rtty | Mode::Psk | Mode::DataUsb => Some('6'),
        Mode::RttyReverse | Mode::PskReverse | Mode::DataLsb => Some('9'),
        _ => None,
    }
}

fn current_mode(target: ModeTarget, state: &RadioState) -> Option<Mode> {
    match target {
        ModeTarget::Main => state.main_rx.mode,
        ModeTarget::Sub => state.sub_rx.as_ref()?.mode,
    }
}

fn encode_yaesu_code(profile: &KenwoodAsciiProfile, mode: Mode) -> Result<char> {
    match mode {
        Mode::Lsb => Ok('1'),
        Mode::Usb => Ok('2'),
        Mode::Cw => Ok('3'),
        Mode::Fm => Ok('4'),
        Mode::Am => Ok('5'),
        Mode::Rtty => Ok('6'),
        Mode::CwReverse => Ok('7'),
        Mode::DataLsb => Ok('8'),
        Mode::RttyReverse => Ok('9'),
        Mode::DataFm => {
            if profile.id() == "yaesu-ft891" {
                Err(RadioError::InvalidValue {
                    field: "mode",
                    message: format!("mode {mode} is not supported by {}", profile.id()),
                })
            } else {
                Ok('A')
            }
        }
        Mode::DataUsb => Ok('C'),
        Mode::Psk => Ok('E'),
        _ => Err(RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not supported by {}", profile.id()),
        }),
    }
}

fn decode_yaesu_code(
    profile: &KenwoodAsciiProfile,
    code: char,
    command: &'static str,
) -> Result<Mode> {
    match code {
        '1' => Ok(Mode::Lsb),
        '2' => Ok(Mode::Usb),
        '3' => Ok(Mode::Cw),
        '4' => Ok(Mode::Fm),
        '5' => Ok(Mode::Am),
        '6' => Ok(Mode::Rtty),
        '7' => Ok(Mode::CwReverse),
        '8' => Ok(Mode::DataLsb),
        '9' => Ok(Mode::RttyReverse),
        'A' if profile.id() == "yaesu-ft891" => Err(RadioError::Decode {
            command,
            message: "FT-891 does not support mode code 'A'".to_string(),
        }),
        'A' => Ok(Mode::DataFm),
        'B' => Ok(Mode::Fm),
        'C' => Ok(Mode::DataUsb),
        'D' => Ok(Mode::Am),
        'E' if profile.id() == "yaesu-ft891" => Err(RadioError::Decode {
            command,
            message: "FT-891 does not support mode code 'E'".to_string(),
        }),
        'E' => Ok(Mode::Psk),
        'F' if profile.id() == "yaesu-ft891" => Err(RadioError::Decode {
            command,
            message: "FT-891 does not support mode code 'F'".to_string(),
        }),
        'F' => Ok(Mode::DataFm),
        other => Err(RadioError::Decode {
            command,
            message: format!("unsupported Yaesu mode code {other:?} for {}", profile.id()),
        }),
    }
}

fn is_standard_kenwood(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "kenwood-ts2000" | "kenwood-ts480" | "kenwood-ts570" | "kenwood-ts870" | "kenwood-if232"
    )
}

fn is_elecraft_family(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "elecraft-k4" | "elecraft-k3")
}

fn is_yaesu(profile: &KenwoodAsciiProfile) -> bool {
    profile.id().starts_with("yaesu-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::kenwood_ascii::profile_by_id, Frequency, TransmitterState};

    #[test]
    fn ts590_data_mode_encodes_as_md_plus_da() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = RadioState::default();
        let encoded = encode(
            profile,
            &RadioCommand::SetReceiverMode {
                receiver: ReceiverPath::Main,
                mode: Mode::DataUsb,
            },
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "MD2;");
        assert_eq!(encoded.frames[1].as_str(), "DA1;");
    }

    #[test]
    fn ts890_sf_decodes_frequency_and_mode() {
        let profile = profile_by_id("kenwood-ts890").unwrap();
        let mut state = RadioState::default();
        state.main_rx.frequency = Some(Frequency::from_hz(14_074_000));
        state.tx = Some(TransmitterState {
            split: Some(false),
            ..TransmitterState::default()
        });
        let decoded = decode(
            profile,
            &AsciiFrame::new(concat!("SF", "0", "00014074000", "2", ";")).unwrap(),
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::MainRxFrequency(crate::Frequency::from_hz(14_074_000)),
                StatePatch::MainRxMode(Mode::Usb),
                StatePatch::TxMode(Mode::Usb),
            ]
        );
    }

    #[test]
    fn ts990_om_decodes_data_variants_to_normalized_modes() {
        let profile = profile_by_id("kenwood-ts990").unwrap();
        let mut state = RadioState::default();
        state.tx = Some(TransmitterState::default());
        let decoded = decode(profile, &AsciiFrame::new("OM0D;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxMode(Mode::DataUsb)));
        assert!(decoded.patches.contains(&StatePatch::TxMode(Mode::DataUsb)));
    }

    #[test]
    fn elecraft_mode_composes_md_and_dt() {
        let profile = profile_by_id("elecraft-k3").unwrap();
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::DataUsb);

        let md = decode(profile, &AsciiFrame::new("MD6;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert!(md.patches.contains(&StatePatch::MainRxMode(Mode::DataUsb)));

        let dt = decode(profile, &AsciiFrame::new("DT2;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert!(dt.patches.contains(&StatePatch::MainRxMode(Mode::Rtty)));
    }

    #[test]
    fn k2_rejects_unsupported_mode() {
        let profile = profile_by_id("elecraft-k2").unwrap();
        let error = encode(
            profile,
            &RadioCommand::SetReceiverMode {
                receiver: ReceiverPath::Main,
                mode: Mode::Am,
            },
            &RadioState::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RadioError::InvalidValue { field: "mode", .. }
        ));
    }

    #[test]
    fn yaesu_mode_targets_and_ft991_special_case_work() {
        let ftdx101 = profile_by_id("yaesu-ftdx101").unwrap();
        let mut split_state = RadioState::default();
        split_state.tx = Some(TransmitterState {
            split: Some(true),
            ..TransmitterState::default()
        });
        let mut routing = VfoRouting::for_profile(ftdx101);
        routing.set_tx_vfo(super::super::PhysicalVfo::B);
        let encoded = encode_with_routing(
            ftdx101,
            &RadioCommand::SetReceiverMode {
                receiver: ReceiverPath::Sub,
                mode: Mode::DataUsb,
            },
            &split_state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert_eq!(encoded.frames[0].as_str(), "MD1C;");
        assert!(encoded
            .completion_patches
            .contains(&StatePatch::SubRxMode(Mode::DataUsb)));
        assert!(encoded
            .completion_patches
            .contains(&StatePatch::TxMode(Mode::DataUsb)));

        let ft991 = profile_by_id("yaesu-ft991").unwrap();
        let decoded = decode(
            ft991,
            &AsciiFrame::new("MD0E;").unwrap(),
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();
        assert!(decoded.patches.contains(&StatePatch::MainRxMode(Mode::Psk)));
    }

    #[test]
    fn query_encoding_covers_family_specific_mode_frames() {
        let profile = profile_by_id("kenwood-ts990").unwrap();
        let om = encode_query(profile, "OM1").unwrap().unwrap();
        assert_eq!(om.frames[0].as_str(), "OM1;");

        let profile = profile_by_id("elecraft-k3").unwrap();
        let dt = encode_query(profile, "DT$").unwrap().unwrap();
        assert_eq!(dt.frames[0].as_str(), "DT$;");

        let profile = profile_by_id("yaesu-ftdx101").unwrap();
        let md = encode_query(profile, "MD1").unwrap().unwrap();
        assert_eq!(md.frames[0].as_str(), "MD1;");
    }

    #[test]
    fn switched_vfo_routing_maps_mode_commands_and_responses() {
        let profile = profile_by_id("yaesu-ftdx10").unwrap();
        let state = RadioState::default();
        let mut routing = VfoRouting::for_profile(profile);
        super::super::info::decode(
            profile,
            &AsciiFrame::new("VS1;").unwrap(),
            &state,
            &mut routing,
        )
        .unwrap();

        let encoded = encode_with_routing(
            profile,
            &RadioCommand::SetReceiverMode {
                receiver: ReceiverPath::Main,
                mode: Mode::Usb,
            },
            &state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert_eq!(encoded.frames[0].as_str(), "MD02;");

        let decoded =
            decode_with_routing(profile, &AsciiFrame::new("MD02;").unwrap(), &state, routing)
                .unwrap()
                .unwrap();
        assert!(decoded.patches.contains(&StatePatch::MainRxMode(Mode::Usb)));
    }

    #[test]
    fn ts590_da_decode_uses_current_md_context() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::Usb);
        state.main_rx.frequency = Some(Frequency::from_hz(14_074_000));
        let decoded = decode(profile, &AsciiFrame::new("DA1;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxMode(Mode::DataUsb)));
    }
}
