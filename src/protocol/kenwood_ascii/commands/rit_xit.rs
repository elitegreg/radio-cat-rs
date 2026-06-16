use crate::{
    capabilities::Capability, command::RadioCommand, error::RadioError, update::StatePatch,
    RadioState, Result, RitXitOffsetHz,
};

use super::{
    split::{current_tx_vfo, RoutingVfo},
    DecodedFrame, EncodedCommand,
};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiProfile, ResponseMatcher,
};

pub fn encode(
    profile: &KenwoodAsciiProfile,
    command: &RadioCommand,
    state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetRitEnabled(enabled) => {
            require_writable(
                profile.capabilities.rit_xit.rit_enabled,
                "rit_xit.rit_enabled",
            )?;
            let suffix = target_suffix(profile, state);
            let matcher = if suffix.is_empty() {
                ResponseMatcher::Prefix("RT")
            } else {
                ResponseMatcher::Prefix("RT$")
            };
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new(format!(
                    "RT{suffix}{};",
                    bool_digit(*enabled)
                ))?],
                matcher,
                vec![StatePatch::RitEnabled(*enabled)],
                CommandPriority::Normal,
            )))
        }
        RadioCommand::SetXitEnabled(enabled) => {
            require_writable(
                profile.capabilities.rit_xit.xit_enabled,
                "rit_xit.xit_enabled",
            )?;
            let suffix = target_suffix(profile, state);
            let matcher = if suffix.is_empty() {
                ResponseMatcher::Prefix("XT")
            } else {
                ResponseMatcher::Prefix("XT$")
            };
            Ok(Some(EncodedCommand::new(
                vec![AsciiFrame::new(format!(
                    "XT{suffix}{};",
                    bool_digit(*enabled)
                ))?],
                matcher,
                vec![StatePatch::XitEnabled(*enabled)],
                CommandPriority::Normal,
            )))
        }
        RadioCommand::SetRitXitOffset(target_offset) => {
            require_writable(profile.capabilities.rit_xit.offset, "rit_xit.offset_hz")?;
            if is_k2(profile) {
                return Err(RadioError::UnsupportedCapability {
                    capability: "rit_xit.offset_hz",
                });
            }
            Ok(Some(encode_offset(profile, *target_offset, state)?))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    let (frame, matcher) = match semantic {
        "RT" if profile.capabilities.rit_xit.rit_enabled.can_read() => {
            ("RT;", ResponseMatcher::Prefix("RT"))
        }
        "XT" if profile.capabilities.rit_xit.xit_enabled.can_read() => {
            ("XT;", ResponseMatcher::Prefix("XT"))
        }
        "RF" if uses_rf_offset(profile) && profile.capabilities.rit_xit.offset.can_read() => {
            ("RF;", ResponseMatcher::Prefix("RF"))
        }
        "RO" if uses_ro_offset(profile) && profile.capabilities.rit_xit.offset.can_read() => {
            ("RO;", ResponseMatcher::Prefix("RO"))
        }
        "RT$" if is_k4(profile) && profile.capabilities.rit_xit.rit_enabled.can_read() => {
            ("RT$;", ResponseMatcher::Prefix("RT$"))
        }
        "XT$" if is_k4(profile) && profile.capabilities.rit_xit.xit_enabled.can_read() => {
            ("XT$;", ResponseMatcher::Prefix("XT$"))
        }
        "RO$" if is_k4(profile) && profile.capabilities.rit_xit.offset.can_read() => {
            ("RO$;", ResponseMatcher::Prefix("RO$"))
        }
        _ => return Ok(None),
    };

    Ok(Some(EncodedCommand::new(
        vec![AsciiFrame::new(frame)?],
        matcher,
        Vec::new(),
        CommandPriority::Normal,
    )))
}

