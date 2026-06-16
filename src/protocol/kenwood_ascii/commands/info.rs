use crate::{
    error::RadioError, update::StatePatch, Frequency, Mode, RadioState, Result, RitXitOffsetHz,
};

use super::DecodedFrame;
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, EncodedCommand, KenwoodAsciiProfile, ReceiverKind, ResponseMatcher,
};

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    if semantic != "IF" || !supports_generic_if(profile) {
        return Ok(None);
    }

    Ok(Some(EncodedCommand::new(
        vec![AsciiFrame::new("IF;")?],
        ResponseMatcher::Prefix("IF"),
        Vec::new(),
        CommandPriority::Normal,
    )))
}

pub fn decode(
    profile: &KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
) -> Result<Option<DecodedFrame>> {
    if frame.command() != "IF" {
        return Ok(None);
    }

    if !supports_generic_if(profile) {
        return Ok(None);
    }

    let payload = frame.payload();
    let patches = if is_yaesu(profile) {
        decode_yaesu_if(profile, payload, state)?
    } else {
        decode_kenwood_if(profile, payload, state)?
    };

    Ok(Some(DecodedFrame::new(patches)))
}

fn decode_kenwood_if(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
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
    let mode = decode_kenwood_mode(profile, payload.as_bytes()[27] as char)?;
    let active_vfo = decode_active_vfo(payload.as_bytes()[28] as char)?;
    let split = parse_bool_digit("IF", payload.as_bytes()[29])?;

    let mut patches = vec![
        StatePatch::RitXitOffset(offset),
        StatePatch::RitEnabled(rit_enabled),
        StatePatch::XitEnabled(xit_enabled),
        StatePatch::Transmitting(transmitting),
        StatePatch::Split(split),
    ];

    match active_vfo {
        ActiveVfo::A => {
            patches.push(StatePatch::MainRxFrequency(frequency));
            patches.push(StatePatch::MainRxMode(mode));
            if !split {
                patches.push(StatePatch::TxFrequency(frequency));
                patches.push(StatePatch::TxMode(mode));
            } else if state.tx.as_ref().and_then(|tx| tx.frequency).is_none() {
                patches.push(StatePatch::TxFrequency(
                    state
                        .sub_rx
                        .as_ref()
                        .and_then(|rx| rx.frequency)
                        .unwrap_or(frequency),
                ));
            }
        }
        ActiveVfo::B => {
            patches.push(StatePatch::SubRxPresent(true));
            patches.push(StatePatch::SubRxFrequency(frequency));
            patches.push(StatePatch::SubRxMode(mode));
            if !split {
                patches.push(StatePatch::TxFrequency(frequency));
                patches.push(StatePatch::TxMode(mode));
            } else if state.tx.as_ref().and_then(|tx| tx.frequency).is_none() {
                patches.push(StatePatch::TxFrequency(
                    state.main_rx.frequency.unwrap_or(frequency),
                ));
            }
        }
    }

    Ok(patches)
}

