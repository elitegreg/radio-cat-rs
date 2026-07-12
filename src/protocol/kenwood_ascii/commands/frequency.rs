use crate::{
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::StatePatch,
    Frequency, RadioState, Result,
};

use super::{
    split::{current_tx_vfo, tx_vfo_from_state, RoutingVfo},
    DecodedFrame, EncodedCommand, FrequencyCommandTarget, VfoRouting,
};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, FrequencyFormat, KenwoodAsciiProfile, ResponseMatcher,
};

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
        RadioCommand::SetReceiverFrequency {
            receiver,
            frequency,
        } => Ok(Some(encode_targeted_frequency(
            profile,
            physical_target(profile, receiver_target(*receiver), vfo_routing),
            *frequency,
            frequency_optimistic_patch(profile, receiver_target(*receiver), *frequency, state),
        )?)),
        RadioCommand::SetTxFrequency(frequency) => {
            let target = tx_target_from_state(profile, state)?;
            let optimistic = frequency_optimistic_patch(profile, target, *frequency, state);
            Ok(Some(encode_targeted_frequency(
                profile,
                physical_target(profile, target, vfo_routing),
                *frequency,
                optimistic,
            )?))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    encode_query_with_routing(profile, semantic, VfoRouting::for_profile(profile))
}

pub fn encode_query_with_routing(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
    vfo_routing: VfoRouting,
) -> Result<Option<EncodedCommand>> {
    let target = match semantic {
        "FA" => FrequencyCommandTarget::Main,
        "FB" => FrequencyCommandTarget::Sub,
        _ => return Ok(None),
    };

    let command = command_for_target(physical_target(profile, target, vfo_routing));
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
) -> Result<Option<DecodedFrame>> {
    decode_with_routing(profile, frame, state, VfoRouting::for_profile(profile))
}

pub fn decode_with_routing(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
    vfo_routing: VfoRouting,
) -> Result<Option<DecodedFrame>> {
    let physical_target = match frame.command() {
        "FA" => FrequencyCommandTarget::Main,
        "FB" => FrequencyCommandTarget::Sub,
        _ => return Ok(None),
    };

    let target = logical_target(profile, physical_target, vfo_routing);
    let digits = frequency_digits(profile);
    let payload = frame.payload();
    if payload.len() != digits || !payload.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(RadioError::Decode {
            command: frame.command_static_hint(),
            message: format!(
                "expected {digits} frequency digits in {} frame, got {payload:?}",
                frame.command()
            ),
        });
    }

    let hz = payload.parse::<u64>().map_err(|error| RadioError::Decode {
        command: frame.command_static_hint(),
        message: error.to_string(),
    })?;
    let frequency = Frequency::from_hz(hz);

    let mut patches = frequency_decode_patches(profile, target, frequency, state);
    if patches.is_empty() {
        patches.push(StatePatch::TxFrequency(frequency));
    }

    Ok(Some(DecodedFrame::new(patches)))
}

fn encode_targeted_frequency(
    profile: &KenwoodAsciiProfile,
    target: FrequencyCommandTarget,
    frequency: Frequency,
    optimistic: Vec<StatePatch>,
) -> Result<EncodedCommand> {
    let command = command_for_target(target);
    let digits = frequency_digits(profile);
    let frame = AsciiFrame::new(format!("{command}{:0digits$};", frequency.hz()))?;
    Ok(EncodedCommand::new(
        vec![frame],
        ResponseMatcher::Prefix(command),
        optimistic,
        CommandPriority::Normal,
    ))
}

fn frequency_optimistic_patch(
    profile: &KenwoodAsciiProfile,
    target: FrequencyCommandTarget,
    frequency: Frequency,
    state: &RadioState,
) -> Vec<StatePatch> {
    frequency_decode_patches(profile, target, frequency, state)
}

fn frequency_decode_patches(
    profile: &KenwoodAsciiProfile,
    target: FrequencyCommandTarget,
    frequency: Frequency,
    state: &RadioState,
) -> Vec<StatePatch> {
    let mut patches = vec![match target {
        FrequencyCommandTarget::Main => StatePatch::MainRxFrequency(frequency),
        FrequencyCommandTarget::Sub => StatePatch::SubRxFrequency(frequency),
    }];

    if current_tx_vfo(profile, state) == Some(routing_vfo(target)) {
        patches.push(StatePatch::TxFrequency(frequency));
    }

    patches
}

