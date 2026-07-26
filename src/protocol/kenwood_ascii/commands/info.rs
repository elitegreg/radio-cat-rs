use crate::{Frequency, RadioState, Result, RitXitOffsetHz, error::RadioError, update::StatePatch};

use super::{
    DecodedFrame, PhysicalVfo, VfoRouting,
    mode::{decode_kenwood_if_mode, decode_yaesu_if_mode},
};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, EncodedCommand, KenwoodAsciiProfile, ReceiverKind, ResponseMatcher,
};

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    let command = match semantic {
        "IF" if supports_generic_if(profile) => "IF",
        "OI" if supports_yaesu_oi(profile) => "OI",
        "VS" if supports_yaesu_vs(profile) => "VS",
        _ => return Ok(None),
    };

    Ok(Some(EncodedCommand::new(
        vec![AsciiFrame::new(format!("{command};"))?],
        ResponseMatcher::Prefix(command),
        Vec::new(),
        CommandPriority::Normal,
    )))
}

pub fn decode(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
    vfo_routing: &mut VfoRouting,
) -> Result<Option<DecodedFrame>> {
    if frame.command() == "VS" && supports_yaesu_vs(profile) {
        let selected = match frame.payload() {
            "0" => PhysicalVfo::A,
            "1" => PhysicalVfo::B,
            payload => {
                return Err(RadioError::Decode {
                    command: "VS",
                    message: format!("expected 0/1 VFO selector, got {payload:?}"),
                });
            }
        };
        let patches = if vfo_routing.switchable && selected != vfo_routing.main_vfo {
            vfo_routing.main_vfo = selected;
            let mut patches = vec![StatePatch::SwapVfoFrequencies];
            if let (Some(id), Some(mode)) = (vfo_routing.main_bandwidth_id, state.main_rx.mode) {
                patches.push(StatePatch::MainRxFilterBandwidth(
                    super::filter::decode_yaesu_bandwidth(mode, id),
                ));
            }
            patches
        } else {
            Vec::new()
        };
        return Ok(Some(DecodedFrame::new(patches)));
    }

    let payload = frame.payload();
    let patches = match frame.command() {
        "IF" if supports_generic_if(profile) && is_yaesu(profile) => {
            decode_yaesu_info(profile, "IF", payload, state, PhysicalVfo::A, *vfo_routing)?
        }
        "OI" if supports_yaesu_oi(profile) => {
            decode_yaesu_info(profile, "OI", payload, state, PhysicalVfo::B, *vfo_routing)?
        }
        "IF" if supports_generic_if(profile) => decode_kenwood_if(profile, payload, vfo_routing)?,
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

fn decode_kenwood_if(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    routing: &mut VfoRouting,
) -> Result<Vec<StatePatch>> {
    if payload.len() != 35 {
        return Err(RadioError::Decode {
            command: "IF",
            message: format!(
                "expected 35-character Kenwood IF payload, got {}",
                payload.len()
            ),
        });
    }

    let frequency = parse_frequency("IF", &payload[0..11])?;
    let offset = parse_signed_offset("IF", &payload[16..21])?;
    let rit_enabled = parse_bool_digit("IF", payload.as_bytes()[21])?;
    let xit_enabled = parse_bool_digit("IF", payload.as_bytes()[22])?;
    let transmitting = parse_bool_digit("IF", payload.as_bytes()[26])?;
    let mode = decode_kenwood_if_mode(profile, payload.as_bytes()[27] as char)?;
    let active_vfo = decode_active_vfo(payload.as_bytes()[28] as char)?;
    let split = parse_bool_digit("IF", payload.as_bytes()[29])?;

    let physical_vfo = match active_vfo {
        ActiveVfo::A => PhysicalVfo::A,
        ActiveVfo::B => PhysicalVfo::B,
    };
    let changed = routing.select(physical_vfo);
    routing.set_split(split);

    let mut patches = Vec::new();
    if changed {
        patches.push(StatePatch::SwapVfoFrequencies);
    }
    patches.extend([
        StatePatch::RitXitOffset(offset),
        StatePatch::XitEnabled(xit_enabled),
        StatePatch::Transmitting(transmitting),
        StatePatch::Split(split),
    ]);

    match routing.receiver_for_vfo(physical_vfo) {
        crate::ReceiverPath::Main => {
            patches.push(StatePatch::MainRitEnabled(rit_enabled));
            patches.push(StatePatch::MainRxFrequency(frequency));
            patches.push(StatePatch::MainRxMode(mode));
            if !split {
                patches.push(StatePatch::TxFrequency(frequency));
                patches.push(StatePatch::TxMode(mode));
            }
        }
        crate::ReceiverPath::Sub => {
            patches.push(StatePatch::SubRitEnabled(rit_enabled));
            patches.push(StatePatch::SubRxPresent(true));
            patches.push(StatePatch::SubRxFrequency(frequency));
            patches.push(StatePatch::SubRxMode(mode));
            if !split {
                patches.push(StatePatch::TxFrequency(frequency));
                patches.push(StatePatch::TxMode(mode));
            }
        }
    }

    Ok(patches)
}

fn decode_yaesu_info(
    profile: &KenwoodAsciiProfile,
    command: &'static str,
    payload: &str,
    state: &RadioState,
    physical_vfo: PhysicalVfo,
    routing: VfoRouting,
) -> Result<Vec<StatePatch>> {
    if payload.len() != 25 {
        return Err(RadioError::Decode {
            command,
            message: format!(
                "expected 25-character Yaesu {command} payload, got {}",
                payload.len()
            ),
        });
    }

    let frequency = parse_frequency(command, &payload[3..12])?;
    let offset = parse_signed_offset(command, &payload[12..17])?;
    let rit_enabled = parse_bool_digit(command, payload.as_bytes()[17])?;
    let xit_enabled = parse_bool_digit(command, payload.as_bytes()[18])?;
    let mode = decode_yaesu_if_mode(profile, payload.as_bytes()[19] as char)?;
    let split = decode_yaesu_split(payload.as_bytes()[24] as char)?;

    let is_main = matches!(
        routing.receiver_for_vfo(physical_vfo),
        crate::command::ReceiverPath::Main
    );
    let mut patches = vec![StatePatch::Split(split)];
    if is_main {
        patches.extend([
            StatePatch::RitXitOffset(offset),
            StatePatch::MainRitEnabled(rit_enabled),
            StatePatch::XitEnabled(xit_enabled),
            StatePatch::MainRxFrequency(frequency),
            StatePatch::MainRxMode(mode),
        ]);
    } else {
        patches.extend([
            StatePatch::SubRitOffset(offset),
            StatePatch::SubXitOffset(offset),
            StatePatch::SubRitEnabled(rit_enabled),
            StatePatch::SubXitEnabled(xit_enabled),
            StatePatch::SubRxPresent(true),
            StatePatch::SubRxFrequency(frequency),
            StatePatch::SubRxMode(mode),
        ]);
    }
    if matches!(profile.receiver_kind, ReceiverKind::DualRx) {
        patches.push(StatePatch::SubRxPresent(true));
    }

    if is_main && !split {
        patches.push(StatePatch::TxFrequency(frequency));
        patches.push(StatePatch::TxMode(mode));
    } else if is_main && state.tx.as_ref().and_then(|tx| tx.frequency).is_none() {
        patches.push(StatePatch::TxFrequency(
            state
                .sub_rx
                .as_ref()
                .and_then(|rx| rx.frequency)
                .unwrap_or(frequency),
        ));
    }

    Ok(patches)
}

fn supports_yaesu_oi(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991"
    )
}

fn supports_yaesu_vs(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "yaesu-ftdx10" | "yaesu-ft710")
}

