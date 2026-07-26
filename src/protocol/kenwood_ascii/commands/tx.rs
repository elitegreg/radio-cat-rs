use crate::{
    command::RadioCommand, error::RadioError, update::StatePatch, Capability, Power, Result,
};

use super::{DecodedFrame, EncodedCommand, PowerCommandEncoding};
use crate::protocol::kenwood_ascii::{
    AsciiFrame, CommandPriority, KenwoodAsciiOptions, KenwoodAsciiProfile, KenwoodPttSource,
    ResponseMatcher,
};

pub fn encode(
    profile: &KenwoodAsciiProfile,
    options: KenwoodAsciiOptions,
    command: &RadioCommand,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetTxPower(power) => {
            require_tx_power(profile)?;
            let accepted = profile
                .capabilities
                .tx
                .and_then(|tx| tx.power.quantize(*power))
                .ok_or_else(|| RadioError::InvalidValue {
                    field: "tx.power",
                    message: format!("{} is outside the supported power ranges", power),
                })?;
            let encoding = encode_power_value(profile, accepted)?;
            let frame = match encoding {
                PowerCommandEncoding::StandardWatts { watts }
                | PowerCommandEncoding::K4High { watts } => {
                    if is_k4(profile) {
                        AsciiFrame::new(format!("PC{watts:03}H;"))?
                    } else {
                        AsciiFrame::new(format!("PC{watts:03};"))?
                    }
                }
                PowerCommandEncoding::K4Low { deci_watts } => {
                    AsciiFrame::new(format!("PC{deci_watts:03}L;"))?
                }
                PowerCommandEncoding::K4Milli { deci_milliwatts } => {
                    AsciiFrame::new(format!("PC{deci_milliwatts:03}X;"))?
                }
            };

            Ok(Some(EncodedCommand::new(
                vec![frame],
                ResponseMatcher::Prefix("PC"),
                vec![StatePatch::TxPower(accepted)],
                CommandPriority::Normal,
            )))
        }
        RadioCommand::SetPtt(transmitting) => {
            require_ptt(profile)?;
            let frame = AsciiFrame::new(ptt_frame(profile, options, *transmitting))?;
            Ok(Some(EncodedCommand::new(
                vec![frame],
                ResponseMatcher::None,
                vec![StatePatch::Transmitting(*transmitting)],
                CommandPriority::Normal,
            )))
        }
        RadioCommand::SetDataPtt(transmitting) => {
            require_ptt(profile)?;
            let frame = AsciiFrame::new(data_ptt_frame(profile, *transmitting))?;
            Ok(Some(EncodedCommand::new(
                vec![frame],
                ResponseMatcher::None,
                vec![StatePatch::Transmitting(*transmitting)],
                CommandPriority::Normal,
            )))
        }
        _ => Ok(None),
    }
}

pub fn encode_query(
    profile: &KenwoodAsciiProfile,
    semantic: &str,
) -> Result<Option<EncodedCommand>> {
    if semantic != "PC"
        || profile
            .capabilities
            .tx
            .map(|tx| tx.power.access)
            .unwrap_or(Capability::Unsupported)
            == Capability::Unsupported
    {
        return Ok(None);
    }

    Ok(Some(EncodedCommand::new(
        vec![AsciiFrame::new("PC;")?],
        ResponseMatcher::Prefix("PC"),
        Vec::new(),
        CommandPriority::Normal,
    )))
}

