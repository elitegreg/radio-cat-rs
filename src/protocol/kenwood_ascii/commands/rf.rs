use crate::{
    capabilities::Capability,
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::StatePatch,
    LeveledSetting, Result,
};

use super::{DecodedFrame, EncodedCommand, VfoRouting};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiProfile, OutgoingStep, ResponseMatcher,
};

pub fn encode(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
) -> Result<Option<EncodedCommand>> {
    encode_with_routing(profile, command, VfoRouting::for_profile(profile))
}

pub fn encode_with_routing(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
    vfo_routing: VfoRouting,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetReceiverPreamp { receiver, setting } => {
            require_writable(
                receiver_capability(profile, *receiver, Feature::Preamp),
                "receiver.preamp",
            )?;
            Ok(Some(encode_preamp(
                profile,
                *receiver,
                *setting,
                vfo_routing,
            )?))
        }
        RadioCommand::SetReceiverAttenuator { receiver, setting } => {
            require_writable(
                receiver_capability(profile, *receiver, Feature::Attenuator),
                "receiver.attenuator",
            )?;
            Ok(Some(encode_attenuator(
                profile,
                *receiver,
                *setting,
                vfo_routing,
            )?))
        }
        RadioCommand::SetReceiverNoiseBlanker { receiver, setting } => {
            require_writable(
                receiver_capability(profile, *receiver, Feature::NoiseBlanker),
                "receiver.noise_blanker",
            )?;
            Ok(Some(encode_noise_blanker(
                profile,
                *receiver,
                *setting,
                vfo_routing,
            )?))
        }
        RadioCommand::SetReceiverNoiseReduction { receiver, setting } => {
            require_writable(
                receiver_capability(profile, *receiver, Feature::NoiseReduction),
                "receiver.noise_reduction",
            )?;
            Ok(Some(encode_noise_reduction(
                profile,
                *receiver,
                *setting,
                vfo_routing,
            )?))
        }
        RadioCommand::SetReceiverAutoNotch { receiver, enabled } => {
            require_writable(
                receiver_capability(profile, *receiver, Feature::AutoNotch),
                "receiver.auto_notch",
            )?;
            Ok(Some(encode_auto_notch(
                profile,
                *receiver,
                *enabled,
                vfo_routing,
            )?))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    if !is_rf_semantic(semantic) || semantic == "rf-dsp" {
        return Ok(None);
    }

    let matcher = if profile.id() == "kenwood-ts990" && semantic.starts_with("NT") {
        ResponseMatcher::Prefix("NT")
    } else {
        matcher_for_semantic(semantic)
    };
    let frame = AsciiFrame::new(format!("{semantic};"))?;

    // Keep this conservative: if no RF/DSP feature is readable, ignore RF semantic queries.
    let readable = profile.capabilities.main_rx.rf.preamp.can_read()
        || profile.capabilities.main_rx.rf.attenuator.can_read()
        || profile.capabilities.main_rx.rf.noise_blanker.can_read()
        || profile.capabilities.main_rx.rf.noise_reduction.can_read()
        || profile.capabilities.main_rx.rf.auto_notch.can_read();

    if !readable {
        return Ok(None);
    }

    Ok(Some(EncodedCommand::new(
        vec![frame],
        matcher,
        Vec::new(),
        CommandPriority::Normal,
    )))
}

pub fn decode(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<Option<DecodedFrame>> {
    decode_with_routing(profile, frame, &mut VfoRouting::for_profile(profile))
}

pub fn decode_with_routing(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    vfo_routing: &mut VfoRouting,
) -> Result<Option<DecodedFrame>> {
    let patches = match frame.command() {
        "PA" | "PA$" | "PAX" => decode_preamp(profile, frame, *vfo_routing)?,
        "RA" | "RA$" | "RAX" => decode_attenuator(profile, frame, *vfo_routing)?,
        "NB" | "NB$" => decode_noise_blanker(profile, frame, vfo_routing)?,
        "NR" | "NR$" | "NRX" => decode_noise_reduction(profile, frame, *vfo_routing)?,
        "NT" | "NTX" | "NA" | "NA$" | "BC" => decode_auto_notch(profile, frame, *vfo_routing)?,
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

fn encode_preamp(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    setting: LeveledSetting,
    vfo_routing: VfoRouting,
) -> Result<EncodedCommand> {
    let index = normalize_index(setting, 1)?;

    let (frame, max) = match profile.id() {
        "kenwood-ts990" => (format!("PA{}{index};", target_digit(receiver)), 1),
        "elecraft-k4" | "elecraft-k3" => (
            format!(
                "PA{}{index};",
                if matches!(receiver, ReceiverPath::Sub) {
                    "$"
                } else {
                    ""
                }
            ),
            9,
        ),
        "yaesu-ftdx101" => (format!("PA{}{index};", target_digit(receiver)), 2),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => (
            format!("PA{}{index};", yaesu_target(profile, receiver, vfo_routing)),
            2,
        ),
        "kenwood-ts890" => (format!("PA{index};"), 2),
        "kenwood-ts590" | "kenwood-ts2000" | "kenwood-ts570" | "kenwood-ts870"
        | "kenwood-ts480" | "elecraft-k2" => (format!("PA{index};"), 1),
        _ => {
            return Err(RadioError::UnsupportedCapability {
                capability: "receiver.preamp",
            })
        }
    };

    if index > max {
        return Err(RadioError::InvalidValue {
            field: "receiver.preamp",
            message: format!(
                "index {index} is out of range 0..={max} for {}",
                profile.id()
            ),
        });
    }

    simple_setting_command(
        frame,
        "PA",
        receiver,
        setting_patch(receiver, Feature::Preamp, index),
    )
}

fn encode_attenuator(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    setting: LeveledSetting,
    vfo_routing: VfoRouting,
) -> Result<EncodedCommand> {
    let index = normalize_index(setting, 1)?;

    let frame = match profile.id() {
        "kenwood-ts990" => format!("RA{}{index};", target_digit(receiver)),
        "elecraft-k4" => {
            // K4 RA$NNM: NN is value index in this API stage, M is off/on.
            let enabled = if index == 0 { 0 } else { 1 };
            format!(
                "RA{}{:02}{enabled};",
                if matches!(receiver, ReceiverPath::Sub) {
                    "$"
                } else {
                    ""
                },
                index
            )
        }
        "elecraft-k3" => format!(
            "RA{}{:02};",
            if matches!(receiver, ReceiverPath::Sub) {
                "$"
            } else {
                ""
            },
            index
        ),
        "elecraft-k2" => format!("RA{index:02};"),
        "yaesu-ftdx101" => format!("RA{}{index};", target_digit(receiver)),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => {
            format!("RA{}{index};", yaesu_target(profile, receiver, vfo_routing))
        }
        "kenwood-ts890" => format!("RA{index};"),
        "kenwood-ts590" | "kenwood-ts2000" | "kenwood-ts570" | "kenwood-ts870"
        | "kenwood-ts480" => {
            format!("RA{index};")
        }
        _ => {
            return Err(RadioError::UnsupportedCapability {
                capability: "receiver.attenuator",
            })
        }
    };

    simple_setting_command(
        frame,
        "RA",
        receiver,
        setting_patch(receiver, Feature::Attenuator, index),
    )
}

fn encode_noise_blanker(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    setting: LeveledSetting,
    vfo_routing: VfoRouting,
) -> Result<EncodedCommand> {
    let index = normalize_index(setting, 1)?;

    let (frames, matcher) = match profile.id() {
        "kenwood-ts890" => {
            if index > 3 {
                return Err(RadioError::InvalidValue {
                    field: "receiver.noise_blanker",
                    message: "TS-890 NB index must be 0..=3".to_string(),
                });
            }
            let nb1 = if index == 1 || index == 3 { 1 } else { 0 };
            let nb2 = if index == 2 || index == 3 { 1 } else { 0 };
            (
                vec![
                    AsciiFrame::new(format!("NB1{nb1};"))?,
                    AsciiFrame::new(format!("NB2{nb2};"))?,
                ],
                ResponseMatcher::OneOf(&["NB1", "NB2"]),
            )
        }
        "kenwood-ts990" => {
            if index > 3 {
                return Err(RadioError::InvalidValue {
                    field: "receiver.noise_blanker",
                    message: "TS-990 NB index must be 0..=3".to_string(),
                });
            }
            let t = target_digit(receiver);
            let nb1 = if index == 1 || index == 3 { 1 } else { 0 };
            let nb2 = if index == 2 || index == 3 { 1 } else { 0 };
            (
                vec![
                    AsciiFrame::new(format!("NB1{t}{nb1};"))?,
                    AsciiFrame::new(format!("NB2{t}{nb2};"))?,
                ],
                ResponseMatcher::OneOf(&["NB"]),
            )
        }
        "kenwood-ts590" => {
            if index > 2 {
                return Err(RadioError::InvalidValue {
                    field: "receiver.noise_blanker",
                    message: "TS-590 NB index must be 0..=2".to_string(),
                });
            }
            (
                vec![AsciiFrame::new(format!("NB{index};"))?],
                ResponseMatcher::Prefix("NB"),
            )
        }
        "elecraft-k4" | "elecraft-k3" => (
            vec![AsciiFrame::new(format!(
                "NB{}{on};",
                if matches!(receiver, ReceiverPath::Sub) {
                    "$"
                } else {
                    ""
                },
                on = bool_digit(index > 0)
            ))?],
            ResponseMatcher::Prefix(if matches!(receiver, ReceiverPath::Sub) {
                "NB$"
            } else {
                "NB"
            }),
        ),
        "yaesu-ftdx101" => (
            vec![AsciiFrame::new(format!(
                "NB{}{on};",
                target_digit(receiver),
                on = bool_digit(index > 0)
            ))?],
            ResponseMatcher::Prefix("NB"),
        ),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => (
            vec![AsciiFrame::new(format!(
                "NB{}{};",
                yaesu_target(profile, receiver, vfo_routing),
                bool_digit(index > 0)
            ))?],
            ResponseMatcher::Prefix("NB"),
        ),
        "kenwood-ts2000" | "kenwood-ts480" | "kenwood-ts570" | "kenwood-ts870" => (
            vec![AsciiFrame::new(format!("NB{};", bool_digit(index > 0)))?],
            ResponseMatcher::Prefix("NB"),
        ),
        "elecraft-k2" => {
            return Err(RadioError::UnsupportedCapability {
                capability: "receiver.noise_blanker",
            });
        }
        _ => {
            return Err(RadioError::UnsupportedCapability {
                capability: "receiver.noise_blanker",
            })
        }
    };

    let patches = vec![setting_patch(receiver, Feature::NoiseBlanker, index)];
    if frames.len() == 2 && profile.id() == "kenwood-ts990" {
        let steps = frames
            .into_iter()
            .zip([
                ResponseMatcher::Prefix("NB1"),
                ResponseMatcher::Prefix("NB2"),
            ])
            .map(|(frame, expected)| {
                OutgoingStep::decoded(frame, expected, CommandPriority::Normal)
            })
            .collect();
        return Ok(EncodedCommand::with_steps(steps, patches));
    }

    Ok(EncodedCommand::new(
        frames,
        matcher,
        patches,
        CommandPriority::Normal,
    ))
}

fn encode_noise_reduction(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    setting: LeveledSetting,
    vfo_routing: VfoRouting,
) -> Result<EncodedCommand> {
    let index = normalize_index(setting, 1)?;

    if profile.id() == "elecraft-k4" && index > 10 {
        return Err(RadioError::InvalidValue {
            field: "receiver.noise_reduction",
            message: "K4 noise-reduction level must be 0..=10".to_string(),
        });
    }

    let frame = match profile.id() {
        "kenwood-ts990" => format!("NR{}{index};", target_digit(receiver)),
        "elecraft-k4" => {
            let enabled = bool_digit(index > 0);
            format!(
                "NR{}{:02}{enabled};",
                if matches!(receiver, ReceiverPath::Sub) {
                    "$"
                } else {
                    ""
                },
                index
            )
        }
        "yaesu-ftdx101" => format!(
            "NR{}{on};",
            target_digit(receiver),
            on = bool_digit(index > 0)
        ),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => {
            format!(
                "NR{}{};",
                yaesu_target(profile, receiver, vfo_routing),
                bool_digit(index > 0)
            )
        }
        "kenwood-ts590" | "kenwood-ts890" | "kenwood-ts2000" | "kenwood-ts480"
        | "kenwood-ts570" | "kenwood-ts870" => {
            if index > 2 {
                return Err(RadioError::InvalidValue {
                    field: "receiver.noise_reduction",
                    message: format!("NR index must be 0..=2 for {}", profile.id()),
                });
            }
            format!("NR{index};")
        }
        _ => {
            return Err(RadioError::UnsupportedCapability {
                capability: "receiver.noise_reduction",
            })
        }
    };

    simple_setting_command(
        frame,
        "NR",
        receiver,
        setting_patch(receiver, Feature::NoiseReduction, index),
    )
}

fn encode_auto_notch(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    enabled: bool,
    vfo_routing: VfoRouting,
) -> Result<EncodedCommand> {
    let frame = match profile.id() {
        "kenwood-ts590" => format!("NT1{};", bool_digit(enabled)),
        "kenwood-ts890" | "kenwood-ts2000" => format!("NT{};", bool_digit(enabled)),
        "kenwood-ts990" => format!(
            "NT{}{v};",
            target_digit(receiver),
            v = if enabled { 1 } else { 0 }
        ),
        "elecraft-k4" => format!(
            "NA{}{v};",
            if matches!(receiver, ReceiverPath::Sub) {
                "$"
            } else {
                ""
            },
            v = bool_digit(enabled)
        ),
        "yaesu-ftdx101" => format!("BC{}{v};", target_digit(receiver), v = bool_digit(enabled)),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => {
            format!(
                "BC{}{};",
                yaesu_target(profile, receiver, vfo_routing),
                bool_digit(enabled)
            )
        }
        _ => {
            return Err(RadioError::UnsupportedCapability {
                capability: "receiver.auto_notch",
            })
        }
    };

    Ok(EncodedCommand::new(
        vec![AsciiFrame::new(frame)?],
        auto_notch_matcher(profile, receiver),
        vec![auto_notch_patch(receiver, enabled)],
        CommandPriority::Normal,
    ))
}

fn decode_preamp(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    _vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if frame.command() == "PA$" {
        let index = parse_u8("PA", frame.payload())?;
        return Ok(vec![StatePatch::SubRxPreamp(setting_from_index(index))]);
    }

    if frame.command() == "PAX" {
        let index = parse_u8("PA", frame.payload())?;
        return Ok(vec![StatePatch::MainRxPreamp(setting_from_index(index))]);
    }

    if profile.id() == "yaesu-ftdx101" {
        let (receiver, index) = parse_targeted_value("PA", frame.payload())?;
        return Ok(vec![setting_patch(receiver, Feature::Preamp, index)]);
    }

    if is_single_target_payload(frame.payload()) {
        let index = parse_last_digit("PA", frame.payload())?;
        let receiver = if matches!(profile.id(), "yaesu-ftdx10" | "yaesu-ft710") {
            ReceiverPath::Main
        } else if frame.payload().len() > 1 {
            decode_target("PA", frame.payload().as_bytes()[0])?
        } else {
            ReceiverPath::Main
        };
        return Ok(vec![setting_patch(receiver, Feature::Preamp, index)]);
    }

    let index = parse_u8("PA", frame.payload())?;
    Ok(vec![StatePatch::MainRxPreamp(setting_from_index(index))])
}

fn decode_attenuator(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    _vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if frame.command() == "RA$" {
        let index = parse_ra_or_nr_dollar(frame.payload())?;
        return Ok(vec![StatePatch::SubRxAttenuator(setting_from_index(index))]);
    }

    if frame.command() == "RAX" {
        let index = parse_u8("RA", frame.payload())?;
        return Ok(vec![StatePatch::MainRxAttenuator(setting_from_index(
            index,
        ))]);
    }

    if profile.id() == "yaesu-ftdx101" {
        let (receiver, index) = parse_targeted_value("RA", frame.payload())?;
        return Ok(vec![setting_patch(receiver, Feature::Attenuator, index)]);
    }

    if is_single_target_payload(frame.payload()) {
        let index = parse_last_digit("RA", frame.payload())?;
        let receiver = if matches!(profile.id(), "yaesu-ftdx10" | "yaesu-ft710") {
            ReceiverPath::Main
        } else if frame.payload().len() > 1 {
            decode_target("RA", frame.payload().as_bytes()[0])?
        } else {
            ReceiverPath::Main
        };
        return Ok(vec![setting_patch(receiver, Feature::Attenuator, index)]);
    }

    let index = parse_u8("RA", frame.payload())?;
    Ok(vec![StatePatch::MainRxAttenuator(setting_from_index(
        index,
    ))])
}

fn decode_noise_blanker(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    vfo_routing: &mut VfoRouting,
) -> Result<Vec<StatePatch>> {
    if frame.command() == "NB$" {
        let enabled = parse_flag("NB", frame.payload())?;
        let index = if enabled { 1 } else { 0 };
        return Ok(vec![StatePatch::SubRxNoiseBlanker(setting_from_index(
            index,
        ))]);
    }

    let payload = frame.payload();

    if profile.id() == "kenwood-ts890" {
        // NB1x / NB2x collapse to one index.
        if payload.len() != 2 {
            return Err(RadioError::Decode {
                command: "NB",
                message: format!("expected TS-890 NB payload len 2, got {}", payload.len()),
            });
        }
        let family = payload.as_bytes()[0];
        if !matches!(family, b'1' | b'2') {
            return Err(RadioError::Decode {
                command: "NB",
                message: format!("expected TS-890 NB1/NB2 response, got {family:?}"),
            });
        }
        let enabled = parse_flag("NB", &payload[1..2])?;
        let slot = (family - b'1') as usize;
        vfo_routing.ts890_nb[slot] = Some(enabled);
        let index = match vfo_routing.ts890_nb {
            [Some(nb1), Some(nb2)] => bool_digit_value(nb1) + 2 * bool_digit_value(nb2),
            _ => return Ok(Vec::new()),
        };
        return Ok(vec![StatePatch::MainRxNoiseBlanker(setting_from_index(
            index,
        ))]);
    }

    if profile.id() == "kenwood-ts990" {
        // NB1xy / NB2xy collapse to one index per target.
        if payload.len() != 3 {
            return Err(RadioError::Decode {
                command: "NB",
                message: format!("expected TS-990 NB payload len 3, got {}", payload.len()),
            });
        }
        let family = payload.as_bytes()[0];
        if !matches!(family, b'1' | b'2') {
            return Err(RadioError::Decode {
                command: "NB",
                message: format!("expected TS-990 NB1/NB2 response, got {family:?}"),
            });
        }
        let receiver = decode_target("NB", payload.as_bytes()[1])?;
        let enabled = parse_flag("NB", &payload[2..3])?;
        let receiver_index = match receiver {
            ReceiverPath::Main => 0,
            ReceiverPath::Sub => 1,
        };
        let slot = (family - b'1') as usize;
        vfo_routing.ts990_nb[receiver_index][slot] = Some(enabled);
        let index = match vfo_routing.ts990_nb[receiver_index] {
            [Some(nb1), Some(nb2)] => bool_digit_value(nb1) + 2 * bool_digit_value(nb2),
            _ => return Ok(Vec::new()),
        };
        return Ok(vec![setting_patch(receiver, Feature::NoiseBlanker, index)]);
    }

    if profile.id().starts_with("yaesu-") && payload.len() == 2 {
        let receiver = if profile.id() == "yaesu-ftdx101" {
            decode_target("NB", payload.as_bytes()[0])?
        } else {
            ReceiverPath::Main
        };
        let enabled = parse_flag("NB", &payload[1..2])?;
        let index = if enabled { 1 } else { 0 };
        return Ok(vec![setting_patch(receiver, Feature::NoiseBlanker, index)]);
    }

    let index = parse_u8("NB", payload)?;
    Ok(vec![StatePatch::MainRxNoiseBlanker(setting_from_index(
        index,
    ))])
}

fn decode_noise_reduction(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    _vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if frame.command() == "NR$" {
        let index = parse_ra_or_nr_dollar(frame.payload())?;
        return Ok(vec![StatePatch::SubRxNoiseReduction(setting_from_index(
            index,
        ))]);
    }

    if frame.command() == "NRX" {
        let index = parse_u8("NR", frame.payload())?;
        return Ok(vec![StatePatch::MainRxNoiseReduction(setting_from_index(
            index,
        ))]);
    }

    if profile.id() == "elecraft-k4" {
        let index = parse_ra_or_nr_dollar(frame.payload())?;
        return Ok(vec![StatePatch::MainRxNoiseReduction(setting_from_index(
            index,
        ))]);
    }

    if profile.id().starts_with("yaesu-") && frame.payload().len() == 2 {
        let receiver = if profile.id() == "yaesu-ftdx101" {
            decode_target("NR", frame.payload().as_bytes()[0])?
        } else {
            ReceiverPath::Main
        };
        let enabled = parse_flag("NR", &frame.payload()[1..2])?;
        let index = if enabled { 1 } else { 0 };
        return Ok(vec![setting_patch(
            receiver,
            Feature::NoiseReduction,
            index,
        )]);
    }

    if profile.id() == "kenwood-ts990" && frame.payload().len() == 2 {
        let receiver = decode_target("NR", frame.payload().as_bytes()[0])?;
        let index = parse_u8("NR", &frame.payload()[1..2])?;
        return Ok(vec![setting_patch(
            receiver,
            Feature::NoiseReduction,
            index,
        )]);
    }

    let index = parse_u8("NR", frame.payload())?;
    Ok(vec![StatePatch::MainRxNoiseReduction(setting_from_index(
        index,
    ))])
}

fn decode_auto_notch(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    _vfo_routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    let command = frame.command();
    let payload = frame.payload();

    let (receiver, enabled) = match command {
        "NA$" => (ReceiverPath::Sub, parse_flag("NA", payload)?),
        "NA" => (ReceiverPath::Main, parse_flag("NA", payload)?),
        "NTX" => {
            if payload.len() < 2 {
                return Err(RadioError::Decode {
                    command: "NTX",
                    message: format!("expected NTX payload len >=2, got {}", payload.len()),
                });
            }
            (
                decode_target("NT", payload.as_bytes()[0])?,
                parse_u8("NT", &payload[1..2])? == 1,
            )
        }
        "BC" => {
            if payload.len() < 2 {
                return Err(RadioError::Decode {
                    command: "BC",
                    message: format!("expected BC payload len >=2, got {}", payload.len()),
                });
            }
            let receiver = if profile.id() == "yaesu-ftdx101" {
                decode_target("BC", payload.as_bytes()[0])?
            } else {
                ReceiverPath::Main
            };
            (receiver, parse_flag("BC", &payload[payload.len() - 1..])?)
        }
        "NT" => {
            if profile.id() == "kenwood-ts990" {
                if payload.len() != 2 {
                    return Err(RadioError::Decode {
                        command: "NT",
                        message: format!("expected TS-990 NT payload len 2, got {}", payload.len()),
                    });
                }
                let receiver = decode_target("NT", payload.as_bytes()[0])?;
                let mode = parse_u8("NT", &payload[1..2])?;
                return Ok(vec![auto_notch_patch(receiver, mode == 1)]);
            }
            let enabled = if profile.id() == "kenwood-ts590" {
                if payload.len() != 1 {
                    return Err(RadioError::Decode {
                        command: "NT",
                        message: format!("expected NT payload len 1, got {}", payload.len()),
                    });
                }
                parse_u8("NT", &payload[0..1])? > 0
            } else {
                parse_flag("NT", payload)?
            };
            (ReceiverPath::Main, enabled)
        }
        _ => {
            return Err(RadioError::Decode {
                command: "auto-notch",
                message: format!("unsupported auto notch frame {:?}", frame.as_str()),
            })
        }
    };

    Ok(vec![auto_notch_patch(receiver, enabled)])
}

fn simple_setting_command(
    frame_text: String,
    matcher_prefix: &'static str,
    receiver: ReceiverPath,
    patch: StatePatch,
) -> Result<EncodedCommand> {
    let matcher = if matcher_prefix == "PA" && frame_text.starts_with("PA$") {
        ResponseMatcher::Prefix("PA$")
    } else if matcher_prefix == "RA" && frame_text.starts_with("RA$") {
        ResponseMatcher::Prefix("RA$")
    } else if matcher_prefix == "NR" && frame_text.starts_with("NR$") {
        ResponseMatcher::Prefix("NR$")
    } else {
        ResponseMatcher::Prefix(matcher_prefix)
    };

    let _ = receiver;

    Ok(EncodedCommand::new(
        vec![AsciiFrame::new(frame_text)?],
        matcher,
        vec![patch],
        CommandPriority::Normal,
    ))
}

fn receiver_capability(
    profile: &KenwoodAsciiProfile,
    receiver: ReceiverPath,
    feature: Feature,
) -> Capability {
    let rx = match receiver {
        ReceiverPath::Main => &profile.capabilities.main_rx,
        ReceiverPath::Sub => profile
            .capabilities
            .sub_rx
            .as_ref()
            .unwrap_or(&profile.capabilities.main_rx),
    };

    match feature {
        Feature::Preamp => rx.rf.preamp,
        Feature::Attenuator => rx.rf.attenuator,
        Feature::NoiseBlanker => rx.rf.noise_blanker,
        Feature::NoiseReduction => rx.rf.noise_reduction,
        Feature::AutoNotch => rx.rf.auto_notch,
    }
}

fn setting_patch(receiver: ReceiverPath, feature: Feature, index: u8) -> StatePatch {
    let setting = setting_from_index(index);
    match (receiver, feature) {
        (ReceiverPath::Main, Feature::Preamp) => StatePatch::MainRxPreamp(setting),
        (ReceiverPath::Sub, Feature::Preamp) => StatePatch::SubRxPreamp(setting),
        (ReceiverPath::Main, Feature::Attenuator) => StatePatch::MainRxAttenuator(setting),
        (ReceiverPath::Sub, Feature::Attenuator) => StatePatch::SubRxAttenuator(setting),
        (ReceiverPath::Main, Feature::NoiseBlanker) => StatePatch::MainRxNoiseBlanker(setting),
        (ReceiverPath::Sub, Feature::NoiseBlanker) => StatePatch::SubRxNoiseBlanker(setting),
        (ReceiverPath::Main, Feature::NoiseReduction) => StatePatch::MainRxNoiseReduction(setting),
        (ReceiverPath::Sub, Feature::NoiseReduction) => StatePatch::SubRxNoiseReduction(setting),
        (ReceiverPath::Main, Feature::AutoNotch) | (ReceiverPath::Sub, Feature::AutoNotch) => {
            unreachable!("auto notch uses bool patches")
        }
    }
}

fn auto_notch_patch(receiver: ReceiverPath, enabled: bool) -> StatePatch {
    match receiver {
        ReceiverPath::Main => StatePatch::MainRxAutoNotch(enabled),
        ReceiverPath::Sub => StatePatch::SubRxAutoNotch(enabled),
    }
}

fn setting_from_index(index: u8) -> LeveledSetting {
    LeveledSetting::new(Some(index > 0), Some(index))
}

fn normalize_index(setting: LeveledSetting, default_on: u8) -> Result<u8> {
    match setting.level() {
        None => Ok(0),
        Some(0) => Ok(default_on),
        Some(level) => Ok(level),
    }
}

fn require_writable(capability: Capability, field: &'static str) -> Result<()> {
    if capability.can_write() {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability { capability: field })
    }
}

fn yaesu_target(
    _profile: &KenwoodAsciiProfile,
    _receiver: ReceiverPath,
    _routing: VfoRouting,
) -> char {
    '0'
}

fn target_digit(receiver: ReceiverPath) -> char {
    match receiver {
        ReceiverPath::Main => '0',
        ReceiverPath::Sub => '1',
    }
}

fn bool_digit(value: bool) -> char {
    if value {
        '1'
    } else {
        '0'
    }
}

fn bool_digit_value(value: bool) -> u8 {
    u8::from(value)
}

fn parse_flag(command: &'static str, payload: &str) -> Result<bool> {
    match payload {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(RadioError::Decode {
            command,
            message: format!("expected 0/1 payload, got {payload:?}"),
        }),
    }
}

fn parse_u8(command: &'static str, payload: &str) -> Result<u8> {
    payload.parse::<u8>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })
}

fn parse_last_digit(command: &'static str, payload: &str) -> Result<u8> {
    let last = payload
        .as_bytes()
        .last()
        .copied()
        .ok_or(RadioError::Decode {
            command,
            message: "payload is empty".to_string(),
        })?;
    if !last.is_ascii_digit() {
        return Err(RadioError::Decode {
            command,
            message: format!("expected trailing digit in {payload:?}"),
        });
    }
    Ok(last - b'0')
}

fn parse_targeted_value(command: &'static str, payload: &str) -> Result<(ReceiverPath, u8)> {
    if payload.len() < 2 {
        return Err(RadioError::Decode {
            command,
            message: format!("expected target+value payload, got {payload:?}"),
        });
    }
    let receiver = decode_target(command, payload.as_bytes()[0])?;
    let value = parse_u8(command, &payload[1..])?;
    Ok((receiver, value))
}

fn parse_ra_or_nr_dollar(payload: &str) -> Result<u8> {
    if payload.len() == 3
        && payload.as_bytes()[0].is_ascii_digit()
        && payload.as_bytes()[1].is_ascii_digit()
    {
        // NNM shape
        let level = parse_u8("RF", &payload[0..2])?;
        let enabled = parse_flag("RF", &payload[2..3])?;
        Ok(if enabled { level } else { 0 })
    } else {
        parse_u8("RF", payload)
    }
}

fn decode_target(command: &'static str, target: u8) -> Result<ReceiverPath> {
    match target {
        b'0' => Ok(ReceiverPath::Main),
        b'1' => Ok(ReceiverPath::Sub),
        _ => Err(RadioError::Decode {
            command,
            message: format!("expected target 0/1, got {:?}", target as char),
        }),
    }
}

fn matcher_for_semantic(semantic: &str) -> ResponseMatcher {
    let prefix = semantic
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic() || *ch == '$')
        .collect::<String>();

    match prefix.as_str() {
        "NB1" | "NB2" | "NB" => ResponseMatcher::Prefix("NB"),
        "NR" | "NR$" | "NRX" => ResponseMatcher::Prefix("NR"),
        "PA" | "PA$" | "PAX" => ResponseMatcher::Prefix("PA"),
        "RA" | "RA$" | "RAX" => ResponseMatcher::Prefix("RA"),
        "NT" | "NTX" | "NA" | "NA$" | "BC" => {
            ResponseMatcher::OneOf(&["NT", "NTX", "NA", "NA$", "BC"])
        }
        _ => ResponseMatcher::None,
    }
}