fn decode_yaesu_if(
    profile: &KenwoodAsciiProfile,
    payload: &str,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    if payload.len() != 25 {
        return Err(RadioError::Decode {
            command: "IF",
            message: format!(
                "expected 25-character Yaesu IF payload, got {}",
                payload.len()
            ),
        });
    }

    let frequency = parse_frequency("IF", &payload[3..12])?;
    let offset = parse_signed_offset("IF", &payload[12..17])?;
    let rit_enabled = parse_bool_digit("IF", payload.as_bytes()[17])?;
    let xit_enabled = parse_bool_digit("IF", payload.as_bytes()[18])?;
    let mode = decode_yaesu_mode(profile, payload.as_bytes()[19] as char)?;
    let split = decode_yaesu_split(payload.as_bytes()[24] as char)?;

    let mut patches = vec![
        StatePatch::RitXitOffset(offset),
        StatePatch::RitEnabled(rit_enabled),
        StatePatch::XitEnabled(xit_enabled),
        StatePatch::Split(split),
    ];

    patches.push(StatePatch::MainRxFrequency(frequency));
    patches.push(StatePatch::MainRxMode(mode));
    if matches!(profile.receiver_kind, ReceiverKind::DualRx) {
        patches.push(StatePatch::SubRxPresent(true));
    }

    if !split {
        patches.push(StatePatch::TxFrequency(frequency));
        patches.push(StatePatch::TxMode(mode));
    } else if state.tx.as_ref().and_then(|tx| tx.frequency).is_none() {
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

fn decode_kenwood_mode(profile: &KenwoodAsciiProfile, code: char) -> Result<Mode> {
    let mode = match code {
        '1' => Mode::Lsb,
        '2' => Mode::Usb,
        '3' => Mode::Cw,
        '4' => Mode::Fm,
        '5' => Mode::Am,
        '6' => Mode::Rtty,
        '7' => Mode::CwReverse,
        '9' => Mode::RttyReverse,
        'C' if profile.id() == "kenwood-ts590" => Mode::DataLsb,
        'D' if profile.id() == "kenwood-ts590" => Mode::DataUsb,
        'E' if profile.id() == "kenwood-ts590" => Mode::DataFm,
        other => {
            return Err(RadioError::Decode {
                command: "IF",
                message: format!(
                    "unsupported Kenwood mode code {other:?} for {}",
                    profile.id()
                ),
            })
        }
    };

    Ok(mode)
}

fn decode_yaesu_mode(profile: &KenwoodAsciiProfile, code: char) -> Result<Mode> {
    let mode = match code {
        '1' => Mode::Lsb,
        '2' => Mode::Usb,
        '3' => Mode::Cw,
        '4' => Mode::Fm,
        '5' => Mode::Am,
        '6' => Mode::Rtty,
        '7' => Mode::CwReverse,
        '8' => Mode::DataLsb,
        '9' => Mode::RttyReverse,
        'A' => Mode::DataFm,
        'B' => Mode::Fm,
        'C' => Mode::DataUsb,
        'D' => Mode::Am,
        'E' if profile.id() == "yaesu-ft991" => Mode::Digital,
        'E' => Mode::Digital,
        'F' => Mode::DataFm,
        other => {
            return Err(RadioError::Decode {
                command: "IF",
                message: format!("unsupported Yaesu mode code {other:?} for {}", profile.id()),
            })
        }
    };

    Ok(mode)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveVfo {
    A,
    B,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::kenwood_ascii::profile_by_id, TransmitterState};

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

        let decoded = decode(profile, &frame, &state).unwrap().unwrap();
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::RitXitOffset(RitXitOffsetHz::new(123).unwrap()),
                StatePatch::RitEnabled(true),
                StatePatch::XitEnabled(false),
                StatePatch::Transmitting(true),
                StatePatch::Split(false),
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

        let decoded = decode(profile, &frame, &state).unwrap().unwrap();
        assert_eq!(
            decoded.patches[0],
            StatePatch::RitXitOffset(RitXitOffsetHz::new(-251).unwrap())
        );
        assert!(decoded.patches.contains(&StatePatch::SubRxPresent(true)));
        assert!(decoded
            .patches
            .contains(&StatePatch::SubRxFrequency(Frequency::from_hz(7_074_000))));
        assert!(decoded.patches.contains(&StatePatch::Split(true)));
        assert!(decoded
            .patches
            .contains(&StatePatch::TxFrequency(Frequency::from_hz(14_074_000))));
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

        let decoded = decode(profile, &frame, &state).unwrap().unwrap();
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::RitXitOffset(RitXitOffsetHz::new(500).unwrap()),
                StatePatch::RitEnabled(true),
                StatePatch::XitEnabled(false),
                StatePatch::Split(true),
                StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000)),
                StatePatch::MainRxMode(Mode::Usb),
                StatePatch::TxFrequency(Frequency::from_hz(14_074_000)),
            ]
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

        let decoded = decode(profile, &frame, &state).unwrap().unwrap();
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxMode(Mode::Digital)));
    }
}
