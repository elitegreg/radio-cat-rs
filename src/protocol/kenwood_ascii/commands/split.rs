use crate::{command::RadioCommand, error::RadioError, update::StatePatch, RadioState, Result};

use super::{DecodedFrame, EncodedCommand, PhysicalVfo, VfoRouting};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiProfile, ResponseMatcher,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RoutingVfo {
    Main,
    Sub,
}

impl RoutingVfo {
    pub(crate) const fn opposite(self) -> Self {
        match self {
            Self::Main => Self::Sub,
            Self::Sub => Self::Main,
        }
    }
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
    routing: VfoRouting,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetSplit(split) => {
            encode_set_split(profile, *split, state, routing).map(Some)
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    let (frame, matcher) = match semantic {
        "FR" if supports_fr(profile) => ("FR;", ResponseMatcher::Prefix("FR")),
        "FT" if uses_yaesu_ft(profile) => ("FTX;", ResponseMatcher::Prefix("FT")),
        "FT" if supports_ft(profile) => ("FT;", ResponseMatcher::Prefix("FT")),
        "SP" if uses_sp(profile) => ("SP;", ResponseMatcher::Prefix("SP")),
        "ST" if uses_st(profile) => ("ST;", ResponseMatcher::Prefix("ST")),
        _ => return Ok(None),
    };

    Ok(Some(EncodedCommand::new(
        vec![AsciiFrame::new(frame)?],
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
    decode_with_routing(profile, frame, state, &mut VfoRouting::for_profile(profile))
}

pub fn decode_with_routing(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
    routing: &mut VfoRouting,
) -> Result<Option<DecodedFrame>> {
    let patches = match frame.command() {
        "FR" if supports_fr(profile) => decode_fr(profile, frame.payload(), state, routing)?,
        "FT" if supports_ft(profile) || uses_yaesu_ft(profile) => {
            decode_ft(profile, frame.payload(), state, routing)?
        }
        "SP" if uses_sp(profile) => decode_direct_split("SP", frame.payload(), state, routing)?,
        "ST" if uses_st(profile) => decode_direct_split("ST", frame.payload(), state, routing)?,
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

fn encode_set_split(
    profile: &KenwoodAsciiProfile,
    split: bool,
    state: &RadioState,
    routing: VfoRouting,
) -> Result<EncodedCommand> {
    if uses_routed_ft(profile) {
        let rx_vfo = routing_vfo(routing.main_vfo);
        let tx_vfo = if split { rx_vfo.opposite() } else { rx_vfo };
        return Ok(EncodedCommand::new(
            vec![AsciiFrame::new(format!("FT{};", encode_vfo(tx_vfo)))?],
            ResponseMatcher::Prefix("FT"),
            direct_split_patches(split, tx_vfo, state, routing),
            CommandPriority::Normal,
        ));
    }

    let (frame, matcher, optimistic) = if uses_sp(profile) {
        (
            format!("SP{};", split_digit(split)),
            ResponseMatcher::Prefix("SP"),
            direct_split_patches(split, selected_tx_vfo(split, routing), state, routing),
        )
    } else if profile.id() == "elecraft-k4" {
        (
            format!("FT{};", split_digit(split)),
            ResponseMatcher::Prefix("FT"),
            direct_split_patches(split, selected_tx_vfo(split, routing), state, routing),
        )
    } else if profile.id() == "elecraft-k3" {
        if split {
            (
                "FT1;".to_string(),
                ResponseMatcher::Prefix("FT"),
                direct_split_patches(true, selected_tx_vfo(true, routing), state, routing),
            )
        } else {
            (
                "FR0;".to_string(),
                ResponseMatcher::Prefix("FR"),
                direct_split_patches(false, selected_tx_vfo(false, routing), state, routing),
            )
        }
    } else if uses_yaesu_ft(profile) {
        (
            if split { "FT3;" } else { "FT2;" }.to_string(),
            ResponseMatcher::Prefix("FT"),
            direct_split_patches(split, selected_tx_vfo(split, routing), state, routing),
        )
    } else if uses_st(profile) {
        (
            format!("ST{};", split_digit(split)),
            ResponseMatcher::Prefix("ST"),
            direct_split_patches(split, selected_tx_vfo(split, routing), state, routing),
        )
    } else {
        return Err(RadioError::UnsupportedCapability {
            capability: "tx.split",
        });
    };

    Ok(EncodedCommand::new(
        vec![AsciiFrame::new(frame)?],
        matcher,
        optimistic,
        CommandPriority::Normal,
    ))
}

fn decode_fr(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    routing: &mut VfoRouting,
) -> Result<Vec<StatePatch>> {
    let physical_rx_vfo = decode_vfo("FR", payload)?;
    let physical = physical_vfo(physical_rx_vfo);
    let old_routing = *routing;
    let changed = routing.select(physical);

    if profile.id() == "elecraft-k3" {
        routing.set_split(false);
        return Ok(direct_split_patches(
            false,
            routing_vfo(routing.tx_vfo()),
            state,
            old_routing,
        ));
    }

    let mut patches = if changed {
        vec![StatePatch::SwapVfoFrequencies]
    } else {
        Vec::new()
    };
    patches.push(StatePatch::Split(routing.split()));
    append_tx_from_vfo(
        &mut patches,
        routing_vfo(routing.tx_vfo()),
        state,
        old_routing,
    );

    Ok(patches)
}

fn decode_ft(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
    routing: &mut VfoRouting,
) -> Result<Vec<StatePatch>> {
    if uses_yaesu_ft(profile) || matches!(profile.id(), "elecraft-k4" | "elecraft-k3") {
        let split = decode_split_digit("FT", payload)?;
        routing.set_split(split);
        return Ok(direct_split_patches(
            split,
            routing_vfo(routing.tx_vfo()),
            state,
            *routing,
        ));
    }

    let tx_vfo = decode_vfo("FT", payload)?;
    routing.set_tx_vfo(physical_vfo(tx_vfo));
    let mut patches = vec![StatePatch::Split(routing.split())];
    append_tx_from_vfo(&mut patches, tx_vfo, state, *routing);
    Ok(patches)
}

fn decode_direct_split(
    command: &'static str,
    payload: &str,
    state: &RadioState,
    routing: &mut VfoRouting,
) -> Result<Vec<StatePatch>> {
    let split = decode_split_digit(command, payload)?;
    routing.set_split(split);
    Ok(direct_split_patches(
        split,
        routing_vfo(routing.tx_vfo()),
        state,
        *routing,
    ))
}

fn direct_split_patches(
    split: bool,
    tx_vfo: RoutingVfo,
    state: &RadioState,
    routing: VfoRouting,
) -> Vec<StatePatch> {
    let mut patches = vec![StatePatch::Split(split)];
    append_tx_from_vfo(&mut patches, tx_vfo, state, routing);
    patches
}

fn append_tx_from_vfo(
    patches: &mut Vec<StatePatch>,
    tx_vfo: RoutingVfo,
    state: &RadioState,
    routing: VfoRouting,
) {
    let receiver = routing.receiver_for_vfo(physical_vfo(tx_vfo));
    let receiver_state = match receiver {
        crate::ReceiverPath::Main => &state.main_rx,
        crate::ReceiverPath::Sub => match state.sub_rx.as_ref() {
            Some(receiver) => receiver,
            None => return,
        },
    };
    if let Some(frequency) = receiver_state.frequency {
        patches.push(StatePatch::TxFrequency(frequency));
    }
    if let Some(mode) = receiver_state.mode {
        patches.push(StatePatch::TxMode(mode));
    }
}

fn decode_vfo(command: &'static str, payload: &str) -> Result<RoutingVfo> {
    match payload {
        "0" => Ok(RoutingVfo::Main),
        "1" => Ok(RoutingVfo::Sub),
        _ => Err(RadioError::Decode {
            command,
            message: format!("expected 0/1 VFO selector, got {payload:?}"),
        }),
    }
}

fn decode_split_digit(command: &'static str, payload: &str) -> Result<bool> {
    match payload {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(RadioError::Decode {
            command,
            message: format!("expected 0/1 split flag, got {payload:?}"),
        }),
    }
}

fn encode_vfo(vfo: RoutingVfo) -> char {
    match vfo {
        RoutingVfo::Main => '0',
        RoutingVfo::Sub => '1',
    }
}

fn physical_vfo(vfo: RoutingVfo) -> PhysicalVfo {
    match vfo {
        RoutingVfo::Main => PhysicalVfo::A,
        RoutingVfo::Sub => PhysicalVfo::B,
    }
}

fn routing_vfo(vfo: PhysicalVfo) -> RoutingVfo {
    match vfo {
        PhysicalVfo::A => RoutingVfo::Main,
        PhysicalVfo::B => RoutingVfo::Sub,
    }
}

fn selected_tx_vfo(split: bool, routing: VfoRouting) -> RoutingVfo {
    let rx_vfo = routing_vfo(routing.main_vfo);
    if split {
        rx_vfo.opposite()
    } else {
        rx_vfo
    }
}

fn split_digit(split: bool) -> char {
    if split {
        '1'
    } else {
        '0'
    }
}

fn supports_fr(profile: &KenwoodAsciiProfile) -> bool {
    uses_routed_ft(profile) || profile.id() == "elecraft-k3"
}

fn supports_ft(profile: &KenwoodAsciiProfile) -> bool {
    uses_routed_ft(profile) || matches!(profile.id(), "elecraft-k4" | "elecraft-k3")
}

fn uses_routed_ft(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "kenwood-ts590"
            | "kenwood-ts890"
            | "kenwood-ts2000"
            | "kenwood-ts480"
            | "kenwood-ts570"
            | "kenwood-ts870"
            | "elecraft-k2"
    )
}

fn uses_sp(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "kenwood-ts990" | "kenwood-if232")
}

fn uses_yaesu_ft(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "yaesu-ftdx101" | "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft991"
    )
}

fn uses_st(profile: &KenwoodAsciiProfile) -> bool {
    profile.id() == "yaesu-ft891"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::kenwood_ascii::profile_by_id, Frequency, Mode, ReceiverState, TransmitterState,
    };

    fn routed_state(split: bool, tx_frequency: Frequency) -> RadioState {
        RadioState {
            main_rx: ReceiverState {
                frequency: Some(Frequency::from_hz(14_074_000)),
                mode: Some(Mode::Usb),
                ..ReceiverState::default()
            },
            sub_rx: Some(ReceiverState {
                frequency: Some(Frequency::from_hz(7_074_000)),
                mode: Some(Mode::Lsb),
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
    fn routed_ft_split_uses_opposite_vfo() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut state = routed_state(false, Frequency::from_hz(7_074_000));
        let old_main = state.main_rx.clone();
        state.main_rx = state.sub_rx.take().unwrap();
        state.sub_rx = Some(old_main);
        let mut routing = VfoRouting::for_profile(profile);
        routing.select(PhysicalVfo::B);
        routing.set_tx_vfo(PhysicalVfo::B);

        let encoded = encode_with_routing(profile, &RadioCommand::SetSplit(true), &state, routing)
            .unwrap()
            .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "FT0;");
        assert_eq!(
            encoded.completion_patches,
            vec![
                StatePatch::Split(true),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::TxMode(Mode::Usb),
            ]
        );
    }

    #[test]
    fn fr_switch_routes_new_main_and_tx_to_selected_physical_vfo() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let state = routed_state(false, Frequency::from_hz(14_074_000));
        let mut routing = VfoRouting::for_profile(profile);

        let decoded = decode_with_routing(
            profile,
            &AsciiFrame::new("FR1;").unwrap(),
            &state,
            &mut routing,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::SwapVfoFrequencies,
                StatePatch::Split(true),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::TxMode(Mode::Usb),
            ]
        );
    }

    #[test]
    fn direct_split_profiles_use_family_specific_frames() {
        let ts990 = profile_by_id("kenwood-ts990").unwrap();
        let ft891 = profile_by_id("yaesu-ft891").unwrap();
        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();

        assert_eq!(
            encode(ts990, &RadioCommand::SetSplit(true), &RadioState::default())
                .unwrap()
                .unwrap()
                .frames[0]
                .as_str(),
            "SP1;"
        );
        assert_eq!(
            encode(
                ft891,
                &RadioCommand::SetSplit(false),
                &RadioState::default()
            )
            .unwrap()
            .unwrap()
            .frames[0]
                .as_str(),
            "ST0;"
        );
        assert_eq!(
            encode(yaesu, &RadioCommand::SetSplit(true), &RadioState::default())
                .unwrap()
                .unwrap()
                .frames[0]
                .as_str(),
            "FT3;"
        );
    }

    #[test]
    fn split_queries_follow_profile_specific_syntax() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let ft891 = profile_by_id("yaesu-ft891").unwrap();

        assert_eq!(
            encode_query(ts590, "FR").unwrap().unwrap().frames[0].as_str(),
            "FR;"
        );
        assert_eq!(
            encode_query(yaesu, "FT").unwrap().unwrap().frames[0].as_str(),
            "FTX;"
        );
        assert_eq!(
            encode_query(ft891, "ST").unwrap().unwrap().frames[0].as_str(),
            "ST;"
        );
    }

    #[test]
    fn ft_decode_uses_known_routing_to_emit_split_and_tx_updates() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut state = routed_state(false, Frequency::from_hz(7_074_000));
        let old_main = state.main_rx.clone();
        state.main_rx = state.sub_rx.take().unwrap();
        state.sub_rx = Some(old_main);
        let mut routing = VfoRouting::for_profile(profile);
        routing.select(PhysicalVfo::B);

        let decoded = decode_with_routing(
            profile,
            &AsciiFrame::new("FT0;").unwrap(),
            &state,
            &mut routing,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::Split(true),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::TxMode(Mode::Usb),
            ]
        );
    }

    #[test]
    fn direct_split_decode_updates_split_and_tx_state() {
        let profile = profile_by_id("kenwood-ts990").unwrap();
        let state = routed_state(false, Frequency::from_hz(14_074_000));

        let decoded = decode(profile, &AsciiFrame::new("SP1;").unwrap(), &state)
            .unwrap()
            .unwrap();

        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::Split(true),
                StatePatch::TxFrequency(Frequency::from_hz(7_074_000)),
                StatePatch::TxMode(Mode::Lsb),
            ]
        );
    }

    #[test]
    fn routed_split_uses_default_session_routing_when_public_state_is_unknown() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let encoded = encode(
            profile,
            &RadioCommand::SetSplit(true),
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "FT1;");
    }
}