fn supports_generic_if(profile: &KenwoodAsciiProfile) -> bool {
    !matches!(
        profile.id(),
        "kenwood-ts890" | "kenwood-ts990" | "elecraft-k4"
    )
}

fn is_yaesu(profile: &KenwoodAsciiProfile) -> bool {
    profile.id().starts_with("yaesu-")
}

fn parse_frequency(command: &'static str, digits: &str) -> Result<Frequency> {
    let hz = digits.parse::<u64>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })?;
    Ok(Frequency::from_hz(hz))
}

fn parse_signed_offset(command: &'static str, text: &str) -> Result<RitXitOffsetHz> {
    let value = text.parse::<i16>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })?;
    RitXitOffsetHz::new(value).map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })
}

fn parse_bool_digit(command: &'static str, byte: u8) -> Result<bool> {
    match byte {
        b'0' => Ok(false),
        b'1' => Ok(true),
        _ => Err(RadioError::Decode {
            command,
            message: format!("expected 0/1 flag, got {:?}", byte as char),
        }),
    }
}

fn decode_yaesu_split(byte: char) -> Result<bool> {
    match byte {
        '0' => Ok(false),
        '1' | '2' => Ok(true),
        _ => Err(RadioError::Decode {
            command: "IF",
            message: format!("expected Yaesu split flag 0/1/2, got {byte:?}"),
        }),
    }
}