pub fn decode(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<Option<DecodedFrame>> {
    let patches = match frame.command() {
        "RT" | "RT$" => vec![StatePatch::RitEnabled(parse_flag("RT", frame.payload())?)],
        "XT" | "XT$" => vec![StatePatch::XitEnabled(parse_flag("XT", frame.payload())?)],
        "RF" | "RFS" if uses_rf_offset(profile) => {
            vec![StatePatch::RitXitOffset(parse_offset(
                "RF",
                frame.payload(),
            )?)]
        }
        "RO" | "ROS" | "RO$" | "RO$S" if uses_ro_offset(profile) => {
            vec![StatePatch::RitXitOffset(parse_offset(
                "RO",
                frame.payload(),
            )?)]
        }
        "RC" | "RC$" => vec![StatePatch::RitXitOffset(RitXitOffsetHz::new(0).unwrap())],
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

fn encode_offset(
    profile: &KenwoodAsciiProfile,
    target_offset: RitXitOffsetHz,
    state: &RadioState,
) -> Result<EncodedCommand> {
    if uses_ro_offset(profile) {
        let suffix = target_suffix(profile, state);
        let (sign, magnitude) = signed_parts(target_offset);
        let matcher = if suffix.is_empty() {
            ResponseMatcher::Prefix("RO")
        } else {
            ResponseMatcher::Prefix("RO$")
        };

        return Ok(EncodedCommand::new(
            vec![AsciiFrame::new(format!("RO{suffix}{sign}{magnitude:04};"))?],
            matcher,
            vec![StatePatch::RitXitOffset(target_offset)],
            CommandPriority::Normal,
        ));
    }

    let mut frames = Vec::new();

    if target_offset.as_hz() == 0 {
        frames.push(AsciiFrame::new("RC;")?);
    } else {
        let current = state.rit_xit.offset_hz.ok_or(RadioError::InvalidValue {
            field: "rit_xit.offset_hz",
            message: format!(
                "cannot compute relative RIT/XIT delta for {}; refresh offset first",
                profile.id()
            ),
        })?;

        append_relative_steps(&mut frames, current, target_offset)?;
    }

    let (confirm_frame, matcher) = confirm_query(profile)?;
    frames.push(confirm_frame);

    Ok(EncodedCommand::new(
        frames,
        matcher,
        vec![StatePatch::RitXitOffset(target_offset)],
        CommandPriority::Normal,
    ))
}

fn append_relative_steps(
    frames: &mut Vec<AsciiFrame>,
    current: RitXitOffsetHz,
    target: RitXitOffsetHz,
) -> Result<()> {
    let delta = target.as_hz() as i32 - current.as_hz() as i32;
    if delta == 0 {
        return Ok(());
    }

    let command = if delta > 0 { "RU" } else { "RD" };
    let mut remaining = delta.unsigned_abs();

    while remaining > 0 {
        let step = remaining.min(9_999);
        frames.push(AsciiFrame::new(format!("{command}{step:04};"))?);
        remaining -= step;
    }

    Ok(())
}

fn confirm_query(profile: &KenwoodAsciiProfile) -> Result<(AsciiFrame, ResponseMatcher)> {
    if uses_rf_offset(profile) {
        Ok((AsciiFrame::new("RF;")?, ResponseMatcher::Prefix("RF")))
    } else if supports_if_offset(profile) {
        Ok((AsciiFrame::new("IF;")?, ResponseMatcher::Prefix("IF")))
    } else {
        Err(RadioError::UnsupportedCapability {
            capability: "rit_xit.offset_hz",
        })
    }
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

fn parse_offset(command: &'static str, payload: &str) -> Result<RitXitOffsetHz> {
    let signed = payload.strip_prefix('S').unwrap_or(payload);
    let value = signed.parse::<i16>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })?;

    RitXitOffsetHz::new(value).map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })
}

fn signed_parts(offset: RitXitOffsetHz) -> (char, u16) {
    let value = offset.as_hz();
    let sign = if value < 0 { '-' } else { '+' };
    (sign, value.unsigned_abs())
}

fn target_suffix(profile: &KenwoodAsciiProfile, state: &RadioState) -> &'static str {
    if !is_k4(profile) {
        return "";
    }

    match current_tx_vfo(profile, state) {
        Some(RoutingVfo::Sub) => "$",
        _ => "",
    }
}

fn require_writable(capability: Capability, field: &'static str) -> Result<()> {
    if capability.can_write() {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability { capability: field })
    }
}

fn bool_digit(value: bool) -> char {
    if value {
        '1'
    } else {
        '0'
    }
}

fn uses_rf_offset(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "kenwood-ts890" | "kenwood-ts990")
}

fn uses_ro_offset(profile: &KenwoodAsciiProfile) -> bool {
    matches!(profile.id(), "elecraft-k4" | "elecraft-k3")
}

fn supports_if_offset(profile: &KenwoodAsciiProfile) -> bool {
    !matches!(
        profile.id(),
        "kenwood-ts890" | "kenwood-ts990" | "elecraft-k4"
    )
}

fn is_k4(profile: &KenwoodAsciiProfile) -> bool {
    profile.id() == "elecraft-k4"
}

