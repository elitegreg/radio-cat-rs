use crate::{
    command::RadioCommand, error::RadioError, update::StatePatch, Frequency, Mode, RadioState,
    Result,
};

use super::{DecodedFrame, EncodedCommand};
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
    match command {
        RadioCommand::SetSplit(split) => encode_set_split(profile, *split, state).map(Some),
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
    let patches = match frame.command() {
        "FR" if supports_fr(profile) => decode_fr(profile, frame.payload(), state)?,
        "FT" if supports_ft(profile) || uses_yaesu_ft(profile) => {
            decode_ft(profile, frame.payload(), state)?
        }
        "SP" if uses_sp(profile) => decode_direct_split("SP", frame.payload(), state)?,
        "ST" if uses_st(profile) => decode_direct_split("ST", frame.payload(), state)?,
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

pub(crate) fn tx_vfo_from_state(
    profile: &KenwoodAsciiProfile,
    state: &RadioState,
    field: &'static str,
) -> Result<RoutingVfo> {
    current_tx_vfo(profile, state).ok_or(RadioError::InvalidValue {
        field,
        message: format!(
            "cannot determine TX VFO routing for {}; refresh the radio state first",
            profile.id()
        ),
    })
}

pub(crate) fn current_tx_vfo(
    profile: &KenwoodAsciiProfile,
    state: &RadioState,
) -> Option<RoutingVfo> {
    if uses_routed_ft(profile) {
        routed_tx_vfo(state)
    } else {
        Some(default_tx_vfo(
            state.tx.as_ref().and_then(|tx| tx.split).unwrap_or(false),
        ))
    }
}

pub(crate) fn frequency_for_vfo(vfo: RoutingVfo, state: &RadioState) -> Option<Frequency> {
    match vfo {
        RoutingVfo::Main => state.main_rx.frequency,
        RoutingVfo::Sub => state.sub_rx.as_ref().and_then(|rx| rx.frequency),
    }
}

pub(crate) fn mode_for_vfo(vfo: RoutingVfo, state: &RadioState) -> Option<Mode> {
    match vfo {
        RoutingVfo::Main => state.main_rx.mode,
        RoutingVfo::Sub => state.sub_rx.as_ref().and_then(|rx| rx.mode),
    }
}

fn encode_set_split(
    profile: &KenwoodAsciiProfile,
    split: bool,
    state: &RadioState,
) -> Result<EncodedCommand> {
    if uses_routed_ft(profile) {
        let rx_vfo = routed_rx_vfo(state).ok_or(RadioError::InvalidValue {
            field: "split",
            message: format!(
                "cannot determine active RX VFO for {}; refresh routing state first",
                profile.id()
            ),
        })?;
        let tx_vfo = if split { rx_vfo.opposite() } else { rx_vfo };
        return Ok(EncodedCommand::new(
            vec![AsciiFrame::new(format!("FT{};", encode_vfo(tx_vfo)))?],
            ResponseMatcher::Prefix("FT"),
            direct_split_patches(split, tx_vfo, state),
            CommandPriority::Normal,
        ));
    }

    let (frame, matcher, optimistic) = if uses_sp(profile) {
        (
            format!("SP{};", split_digit(split)),
            ResponseMatcher::Prefix("SP"),
            direct_split_patches(split, default_tx_vfo(split), state),
        )
    } else if profile.id() == "elecraft-k4" {
        (
            format!("FT{};", split_digit(split)),
            ResponseMatcher::Prefix("FT"),
            direct_split_patches(split, default_tx_vfo(split), state),
        )
    } else if profile.id() == "elecraft-k3" {
        if split {
            (
                "FT1;".to_string(),
                ResponseMatcher::Prefix("FT"),
                direct_split_patches(true, RoutingVfo::Sub, state),
            )
        } else {
            (
                "FR0;".to_string(),
                ResponseMatcher::Prefix("FR"),
                direct_split_patches(false, RoutingVfo::Main, state),
            )
        }
    } else if uses_yaesu_ft(profile) {
        (
            if split { "FT3;" } else { "FT2;" }.to_string(),
            ResponseMatcher::Prefix("FT"),
            direct_split_patches(split, default_tx_vfo(split), state),
        )
    } else if uses_st(profile) {
        (
            format!("ST{};", split_digit(split)),
            ResponseMatcher::Prefix("ST"),
            direct_split_patches(split, default_tx_vfo(split), state),
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
) -> Result<Vec<StatePatch>> {
    let rx_vfo = decode_vfo("FR", payload)?;

    if profile.id() == "elecraft-k3" {
        return Ok(direct_split_patches(false, RoutingVfo::Main, state));
    }

    let current_split = state.tx.as_ref().and_then(|tx| tx.split);
    let mut patches = Vec::new();

    if let Some(tx_vfo) = routed_tx_vfo(state) {
        patches.push(StatePatch::Split(tx_vfo != rx_vfo));
    } else if current_split == Some(false) {
        patches.push(StatePatch::Split(false));
    }

    if current_split != Some(true) {
        append_tx_from_vfo(&mut patches, rx_vfo, state);
    }

    Ok(patches)
}

fn decode_ft(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    if uses_yaesu_ft(profile) || matches!(profile.id(), "elecraft-k4" | "elecraft-k3") {
        let split = decode_split_digit("FT", payload)?;
        return Ok(direct_split_patches(split, default_tx_vfo(split), state));
    }

    let tx_vfo = decode_vfo("FT", payload)?;
    let mut patches = Vec::new();
    if let Some(rx_vfo) = routed_rx_vfo(state) {
        patches.push(StatePatch::Split(tx_vfo != rx_vfo));
    }
    append_tx_from_vfo(&mut patches, tx_vfo, state);
    Ok(patches)
}

fn decode_direct_split(
    command: &'static str,
    payload: &str,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    let split = decode_split_digit(command, payload)?;
    Ok(direct_split_patches(split, default_tx_vfo(split), state))
}

fn direct_split_patches(split: bool, tx_vfo: RoutingVfo, state: &RadioState) -> Vec<StatePatch> {
    let mut patches = vec![StatePatch::Split(split)];
    append_tx_from_vfo(&mut patches, tx_vfo, state);
    patches
}

fn append_tx_from_vfo(patches: &mut Vec<StatePatch>, tx_vfo: RoutingVfo, state: &RadioState) {
    if let Some(frequency) = frequency_for_vfo(tx_vfo, state) {
        patches.push(StatePatch::TxFrequency(frequency));
    }
    if let Some(mode) = mode_for_vfo(tx_vfo, state) {
        patches.push(StatePatch::TxMode(mode));
    }
}

fn routed_rx_vfo(state: &RadioState) -> Option<RoutingVfo> {
    let split = state.tx.as_ref().and_then(|tx| tx.split);
    let tx_vfo = tx_vfo_from_frequency(state);
    match split {
        Some(true) => tx_vfo.map(RoutingVfo::opposite),
        Some(false) => tx_vfo.or_else(|| unique_known_vfo(state)),
        None => tx_vfo.or_else(|| unique_known_vfo(state)),
    }
}

fn routed_tx_vfo(state: &RadioState) -> Option<RoutingVfo> {
    let split = state.tx.as_ref().and_then(|tx| tx.split);
    let tx_vfo = tx_vfo_from_frequency(state);
    match split {
        Some(true) => tx_vfo.or_else(|| routed_rx_vfo(state).map(RoutingVfo::opposite)),
        Some(false) => tx_vfo.or_else(|| routed_rx_vfo(state)),
        None => tx_vfo,
    }
}

fn tx_vfo_from_frequency(state: &RadioState) -> Option<RoutingVfo> {
    let tx_frequency = state.tx.as_ref().and_then(|tx| tx.frequency)?;
    vfo_for_frequency(state, tx_frequency)
}

fn vfo_for_frequency(state: &RadioState, frequency: Frequency) -> Option<RoutingVfo> {
    let main_matches = state.main_rx.frequency == Some(frequency);
    let sub_matches = state.sub_rx.as_ref().and_then(|rx| rx.frequency) == Some(frequency);
    match (main_matches, sub_matches) {
        (true, false) => Some(RoutingVfo::Main),
        (false, true) => Some(RoutingVfo::Sub),
        _ => None,
    }
}

fn unique_known_vfo(state: &RadioState) -> Option<RoutingVfo> {
    match (
        state.main_rx.frequency.is_some(),
        state.sub_rx.as_ref().and_then(|rx| rx.frequency).is_some(),
    ) {
        (true, false) => Some(RoutingVfo::Main),
        (false, true) => Some(RoutingVfo::Sub),
        _ => None,
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

fn default_tx_vfo(split: bool) -> RoutingVfo {
    if split {
        RoutingVfo::Sub
    } else {
        RoutingVfo::Main
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
        protocol::kenwood_ascii::profile_by_id, Frequency, ReceiverState, TransmitterState,
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
        let state = routed_state(false, Frequency::from_hz(7_074_000));

        let encoded = encode(profile, &RadioCommand::SetSplit(true), &state)
            .unwrap()
            .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "FT0;");
        assert_eq!(
            encoded.optimistic,
            vec![
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
        let state = routed_state(false, Frequency::from_hz(7_074_000));

        let decoded = decode(profile, &AsciiFrame::new("FT0;").unwrap(), &state)
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
    fn routed_split_requires_known_routing() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let error = encode(
            profile,
            &RadioCommand::SetSplit(true),
            &RadioState::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RadioError::InvalidValue { field: "split", .. }
        ));
    }
}
