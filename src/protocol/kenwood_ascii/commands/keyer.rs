use crate::{
    Mode, RadioState, Result, command::RadioCommand, error::RadioError, update::StatePatch,
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
        RadioCommand::SetKeyerSpeed(wpm) => {
            require_keyer_speed(profile)?;
            validate_keyer_speed(profile, *wpm)?;
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new(format!("KS{wpm:03};"))?],
                ResponseMatcher::Prefix("KS"),
                vec![StatePatch::KeyerSpeed(*wpm)],
                CommandPriority::Normal,
            )))
        }
        RadioCommand::SendCw(text) => {
            require_send_cw(profile)?;
            if is_elecraft_k3_or_k4(profile) {
                require_mode(state, "keyer.cw", is_cw_mode, "CW or CW-R")?;
            }
            validate_cw_text(profile, text)?;
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new(format!("KY {text};"))?],
                ResponseMatcher::None,
                Vec::new(),
                CommandPriority::Normal,
            )))
        }
        RadioCommand::StopCw => {
            require_stop_cw(profile)?;
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new(stop_frame(profile))?],
                ResponseMatcher::None,
                Vec::new(),
                CommandPriority::High,
            )))
        }
        RadioCommand::SendData(text) => {
            require_send_data(profile)?;
            require_mode(
                state,
                "keyer.data",
                is_data_mode,
                "RTTY, RTTY-R, PSK, or PSK-R",
            )?;
            validate_data_text(profile, text)?;
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new(format!("KY {text};"))?],
                ResponseMatcher::None,
                Vec::new(),
                CommandPriority::Normal,
            )))
        }
        RadioCommand::StopData => {
            require_stop_data(profile)?;
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new("KY |;")?],
                ResponseMatcher::None,
                Vec::new(),
                CommandPriority::High,
            )))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    match semantic {
        "KS" if profile
            .capabilities
            .keyer
            .is_some_and(|keyer| keyer.speed_wpm.is_supported()) =>
        {
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new("KS;")?],
                ResponseMatcher::Prefix("KS"),
                Vec::new(),
                CommandPriority::Normal,
            )))
        }
        "KY" if supports_ky_query(profile) => Ok(Some(EncodedCommand::new(
            vec![AsciiFrame::new("KY;")?],
            ResponseMatcher::Prefix("KY"),
            Vec::new(),
            CommandPriority::Normal,
        ))),
        _ => Ok(None),
    }
}

pub fn decode(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<Option<DecodedFrame>> {
    match frame.command() {
        "KS" => {
            let wpm = frame
                .payload()
                .parse::<u8>()
                .map_err(|error| RadioError::Decode {
                    command: "KS",
                    message: error.to_string(),
                })?;
            Ok(Some(DecodedFrame::new(vec![StatePatch::KeyerSpeed(wpm)])))
        }
        "KY" if supports_ky_query(profile) => {
            let payload = frame.payload().trim();
            let sending = payload != "0" && !payload.is_empty();
            Ok(Some(DecodedFrame::new(vec![StatePatch::KeyerSending(
                sending,
            )])))
        }
        _ => Ok(None),
    }
}

fn require_keyer_speed(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.keyer {
        Some(keyer) if keyer.speed_wpm.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "keyer.speed_wpm",
        }),
    }
}

fn require_send_cw(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.keyer {
        Some(keyer) if keyer.send_cw.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "keyer.send_cw",
        }),
    }
}

fn require_stop_cw(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.keyer {
        Some(keyer) if keyer.stop_cw.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "keyer.stop_cw",
        }),
    }
}

fn require_send_data(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.keyer {
        Some(keyer) if keyer.send_data.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "keyer.send_data",
        }),
    }
}

fn require_stop_data(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.keyer {
        Some(keyer) if keyer.stop_data.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "keyer.stop_data",
        }),
    }
}