fn is_k2(profile: &KenwoodAsciiProfile) -> bool {
    profile.id() == "elecraft-k2"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::kenwood_ascii::profile_by_id;

    #[test]
    fn encodes_enable_commands_with_profile_targeting() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let state = RadioState::default();

        let rit = encode(ts590, &RadioCommand::SetRitEnabled(true), &state)
            .unwrap()
            .unwrap();
        assert_eq!(rit.frames[0].as_str(), "RT1;");

        let xit = encode(ts590, &RadioCommand::SetXitEnabled(false), &state)
            .unwrap()
            .unwrap();
        assert_eq!(xit.frames[0].as_str(), "XT0;");

        let k4 = profile_by_id("elecraft-k4").unwrap();
        let mut split_state = RadioState::default();
        split_state.tx = Some(crate::TransmitterState {
            split: Some(true),
            ..crate::TransmitterState::default()
        });

        let targeted = encode(k4, &RadioCommand::SetRitEnabled(true), &split_state)
            .unwrap()
            .unwrap();
        assert_eq!(targeted.frames[0].as_str(), "RT$1;");
    }

    #[test]
    fn relative_offset_flow_uses_delta_and_confirmation() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let mut state = RadioState::default();
        state.rit_xit.offset_hz = Some(RitXitOffsetHz::new(100).unwrap());

        let encoded = encode(
            ts590,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(250).unwrap()),
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "RU0150;");
        assert_eq!(encoded.frames[1].as_str(), "IF;");
        assert_eq!(encoded.matcher, ResponseMatcher::Prefix("IF"));

        let encoded = encode(
            ts590,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(-200).unwrap()),
            &state,
        )
        .unwrap()
        .unwrap();
        assert_eq!(encoded.frames[0].as_str(), "RD0300;");
    }

    #[test]
    fn relative_offset_flow_chunks_large_deltas() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let mut state = RadioState::default();
        state.rit_xit.offset_hz = Some(RitXitOffsetHz::new(-9_999).unwrap());

        let encoded = encode(
            ts590,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(9_999).unwrap()),
            &state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "RU9999;");
        assert_eq!(encoded.frames[1].as_str(), "RU9999;");
        assert_eq!(encoded.frames[2].as_str(), "IF;");
    }

    #[test]
    fn relative_offset_requires_current_state() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let error = encode(
            ts590,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(50).unwrap()),
            &RadioState::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RadioError::InvalidValue {
                field: "rit_xit.offset_hz",
                ..
            }
        ));
    }

    #[test]
    fn k2_offset_write_is_rejected() {
        let k2 = profile_by_id("elecraft-k2").unwrap();
        let error = encode(
            k2,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(50).unwrap()),
            &RadioState::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RadioError::UnsupportedCapability {
                capability: "rit_xit.offset_hz"
            }
        ));
    }

    #[test]
    fn elecraft_absolute_offset_uses_ro_command_family() {
        let k3 = profile_by_id("elecraft-k3").unwrap();
        let encoded = encode(
            k3,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(-321).unwrap()),
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "RO-0321;");

        let k4 = profile_by_id("elecraft-k4").unwrap();
        let mut split_state = RadioState::default();
        split_state.tx = Some(crate::TransmitterState {
            split: Some(true),
            ..crate::TransmitterState::default()
        });

        let encoded = encode(
            k4,
            &RadioCommand::SetRitXitOffset(RitXitOffsetHz::new(25).unwrap()),
            &split_state,
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.frames[0].as_str(), "RO$+0025;");
    }

    #[test]
    fn query_encoding_matches_profile_command_families() {
        let ts990 = profile_by_id("kenwood-ts990").unwrap();
        assert_eq!(
            encode_query(ts990, "RF").unwrap().unwrap().frames[0].as_str(),
            "RF;"
        );

        let k4 = profile_by_id("elecraft-k4").unwrap();
        assert_eq!(
            encode_query(k4, "RO$").unwrap().unwrap().frames[0].as_str(),
            "RO$;"
        );

        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        assert!(encode_query(ts590, "RO").unwrap().is_none());
    }

    #[test]
    fn decode_handles_flags_and_offset_formats() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let rt = decode(ts590, &AsciiFrame::new("RT1;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(rt.patches, vec![StatePatch::RitEnabled(true)]);

        let ts890 = profile_by_id("kenwood-ts890").unwrap();
        let rf = decode(ts890, &AsciiFrame::new("RFS-0123;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            rf.patches,
            vec![StatePatch::RitXitOffset(RitXitOffsetHz::new(-123).unwrap())]
        );

        let k4 = profile_by_id("elecraft-k4").unwrap();
        let ro = decode(k4, &AsciiFrame::new("RO$+0042;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            ro.patches,
            vec![StatePatch::RitXitOffset(RitXitOffsetHz::new(42).unwrap())]
        );

        let rc = decode(ts590, &AsciiFrame::new("RC;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            rc.patches,
            vec![StatePatch::RitXitOffset(RitXitOffsetHz::new(0).unwrap())]
        );
    }
}