fn decode_active_vfo(byte: char) -> Result<ActiveVfo> {
    match byte {
        '0' => Ok(ActiveVfo::A),
        '1' => Ok(ActiveVfo::B),
        _ => Err(RadioError::Decode {
            command: "IF",
            message: format!("expected active VFO 0/1, got {byte:?}"),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveVfo {
    A,
    B,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Mode, StateReducer, TransmitterState,
        protocol::kenwood_ascii::{profile_by_id, split},
    };

    fn decode_with_default_routing(
        profile: &KenwoodAsciiProfile,
        frame: &AsciiFrame,
        state: &RadioState,
    ) -> DecodedFrame {
        decode(profile, frame, state, &mut VfoRouting::for_profile(profile))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn encodes_if_query_only_for_profiles_with_generic_if() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let ts890 = profile_by_id("kenwood-ts890").unwrap();

        assert_eq!(
            encode_query(ts590, "IF").unwrap().unwrap().frames[0].as_str(),
            "IF;"
        );
        assert!(encode_query(ts890, "IF").unwrap().is_none());
    }

    #[test]
    fn decodes_kenwood_if_for_active_vfo_a() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = RadioState::default();
        let frame = AsciiFrame::new(concat!(
            "IF",
            "00014074000",
            "00000",
            "+0123",
            "1",
            "0",
            "000",
            "1",
            "2",
            "0",
            "0",
            "00000",
            ";"
        ))
        .unwrap();

        let decoded = decode_with_default_routing(profile, &frame, &state);
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::RitXitOffset(RitXitOffsetHz::new(123).unwrap()),
                StatePatch::XitEnabled(false),
                StatePatch::Transmitting(true),
                StatePatch::Split(false),
                StatePatch::MainRitEnabled(true),
                StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::MainRxMode(Mode::Usb),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::TxMode(Mode::Usb),
            ]
        );
    }

    #[test]
    fn decodes_kenwood_if_for_active_vfo_b_with_split() {
        let profile = profile_by_id("kenwood-ts2000").unwrap();
        let mut state = RadioState::default();
        state.main_rx.frequency = Some(Frequency::from_hz(14_074_000));
        state.tx = Some(TransmitterState::default());
        let frame = AsciiFrame::new(concat!(
            "IF",
            "00007074000",
            "00000",
            "-0251",
            "0",
            "1",
            "000",
            "1",
            "3",
            "1",
            "1",
            "00000",
            ";"
        ))
        .unwrap();

        let decoded = decode_with_default_routing(profile, &frame, &state);
        assert_eq!(decoded.patches[0], StatePatch::SwapVfoFrequencies);
        assert!(decoded.patches.contains(&StatePatch::MainRitEnabled(false)));
        assert!(
            decoded
                .patches
                .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(7_074_000)))
        );
        assert!(decoded.patches.contains(&StatePatch::Split(true)));
        assert!(
            !decoded
                .patches
                .iter()
                .any(|patch| matches!(patch, StatePatch::TxFrequency(_)))
        );
    }

    #[test]
    fn fr1_then_vfo_b_if_updates_normalized_main_without_another_swap() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut routing = VfoRouting::for_profile(profile);
        let mut reducer = StateReducer::new(RadioState::default());

        let fr = split::decode_with_routing(
            profile,
            &AsciiFrame::new("FR1;").unwrap(),
            reducer.state(),
            &mut routing,
        )
        .unwrap()
        .unwrap();
        reducer.apply_patches(fr.patches);

        let frame = AsciiFrame::new(concat!(
            "IF",
            "00007074000",
            "00000",
            "+0000",
            "0",
            "0",
            "000",
            "0",
            "2",
            "1",
            "0",
            "00000",
            ";"
        ))
        .unwrap();
        let decoded = decode(profile, &frame, reducer.state(), &mut routing)
            .unwrap()
            .unwrap();

        assert!(!decoded.patches.contains(&StatePatch::SwapVfoFrequencies));
        assert!(
            decoded
                .patches
                .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(7_074_000)))
        );
    }

    #[test]
    fn decodes_yaesu_if_payload() {
        let profile = profile_by_id("yaesu-ftdx10").unwrap();
        let state = RadioState::default();
        let frame = AsciiFrame::new(concat!(
            "IF",
            "000",
            "014074000",
            "+0500",
            "1",
            "0",
            "2",
            "0000",
            "1",
            ";"
        ))
        .unwrap();

        let decoded = decode_with_default_routing(profile, &frame, &state);
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::Split(true),
                StatePatch::RitXitOffset(RitXitOffsetHz::new(500).unwrap()),
                StatePatch::MainRitEnabled(true),
                StatePatch::XitEnabled(false),
                StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::MainRxMode(Mode::Usb),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
            ]
        );
    }

    #[test]
    fn yaesu_oi_and_vs_queries_are_read_only_and_profile_specific() {
        let ftdx10 = profile_by_id("yaesu-ftdx10").unwrap();
        let ft891 = profile_by_id("yaesu-ft891").unwrap();
        assert_eq!(
            encode_query(ftdx10, "OI").unwrap().unwrap().frames[0].as_str(),
            "OI;"
        );
        assert_eq!(
            encode_query(ftdx10, "VS").unwrap().unwrap().frames[0].as_str(),
            "VS;"
        );
        assert!(encode_query(ft891, "VS").unwrap().is_none());
    }

    #[test]
    fn vs_routes_if_and_oi_between_main_and_sub_without_duplicate_swaps() {
        let profile = profile_by_id("yaesu-ftdx10").unwrap();
        let state = RadioState::default();
        let mut routing = VfoRouting::for_profile(profile);

        let vs = AsciiFrame::new("VS1;").unwrap();
        let first = decode(profile, &vs, &state, &mut routing).unwrap().unwrap();
        assert_eq!(first.patches, vec![StatePatch::SwapVfoFrequencies]);
        let duplicate = decode(profile, &vs, &state, &mut routing).unwrap().unwrap();
        assert!(duplicate.patches.is_empty());

        let oi = AsciiFrame::new("OI000007074000-025110300001;").unwrap();
        let decoded = decode(profile, &oi, &state, &mut routing).unwrap().unwrap();
        assert!(
            decoded
                .patches
                .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(7_074_000)))
        );
        assert!(decoded.patches.contains(&StatePatch::MainRxMode(Mode::Cw)));
        assert!(decoded.patches.contains(&StatePatch::RitXitOffset(
            RitXitOffsetHz::new(-251).unwrap()
        )));
    }

    #[test]
    fn vs_reinterprets_cached_main_bandwidth_id_for_incoming_mode() {
        let profile = profile_by_id("yaesu-ftdx10").unwrap();
        let mut state = RadioState::default();
        state.main_rx.mode = Some(Mode::Usb);
        state.sub_rx = Some(crate::ReceiverState {
            mode: Some(Mode::Cw),
            ..crate::ReceiverState::default()
        });
        let mut routing = VfoRouting::for_profile(profile);

        crate::protocol::kenwood_ascii::filter::decode_with_routing(
            profile,
            &AsciiFrame::new("SH0013;").unwrap(),
            &state,
            &mut routing,
        )
        .unwrap()
        .unwrap();

        state.main_rx.mode = Some(Mode::Cw);
        let usb_bandwidth =
            crate::protocol::kenwood_ascii::filter::decode_yaesu_bandwidth(Mode::Usb, 13);
        let cw_bandwidth =
            crate::protocol::kenwood_ascii::filter::decode_yaesu_bandwidth(Mode::Cw, 13);
        assert_ne!(usb_bandwidth, cw_bandwidth);

        let decoded = decode(
            profile,
            &AsciiFrame::new("VS1;").unwrap(),
            &state,
            &mut routing,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::SwapVfoFrequencies,
                StatePatch::MainRxFilterBandwidth(cw_bandwidth),
            ]
        );
    }

    #[test]
    fn fixed_vfo_routing_maps_oi_to_sub_vfo() {
        let profile = profile_by_id("yaesu-ft991").unwrap();
        let state = RadioState::default();
        let oi = AsciiFrame::new("OI000007074000+012310E00000;").unwrap();
        let decoded = decode_with_default_routing(profile, &oi, &state);

        assert!(
            decoded
                .patches
                .contains(&StatePatch::SubRxFrequency(Frequency::from_hz(7_074_000)))
        );
        assert!(
            decoded
                .patches
                .contains(&StatePatch::SubRxMode(Mode::DigitalVoice))
        );
        assert!(
            decoded
                .patches
                .contains(&StatePatch::SubRitOffset(RitXitOffsetHz::new(123).unwrap()))
        );
        assert!(
            !decoded
                .patches
                .iter()
                .any(|patch| matches!(patch, StatePatch::TxFrequency(_)))
        );
    }

    #[test]
    fn decodes_ft991_special_mode_code() {
        let profile = profile_by_id("yaesu-ft991").unwrap();
        let state = RadioState::default();
        let frame = AsciiFrame::new(concat!(
            "IF",
            "000",
            "014074000",
            "+0000",
            "0",
            "0",
            "E",
            "0000",
            "0",
            ";"
        ))
        .unwrap();

        let decoded = decode_with_default_routing(profile, &frame, &state);
        assert!(
            decoded
                .patches
                .contains(&StatePatch::MainRxMode(Mode::DigitalVoice))
        );
    }
}