fn require_mode(
    state: &RadioState,
    field: &'static str,
    supported: fn(Mode) -> bool,
    expected: &'static str,
) -> Result<()> {
    match state.tx().and_then(|tx| tx.mode()) {
        Some(mode) if supported(mode) => Ok(()),
        Some(mode) => Err(RadioError::InvalidValue {
            field,
            message: format!("requires {expected} mode, current transmit mode is {mode}"),
        }),
        None => Err(RadioError::InvalidValue {
            field,
            message: format!("requires {expected} mode, but transmit mode is unknown"),
        }),
    }
}

fn is_cw_mode(mode: Mode) -> bool {
    matches!(mode, Mode::Cw | Mode::CwReverse)
}

fn is_data_mode(mode: Mode) -> bool {
    matches!(
        mode,
        Mode::Rtty | Mode::RttyReverse | Mode::Psk | Mode::PskReverse
    )
}

fn validate_keyer_speed(profile: &KenwoodAsciiProfile, wpm: u8) -> Result<()> {
    let (min, max) = keyer_range(profile);
    if (min..=max).contains(&wpm) {
        Ok(())
    } else {
        Err(RadioError::InvalidValue {
            field: "keyer.speed_wpm",
            message: format!("expected {min}..={max} WPM for {}", profile.id()),
        })
    }
}

fn validate_cw_text(profile: &KenwoodAsciiProfile, text: &str) -> Result<()> {
    validate_keyer_text(profile, text, "keyer.cw", "CW", false)
}

fn validate_data_text(profile: &KenwoodAsciiProfile, text: &str) -> Result<()> {
    validate_keyer_text(profile, text, "keyer.data", "data", true)
}

fn validate_keyer_text(
    profile: &KenwoodAsciiProfile,
    text: &str,
    field: &'static str,
    label: &'static str,
    reject_pipe: bool,
) -> Result<()> {
    if text.is_empty() {
        return Err(RadioError::InvalidValue {
            field,
            message: format!("{label} text must not be empty"),
        });
    }
    if text.len() > cw_buffer_limit(profile) {
        return Err(RadioError::InvalidValue {
            field,
            message: format!(
                "{label} text exceeds {} byte buffer for {}",
                cw_buffer_limit(profile),
                profile.id()
            ),
        });
    }
    if !text.chars().all(|ch| ch.is_ascii_graphic() || ch == ' ')
        || text.contains(';')
        || (reject_pipe && text.contains('|'))
    {
        return Err(RadioError::InvalidValue {
            field,
            message: if reject_pipe {
                format!("{label} text must be printable ASCII without semicolons or pipes")
            } else {
                format!("{label} text must be printable ASCII without semicolons")
            },
        });
    }
    Ok(())
}

fn is_elecraft_k3_or_k4(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "elecraft-k3" | "elecraft-k4")
}

fn keyer_range(profile: &KenwoodAsciiProfile) -> (u8, u8) {
    match profile.id() {
        "kenwood-ts590" | "kenwood-ts890" | "kenwood-ts990" => (4, 60),
        "kenwood-ts2000" | "kenwood-ts480" | "kenwood-ts570" | "kenwood-ts870" => (10, 60),
        "elecraft-k4" => (8, 100),
        "elecraft-k3" => (8, 50),
        "elecraft-k2" => (9, 50),
        "yaesu-ftdx101" | "yaesu-ftdx10" | "yaesu-ft710" | "yaesu-ft891" | "yaesu-ft991" => (4, 60),
        _ => (4, 60),
    }
}

fn cw_buffer_limit(profile: &KenwoodAsciiProfile) -> usize {
    match profile.id() {
        "elecraft-k4" => 60,
        _ => 24,
    }
}

fn stop_frame(profile: &KenwoodAsciiProfile) -> &'static str {
    match profile.id() {
        "kenwood-ts590" | "kenwood-ts890" | "kenwood-ts990" => "KY0;",
        "elecraft-k4" | "elecraft-k3" | "elecraft-k2" => "KY @;",
        _ => "RX;",
    }
}