fn physical_target(
    profile: &KenwoodAsciiProfile,
    target: FrequencyCommandTarget,
    routing: VfoRouting,
) -> FrequencyCommandTarget {
    if uses_vfo_mapping(profile) {
        let receiver = match target {
            FrequencyCommandTarget::Main => ReceiverPath::Main,
            FrequencyCommandTarget::Sub => ReceiverPath::Sub,
        };
        match routing.vfo_for_receiver(receiver) {
            super::PhysicalVfo::A => FrequencyCommandTarget::Main,
            super::PhysicalVfo::B => FrequencyCommandTarget::Sub,
        }
    } else {
        target
    }
}

fn logical_target(
    profile: &KenwoodAsciiProfile,
    target: FrequencyCommandTarget,
    routing: VfoRouting,
) -> FrequencyCommandTarget {
    if uses_vfo_mapping(profile) {
        let byte = match target {
            FrequencyCommandTarget::Main => b'0',
            FrequencyCommandTarget::Sub => b'1',
        };
        match routing.receiver_for_target(byte).expect("known VFO target") {
            ReceiverPath::Main => FrequencyCommandTarget::Main,
            ReceiverPath::Sub => FrequencyCommandTarget::Sub,
        }
    } else {
        target
    }
}

fn uses_vfo_mapping(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "yaesu-ftdx10"
            | "yaesu-ft710"
            | "yaesu-ft891"
            | "yaesu-ft991"
            | "kenwood-ts590"
            | "kenwood-ts890"
            | "kenwood-ts2000"
            | "kenwood-ts480"
            | "kenwood-ts570"
            | "kenwood-ts870"
            | "elecraft-k2"
    )
}

fn receiver_target(receiver: ReceiverPath) -> FrequencyCommandTarget {
    match receiver {
        ReceiverPath::Main => FrequencyCommandTarget::Main,
        ReceiverPath::Sub => FrequencyCommandTarget::Sub,
    }
}

fn tx_target_from_state(
    profile: &KenwoodAsciiProfile,
    state: &RadioState,
) -> Result<FrequencyCommandTarget> {
    Ok(match tx_vfo_from_state(profile, state, "tx.frequency")? {
        RoutingVfo::Main => FrequencyCommandTarget::Main,
        RoutingVfo::Sub => FrequencyCommandTarget::Sub,
    })
}

fn routing_vfo(target: FrequencyCommandTarget) -> RoutingVfo {
    match target {
        FrequencyCommandTarget::Main => RoutingVfo::Main,
        FrequencyCommandTarget::Sub => RoutingVfo::Sub,
    }
}

fn command_for_target(target: FrequencyCommandTarget) -> &'static str {
    match target {
        FrequencyCommandTarget::Main => "FA",
        FrequencyCommandTarget::Sub => "FB",
    }
}

fn frequency_digits(profile: &KenwoodAsciiProfile) -> usize {
    match profile.frequency_format {
        FrequencyFormat::Hertz9Digit => 9,
        FrequencyFormat::Hertz11Digit => 11,
    }
}

trait FrameCommandHint {
    fn command_static_hint(&self) -> &'static str;
}

impl FrameCommandHint for AsciiFrame {
    fn command_static_hint(&self) -> &'static str {
        match self.command() {
            "FA" => "FA",
            "FB" => "FB",
            _ => "frequency",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::kenwood_ascii::profile_by_id, ReceiverState, TransmitterState};

    fn routed_state(split: bool, tx_frequency: Frequency) -> RadioState {
        RadioState {
            main_rx: ReceiverState {
                frequency: Some(Frequency::from_hz(14_074_000)),
                ..ReceiverState::default()
            },
            sub_rx: Some(ReceiverState {
                frequency: Some(Frequency::from_hz(7_074_000)),
                ..ReceiverState::default()
            }),
            tx: Some(TransmitterState {
                frequency: Some(tx_frequency),
                split: Some(split),
                ..TransmitterState::default()
            }),
            ..RadioState::default()
        }
    }

    #[test]
    fn encodes_11_and_9_digit_frequency_frames() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let state = RadioState::default();

        let ts590_cmd = encode(
            ts590,
            &RadioCommand::SetReceiverFrequency {
                receiver: ReceiverPath::Main,
                frequency: Frequency::from_hz(14_074_000),
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ts590_cmd.frames[0].as_str(), "FA00014074000;");

        let yaesu_cmd = encode(
            yaesu,
            &RadioCommand::SetReceiverFrequency {
                receiver: ReceiverPath::Main,
                frequency: Frequency::from_hz(14_074_000),
            },
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(yaesu_cmd.frames[0].as_str(), "FA014074000;");
    }

    #[test]
    fn tx_frequency_tracks_split_vfo() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = routed_state(true, Frequency::from_hz(7_074_000));

        let encoded = encode(
            profile,
            &RadioCommand::SetTxFrequency(Frequency::from_hz(7_074_000)),
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "FB00007074000;");
        assert_eq!(
            encoded.completion_patches,
            vec![
                StatePatch::SubRxFrequency(Frequency::from_hz(7_074_000)),
                StatePatch::TxFrequency(Frequency::from_hz(7_074_000)),
            ]
        );
    }

