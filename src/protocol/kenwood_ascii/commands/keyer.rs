use crate::{command::RadioCommand, error::RadioError, update::StatePatch, Result};

use super::{DecodedFrame, EncodedCommand};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiProfile, ResponseMatcher,
};

pub fn encode(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
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
    if text.is_empty() {
        return Err(RadioError::InvalidValue {
            field: "keyer.cw",
            message: "CW text must not be empty".to_string(),
        });
    }
    if text.len() > cw_buffer_limit(profile) {
        return Err(RadioError::InvalidValue {
            field: "keyer.cw",
            message: format!(
                "CW text exceeds {} byte buffer for {}",
                cw_buffer_limit(profile),
                profile.id()
            ),
        });
    }
    if !text.chars().all(|ch| ch.is_ascii_graphic() || ch == ' ') || text.contains(';') {
        return Err(RadioError::InvalidValue {
            field: "keyer.cw",
            message: "CW text must be printable ASCII without semicolons".to_string(),
        });
    }
    Ok(())
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
    use crate::protocol::kenwood_ascii::profile_by_id;

    #[test]
    fn keyer_speed_validates_profile_range() {
        let ts2000 = profile_by_id("kenwood-ts2000").unwrap();
        let error = encode(ts2000, &RadioCommand::SetKeyerSpeed(5)).unwrap_err();
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

        let ts590_send = encode(ts590, &RadioCommand::SendCw("CQ TEST".to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(ts590_send.frames[0].as_str(), "KY CQ TEST;");

        let k4_stop = encode(k4, &RadioCommand::StopCw).unwrap().unwrap();
        assert_eq!(k4_stop.frames[0].as_str(), "KY @;");
        assert_eq!(k4_stop.priority, CommandPriority::High);
    }

    #[test]
    fn yaesu_cw_send_is_unsupported() {
        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let error = encode(yaesu, &RadioCommand::SendCw("CQ".to_string())).unwrap_err();
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
}