fn is_rf_semantic(semantic: &str) -> bool {
    matches!(
        semantic,
        "NT" | "NT0"
            | "NT1"
            | "NTX"
            | "NA"
            | "NA$"
            | "BC0"
            | "BC1"
            | "NB"
            | "NB0"
            | "NB1"
            | "NB2"
            | "NB10"
            | "NB11"
            | "NB20"
            | "NB21"
            | "NB$"
            | "NR"
            | "NR0"
            | "NR1"
            | "NRX"
            | "NR$"
            | "PA"
            | "PA0"
            | "PA1"
            | "PAX"
            | "PA$"
            | "RA"
            | "RA0"
            | "RA1"
            | "RAX"
            | "RA$"
            | "rf-dsp"
    )
}

fn is_single_target_payload(payload: &str) -> bool {
    payload.len() == 2
        && payload.as_bytes()[0].is_ascii_digit()
        && payload.as_bytes()[1].is_ascii_digit()
}

fn auto_notch_matcher(profile: &KenwoodAsciiProfile, receiver: ReceiverPath) -> ResponseMatcher {
    match profile.id() {
        "elecraft-k4" if matches!(receiver, ReceiverPath::Sub) => ResponseMatcher::Prefix("NA$"),
        "elecraft-k4" => ResponseMatcher::Prefix("NA"),
        "yaesu-ftdx101" | "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => {
            ResponseMatcher::Prefix("BC")
        }
        "kenwood-ts990" => ResponseMatcher::Prefix("NT"),
        _ => ResponseMatcher::Prefix("NT"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Feature {
    Preamp,
    Attenuator,
    NoiseBlanker,
    NoiseReduction,
    AutoNotch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::kenwood_ascii::profile_by_id;

    #[test]
    fn encodes_kenwood_and_yaesu_rf_commands() {
        let ts890 = profile_by_id("kenwood-ts890").unwrap();
        let preamp = encode(
            ts890,
            &RadioCommand::SetReceiverPreamp {
                receiver: ReceiverPath::Main,
                setting: LeveledSetting::enabled(2),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(preamp.frames[0].as_str(), "PA2;");

        let ts990 = profile_by_id("kenwood-ts990").unwrap();
        let nb = encode(
            ts990,
            &RadioCommand::SetReceiverNoiseBlanker {
                receiver: ReceiverPath::Sub,
                setting: LeveledSetting::enabled(3),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(nb.frames[0].as_str(), "NB111;");
        assert_eq!(nb.frames[1].as_str(), "NB211;");
        assert_eq!(nb.steps[0].expected, ResponseMatcher::Prefix("NB1"));
        assert_eq!(nb.steps[1].expected, ResponseMatcher::Prefix("NB2"));

        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let an = encode(
            yaesu,
            &RadioCommand::SetReceiverAutoNotch {
                receiver: ReceiverPath::Main,
                enabled: true,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(an.frames[0].as_str(), "BC01;");
    }

    #[test]
    fn malformed_auto_notch_frames_return_errors_without_panicking() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        for frame in ["NT;", "NT ;"] {
            let frame = AsciiFrame::new(frame).unwrap();
            let result = std::panic::catch_unwind(|| decode(profile, &frame));
            assert!(result.is_ok());
            assert!(result.unwrap().is_err());
        }
    }

    #[test]
    fn ts990_auto_notch_reports_only_auto_mode_as_enabled() {
        let profile = profile_by_id("kenwood-ts990").unwrap();
        for (mode, enabled) in [('0', false), ('1', true), ('2', false), ('3', false)] {
            let frame = AsciiFrame::new(format!("NT0{mode};")).unwrap();
            let decoded = decode(profile, &frame).unwrap().unwrap();
            assert!(decoded
                .patches
                .contains(&StatePatch::MainRxAutoNotch(enabled)));
        }

        let query = encode_query(profile, "NT0").unwrap().unwrap();
        assert_eq!(query.frames[0].as_str(), "NT0;");
        assert_eq!(query.matcher, ResponseMatcher::Prefix("NT"));
    }

    #[test]
    fn k2_noise_blanker_is_not_an_exact_setter() {
        let profile = profile_by_id("elecraft-k2").unwrap();
        let result = encode(
            profile,
            &RadioCommand::SetReceiverNoiseBlanker {
                receiver: ReceiverPath::Main,
                setting: LeveledSetting::disabled(),
            },
        );
        assert!(matches!(
            result,
            Err(RadioError::UnsupportedCapability {
                capability: "receiver.noise_blanker"
            })
        ));
    }

    #[test]
    fn encodes_elecraft_dollar_variants() {
        let k4 = profile_by_id("elecraft-k4").unwrap();

        let pa = encode(
            k4,
            &RadioCommand::SetReceiverPreamp {
                receiver: ReceiverPath::Sub,
                setting: LeveledSetting::enabled(2),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(pa.frames[0].as_str(), "PA$2;");

        let ra = encode(
            k4,
            &RadioCommand::SetReceiverAttenuator {
                receiver: ReceiverPath::Sub,
                setting: LeveledSetting::enabled(7),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(ra.frames[0].as_str(), "RA$071;");

        let nr = encode(
            k4,
            &RadioCommand::SetReceiverNoiseReduction {
                receiver: ReceiverPath::Main,
                setting: LeveledSetting::enabled(5),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(nr.frames[0].as_str(), "NR051;");
    }

    #[test]
    fn unsupported_capabilities_are_rejected() {
        let ts480 = profile_by_id("kenwood-ts480").unwrap();
        let err = encode(
            ts480,
            &RadioCommand::SetReceiverAutoNotch {
                receiver: ReceiverPath::Main,
                enabled: true,
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RadioError::UnsupportedCapability {
                capability: "receiver.auto_notch"
            }
        ));
    }

    #[test]
    fn query_encoding_supports_rf_startup_semantics() {
        let profile = profile_by_id("kenwood-ts990").unwrap();
        let q = encode_query(profile, "NB10").unwrap().unwrap();
        assert_eq!(q.frames[0].as_str(), "NB10;");

        let k4 = profile_by_id("elecraft-k4").unwrap();
        let q = encode_query(k4, "NA$").unwrap().unwrap();
        assert_eq!(q.frames[0].as_str(), "NA$;");
    }

    #[test]
    fn decodes_targeted_and_dollar_variants() {
        let ts990 = profile_by_id("kenwood-ts990").unwrap();
        let pa = decode(ts990, &AsciiFrame::new("PA11;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            pa.patches,
            vec![StatePatch::SubRxPreamp(LeveledSetting::enabled(1))]
        );

        let k4 = profile_by_id("elecraft-k4").unwrap();
        let ra = decode(k4, &AsciiFrame::new("RA$051;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            ra.patches,
            vec![StatePatch::SubRxAttenuator(LeveledSetting::enabled(5))]
        );

        let nr = decode(k4, &AsciiFrame::new("NR051;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            nr.patches,
            vec![StatePatch::MainRxNoiseReduction(LeveledSetting::enabled(5))]
        );

        let yaesu = profile_by_id("yaesu-ftdx101").unwrap();
        let bc = decode(yaesu, &AsciiFrame::new("BC11;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(bc.patches, vec![StatePatch::SubRxAutoNotch(true)]);
    }

    #[test]
    fn decodes_noise_reduction_and_nb_basic_frames() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();

        let nr = decode(ts590, &AsciiFrame::new("NR2;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            nr.patches,
            vec![StatePatch::MainRxNoiseReduction(LeveledSetting::enabled(2))]
        );

        let nb = decode(ts590, &AsciiFrame::new("NB0;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            nb.patches,
            vec![StatePatch::MainRxNoiseBlanker(LeveledSetting::disabled())]
        );
    }
}