    #[test]
    fn decodes_frequency_frames_into_receiver_and_tx_state() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = routed_state(false, Frequency::from_hz(14_074_000));

        let decoded = decode(profile, &AsciiFrame::new("FA00014074000;").unwrap(), &state)
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
            ]
        );
    }

    #[test]
    fn frequency_queries_use_matching_response_prefix() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let query = encode_query(profile, "FB").unwrap().unwrap();

        assert_eq!(query.frames[0].as_str(), "FB;");
        assert_eq!(query.matcher, ResponseMatcher::Prefix("FB"));
    }

    #[test]
    fn routed_tx_frequency_uses_active_vfo_when_split_is_off() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = routed_state(false, Frequency::from_hz(7_074_000));

        let encoded = encode(
            profile,
            &RadioCommand::SetTxFrequency(Frequency::from_hz(7_100_000)),
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "FB00007100000;");
    }

    #[test]
    fn switched_vfo_routing_maps_fa_fb_to_normalized_receivers() {
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

        let fa = decode_with_routing(
            profile,
            &AsciiFrame::new("FA007074000;").unwrap(),
            &state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert!(fa
            .patches
            .contains(&StatePatch::SubRxFrequency(Frequency::from_hz(7_074_000))));

        let fb = decode_with_routing(
            profile,
            &AsciiFrame::new("FB014074000;").unwrap(),
            &state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert!(fb
            .patches
            .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000))));

        let main_set = encode_with_routing(
            profile,
            &RadioCommand::SetReceiverFrequency {
                receiver: ReceiverPath::Main,
                frequency: Frequency::from_hz(14_100_000),
            },
            &state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert_eq!(main_set.frames[0].as_str(), "FB014100000;");
        assert_eq!(
            main_set.completion_patches[0],
            StatePatch::MainRxFrequency(Frequency::from_hz(14_100_000))
        );

        assert_eq!(
            encode_query_with_routing(profile, "FA", routing)
                .unwrap()
                .unwrap()
                .frames[0]
                .as_str(),
            "FB;"
        );
        assert_eq!(
            encode_query_with_routing(profile, "FB", routing)
                .unwrap()
                .unwrap()
                .frames[0]
                .as_str(),
            "FA;"
        );
    }

    #[test]
    fn fr_routing_maps_kenwood_fa_fb_and_frequency_setters() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = RadioState::default();
        let mut routing = VfoRouting::for_profile(profile);
        assert!(routing.select(crate::protocol::kenwood_ascii::PhysicalVfo::B));

        let decoded = decode_with_routing(
            profile,
            &AsciiFrame::new("FB00007100000;").unwrap(),
            &state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(7_100_000))));

        let encoded = encode_with_routing(
            profile,
            &RadioCommand::SetReceiverFrequency {
                receiver: ReceiverPath::Main,
                frequency: Frequency::from_hz(7_125_000),
            },
            &state,
            routing,
        )
        .unwrap()
        .unwrap();
        assert_eq!(encoded.frames[0].as_str(), "FB00007125000;");
        assert_eq!(
            encoded.completion_patches[0],
            StatePatch::MainRxFrequency(Frequency::from_hz(7_125_000))
        );
    }

    #[test]
    fn fixed_vfo_routing_keeps_fa_main_and_fb_sub() {
        for id in ["yaesu-ft891", "yaesu-ft991"] {
            let profile = profile_by_id(id).unwrap();
            let routing = VfoRouting::for_profile(profile);
            assert_eq!(
                encode_query_with_routing(profile, "FA", routing)
                    .unwrap()
                    .unwrap()
                    .frames[0]
                    .as_str(),
                "FA;"
            );
            assert_eq!(
                encode_query_with_routing(profile, "FB", routing)
                    .unwrap()
                    .unwrap()
                    .frames[0]
                    .as_str(),
                "FB;"
            );
        }
    }

    #[test]
    fn routed_frequency_decode_updates_tx_for_selected_tx_vfo() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = routed_state(false, Frequency::from_hz(7_074_000));

        let decoded = decode(profile, &AsciiFrame::new("FB00007100000;").unwrap(), &state)
            .unwrap()
            .unwrap();

        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::SubRxFrequency(Frequency::from_hz(7_100_000)),
                StatePatch::TxFrequency(Frequency::from_hz(7_100_000)),
            ]
        );
    }
}