fn supports_ky_query(profile: &KenwoodAsciiProfile) -> bool {
    profile.id() == "elecraft-k4"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StateReducer, protocol::kenwood_ascii::profile_by_id};

    fn state_with_tx_mode(mode: Mode) -> RadioState {
        let mut reducer = StateReducer::new(RadioState::default());
        reducer.apply_patches([StatePatch::TxPresent(true), StatePatch::TxMode(mode)]);
        reducer.state().clone()
    }

    #[test]
    fn keyer_speed_validates_profile_range() {
        let ts2000 = profile_by_id("kenwood-ts2000").unwrap();
        let error = encode(
            ts2000,
            &RadioCommand::SetKeyerSpeed(5),
            &RadioState::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RadioError::InvalidValue {
                field: "keyer.speed_wpm",
                ..
            }
        ));
    }

    #[test]
    fn cw_send_and_stop_use_profile_specific_commands() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let k4 = profile_by_id("elecraft-k4").unwrap();

        let ts590_send = encode(
            ts590,
            &RadioCommand::SendCw("CQ TEST".to_string()),
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(ts590_send.frames[0].as_str(), "KY CQ TEST;");

        let k4_stop = encode(k4, &RadioCommand::StopCw, &RadioState::default())
            .unwrap()
            .unwrap();
        assert_eq!(k4_stop.frames[0].as_str(), "KY @;");
        assert_eq!(k4_stop.priority, CommandPriority::High);
    }

    #[test]
    fn yaesu_cw_send_is_unsupported() {
        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let error = encode(
            yaesu,
            &RadioCommand::SendCw("CQ".to_string()),
            &RadioState::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RadioError::UnsupportedCapability {
                capability: "keyer.send_cw"
            }
        ));
    }

    #[test]
    fn k4_ky_query_updates_keyer_sending() {
        let k4 = profile_by_id("elecraft-k4").unwrap();
        let decoded = decode(k4, &AsciiFrame::new("KY1;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(decoded.patches, vec![StatePatch::KeyerSending(true)]);
    }

    #[test]
    fn elecraft_data_and_cw_sends_require_their_respective_modes() {
        let k4 = profile_by_id("elecraft-k4").unwrap();
        let cw_state = state_with_tx_mode(Mode::CwReverse);
        let data_state = state_with_tx_mode(Mode::Psk);

        let cw = encode(k4, &RadioCommand::SendCw("CQ".to_string()), &cw_state)
            .unwrap()
            .unwrap();
        assert_eq!(cw.frames[0].as_str(), "KY CQ;");

        let data = encode(k4, &RadioCommand::SendData("CQ".to_string()), &data_state)
            .unwrap()
            .unwrap();
        assert_eq!(data.frames[0].as_str(), "KY CQ;");

        let error = encode(k4, &RadioCommand::SendData("CQ".to_string()), &cw_state).unwrap_err();
        assert!(matches!(
            error,
            RadioError::InvalidValue {
                field: "keyer.data",
                ..
            }
        ));

        let error = encode(k4, &RadioCommand::SendCw("CQ".to_string()), &data_state).unwrap_err();
        assert!(matches!(
            error,
            RadioError::InvalidValue {
                field: "keyer.cw",
                ..
            }
        ));
    }

    #[test]
    fn elecraft_data_stop_uses_pipe_and_data_text_rejects_pipe() {
        let k3 = profile_by_id("elecraft-k3").unwrap();
        let state = state_with_tx_mode(Mode::Rtty);
        let stop = encode(k3, &RadioCommand::StopData, &state)
            .unwrap()
            .unwrap();
        assert_eq!(stop.frames[0].as_str(), "KY |;");
        assert_eq!(stop.priority, CommandPriority::High);

        let error = encode(k3, &RadioCommand::SendData("A|B".to_string()), &state).unwrap_err();
        assert!(matches!(
            error,
            RadioError::InvalidValue {
                field: "keyer.data",
                ..
            }
        ));
    }
}