pub fn decode(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<Option<DecodedFrame>> {
    match frame.command() {
        "PC" => decode_power_frame(profile, frame).map(Some),
        command if command.starts_with("TX") || command == "RX" => {
            decode_ptt_frame(profile, frame).map(Some)
        }
        _ => Ok(None),
    }
}

fn decode_power_frame(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<DecodedFrame> {
    let payload = frame.payload();
    let power = if is_k4(profile) && payload.len() == 4 {
        let (digits, suffix) = payload.split_at(3);
        let value = digits.parse::<u16>().map_err(|error| RadioError::Decode {
            command: "PC",
            message: error.to_string(),
        })?;
        match suffix {
            "H" => Power::checked_from_watts(u64::from(value)).expect("u16 watts fit in Power"),
            "L" => Power::checked_from_milliwatts(u64::from(value) * 100)
                .expect("u16 deciwatts fit in Power"),
            "X" => Power::from_microwatts(u64::from(value) * 100),
            _ => {
                return Err(RadioError::Decode {
                    command: "PC",
                    message: format!("unknown K4 power suffix {suffix:?}"),
                })
            }
        }
    } else {
        let watts = payload.parse::<u16>().map_err(|error| RadioError::Decode {
            command: "PC",
            message: error.to_string(),
        })?;
        Power::checked_from_watts(u64::from(watts)).expect("u16 watts fit in Power")
    };

    Ok(DecodedFrame::new(vec![StatePatch::TxPower(power)]))
}

fn decode_ptt_frame(profile: &KenwoodAsciiProfile, frame: &AsciiFrame) -> Result<DecodedFrame> {
    let transmitting = if is_yaesu(profile) {
        match frame.as_str() {
            "TX0;" | "RX;" => false,
            "TX1;" | "TX2;" => true,
            _ => {
                return Err(RadioError::Decode {
                    command: "TX",
                    message: format!("unexpected Yaesu TX/RX frame {:?}", frame.as_str()),
                })
            }
        }
    } else {
        match frame.as_str() {
            "RX;" => false,
            "TX;" | "TX0;" | "TX1;" | "TX2;" => true,
            _ => {
                return Err(RadioError::Decode {
                    command: "TX",
                    message: format!("unexpected TX/RX frame {:?}", frame.as_str()),
                })
            }
        }
    };

    Ok(DecodedFrame::new(vec![StatePatch::Transmitting(
        transmitting,
    )]))
}

fn require_tx_power(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.tx {
        Some(tx) if tx.power.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "tx.power",
        }),
    }
}

fn require_ptt(profile: &KenwoodAsciiProfile) -> Result<()> {
    match profile.capabilities.tx {
        Some(tx) if tx.ptt.is_supported() => Ok(()),
        _ => Err(RadioError::UnsupportedCapability {
            capability: "tx.ptt",
        }),
    }
}

fn encode_power_value(profile: &KenwoodAsciiProfile, power: Power) -> Result<PowerCommandEncoding> {
    if is_k4(profile) {
        return encode_k4_power(power);
    }

    let watts =
        u16::try_from(power.as_microwatts() / Power::MICROWATTS_PER_WATT).map_err(|_| {
            RadioError::InvalidValue {
                field: "tx.power",
                message: format!("{} cannot be encoded by {}", power, profile.id()),
            }
        })?;

    Ok(PowerCommandEncoding::StandardWatts { watts })
}

fn encode_k4_power(power: Power) -> Result<PowerCommandEncoding> {
    let microwatts = power.as_microwatts();
    if microwatts <= 10_000 {
        return Ok(PowerCommandEncoding::K4Milli {
            deci_milliwatts: (microwatts / 100) as u16,
        });
    }
    if microwatts <= 10_000_000 {
        return Ok(PowerCommandEncoding::K4Low {
            deci_watts: (microwatts / 100_000) as u16,
        });
    }

    Ok(PowerCommandEncoding::K4High {
        watts: (microwatts / Power::MICROWATTS_PER_WATT) as u16,
    })
}

fn ptt_frame(
    profile: &KenwoodAsciiProfile,
    options: KenwoodAsciiOptions,
    transmitting: bool,
) -> &'static str {
    if is_yaesu(profile) {
        if transmitting {
            "TX1;"
        } else {
            "TX0;"
        }
    } else if is_split_ptt_kenwood(profile) {
        if transmitting {
            match options.ptt_source {
                KenwoodPttSource::Front => "TX0;",
                KenwoodPttSource::Usb => "TX1;",
            }
        } else {
            "RX;"
        }
    } else if transmitting {
        "TX;"
    } else {
        "RX;"
    }
}

fn data_ptt_frame(profile: &KenwoodAsciiProfile, transmitting: bool) -> &'static str {
    if is_split_ptt_kenwood(profile) {
        if transmitting {
            "TX1;"
        } else {
            "RX;"
        }
    } else {
        ptt_frame(profile, KenwoodAsciiOptions::defaults(), transmitting)
    }
}

fn is_split_ptt_kenwood(profile: &KenwoodAsciiProfile) -> bool {
    matches!(
        profile.id(),
        "kenwood-ts590" | "kenwood-ts890" | "kenwood-ts990" | "kenwood-ts480" | "kenwood-ts2000"
    )
}

fn is_yaesu(profile: &KenwoodAsciiProfile) -> bool {
    profile.id().starts_with("yaesu-")
}

