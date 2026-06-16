use crate::{
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::StatePatch,
    Frequency, RadioState, Result,
};

use super::{DecodedFrame, EncodedCommand, FrequencyCommandTarget};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, FrequencyFormat, KenwoodAsciiProfile, ResponseMatcher,
};

pub fn encode(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
    state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetReceiverFrequency {
            receiver,
            frequency,
        } => Ok(Some(encode_targeted_frequency(
            profile,
            receiver_target(*receiver),
            *frequency,
            frequency_optimistic_patch(receiver_target(*receiver), *frequency, state),
        )?)),
        RadioCommand::SetTxFrequency(frequency) => {
            let target = tx_target_from_state(state);
            let optimistic = frequency_optimistic_patch(target, *frequency, state);
            Ok(Some(encode_targeted_frequency(
                profile, target, *frequency, optimistic,
            )?))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    _profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    let target = match semantic {
        "FA" => FrequencyCommandTarget::Main,
        "FB" => FrequencyCommandTarget::Sub,
        _ => return Ok(None),
    };

    let command = command_for_target(target);
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
    let target = match frame.command() {
        "FA" => FrequencyCommandTarget::Main,
        "FB" => FrequencyCommandTarget::Sub,
        _ => return Ok(None),
    };

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

    let mut patches = frequency_decode_patches(target, frequency, state);
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
    target: FrequencyCommandTarget,
    frequency: Frequency,
    state: &RadioState,
) -> Vec<StatePatch> {
    frequency_decode_patches(target, frequency, state)
}

fn frequency_decode_patches(
    target: FrequencyCommandTarget,
    frequency: Frequency,
    state: &RadioState,
) -> Vec<StatePatch> {
    let mut patches = vec![match target {
        FrequencyCommandTarget::Main => StatePatch::MainRxFrequency(frequency),
        FrequencyCommandTarget::Sub => StatePatch::SubRxFrequency(frequency),
    }];

    if matches!(target, FrequencyCommandTarget::Main)
        && !state.tx.as_ref().and_then(|tx| tx.split).unwrap_or(false)
    {
        patches.push(StatePatch::TxFrequency(frequency));
    }

    if matches!(target, FrequencyCommandTarget::Sub)
        && state.tx.as_ref().and_then(|tx| tx.split).unwrap_or(false)
    {
        patches.push(StatePatch::TxFrequency(frequency));
    }

    patches
}

fn receiver_target(receiver: ReceiverPath) -> FrequencyCommandTarget {
    match receiver {
        ReceiverPath::Main => FrequencyCommandTarget::Main,
        ReceiverPath::Sub => FrequencyCommandTarget::Sub,
    }
}

fn tx_target_from_state(state: &RadioState) -> FrequencyCommandTarget {
    if state.tx.as_ref().and_then(|tx| tx.split).unwrap_or(false) {
        FrequencyCommandTarget::Sub
    } else {
        FrequencyCommandTarget::Main
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
    use crate::{protocol::kenwood_ascii::profile_by_id, TransmitterState};

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
        let mut state = RadioState::default();
        state.tx = Some(TransmitterState {
            split: Some(true),
            ..TransmitterState::default()
        });

        let encoded = encode(
            profile,
            &RadioCommand::SetTxFrequency(Frequency::from_hz(7_074_000)),
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "FB00007074000;");
        assert_eq!(
            encoded.optimistic,
            vec![
                StatePatch::SubRxFrequency(Frequency::from_hz(7_074_000)),
                StatePatch::TxFrequency(Frequency::from_hz(7_074_000)),
            ]
        );
    }

    #[test]
    fn decodes_frequency_frames_into_receiver_and_tx_state() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut state = RadioState::default();
        state.tx = Some(TransmitterState {
            split: Some(false),
            ..TransmitterState::default()
        });

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
}