fn is_k4(profile: &KenwoodAsciiProfile) -> bool {
    profile.id() == "elecraft-k4"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::kenwood_ascii::profile_by_id;

    #[test]
    fn standard_power_encodes_as_zero_padded_watts() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let encoded = encode(
            profile,
            KenwoodAsciiOptions::defaults(),
            &RadioCommand::SetTxPower(Power::from_watts(25)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(encoded.frames[0].as_str(), "PC025;");
    }

    #[test]
    fn standard_power_reports_quantized_value_and_rejects_out_of_range() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let encoded = encode(
            profile,
            KenwoodAsciiOptions::defaults(),
            &RadioCommand::SetTxPower(Power::from_microwatts(25_500_000)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(encoded.frames[0].as_str(), "PC026;");
        assert_eq!(
            encoded.completion_patches,
            vec![StatePatch::TxPower(Power::from_watts(26))]
        );

        let error = encode(
            profile,
            KenwoodAsciiOptions::defaults(),
            &RadioCommand::SetTxPower(Power::from_microwatts(4_900_000)),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RadioError::InvalidValue {
                field: "tx.power",
                ..
            }
        ));
    }

    #[test]
    fn k4_power_preserves_low_range_precision() {
        let profile = profile_by_id("elecraft-k4").unwrap();
        let low = encode(
            profile,
            KenwoodAsciiOptions::defaults(),
            &RadioCommand::SetTxPower(Power::from_milliwatts(1000)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(low.frames[0].as_str(), "PC010L;");

        let micro = encode(
            profile,
            KenwoodAsciiOptions::defaults(),
            &RadioCommand::SetTxPower(Power::from_microwatts(500)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(micro.frames[0].as_str(), "PC005X;");

        let gap = encode(
            profile,
            KenwoodAsciiOptions::defaults(),
            &RadioCommand::SetTxPower(Power::from_milliwatts(50)),
        )
        .unwrap_err();
        assert!(matches!(
            gap,
            RadioError::InvalidValue {
                field: "tx.power",
                ..
            }
        ));
    }

    #[test]
    fn ptt_mapping_differs_by_family() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let k3 = profile_by_id("elecraft-k3").unwrap();

        assert_eq!(
            encode(
                ts590,
                KenwoodAsciiOptions::defaults(),
                &RadioCommand::SetPtt(true)
            )
            .unwrap()
            .unwrap()
            .frames[0]
                .as_str(),
            "TX0;"
        );
        assert_eq!(
            encode(
                k3,
                KenwoodAsciiOptions::defaults(),
                &RadioCommand::SetPtt(false)
            )
            .unwrap()
            .unwrap()
            .frames[0]
                .as_str(),
            "RX;"
        );
    }

    #[test]
    fn yaesu_ptt_always_uses_tx1_for_on_and_tx0_for_off() {
        for id in [
            "yaesu-ftdx101",
            "yaesu-ftdx10",
            "yaesu-ft710",
            "yaesu-ft891",
            "yaesu-ft991",
        ] {
            let profile = profile_by_id(id).unwrap();
            assert_eq!(
                encode(
                    profile,
                    KenwoodAsciiOptions::defaults(),
                    &RadioCommand::SetPtt(true),
                )
                .unwrap()
                .unwrap()
                .frames[0]
                    .as_str(),
                "TX1;",
                "{id} should use TX1 for PTT on"
            );
            assert_eq!(
                encode(
                    profile,
                    KenwoodAsciiOptions::defaults(),
                    &RadioCommand::SetPtt(false),
                )
                .unwrap()
                .unwrap()
                .frames[0]
                    .as_str(),
                "TX0;",
                "{id} should use TX0 for PTT off"
            );
        }
    }

    #[test]
    fn set_data_ptt_and_ptt_source_behave_as_expected() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let usb = KenwoodAsciiOptions {
            ptt_source: KenwoodPttSource::Usb,
            ..KenwoodAsciiOptions::defaults()
        };

        assert_eq!(
            encode(ts590, usb, &RadioCommand::SetPtt(true))
                .unwrap()
                .unwrap()
                .frames[0]
                .as_str(),
            "TX1;"
        );
        assert_eq!(
            encode(
                ts590,
                KenwoodAsciiOptions::defaults(),
                &RadioCommand::SetDataPtt(true),
            )
            .unwrap()
            .unwrap()
            .frames[0]
                .as_str(),
            "TX1;"
        );
    }

    #[test]
    fn decode_power_and_ptt_frames() {
        let ts590 = profile_by_id("kenwood-ts590").unwrap();
        let power = decode(ts590, &AsciiFrame::new("PC100;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            power.patches,
            vec![StatePatch::TxPower(Power::from_watts(100))]
        );

        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let ptt = decode(yaesu, &AsciiFrame::new("TX0;").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(ptt.patches, vec![StatePatch::Transmitting(false)]);
    }
}
