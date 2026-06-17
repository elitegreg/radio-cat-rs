use std::time::Duration;

use crate::{
    capabilities::{
        Capability, KeyerCapabilities, RadioCapabilities, ReceiverCapabilities, ReceiverKind,
        ReceiverRfCapabilities, RitXitCapabilities, StateUpdateCapability, TransmitterCapabilities,
    },
    driver::DriverDescriptor,
    error::{RadioError, Result},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupStep {
    Query(&'static str),
}

impl StartupStep {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Query(label) => label,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PollPlan {
    pub queries: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcomCivProfile {
    pub descriptor: DriverDescriptor,
    pub default_radio_address: u8,
    pub default_controller_address: u8,
    pub max_tx_power_watts: u16,
    pub capabilities: RadioCapabilities,
    pub startup: &'static [StartupStep],
    pub poll: Option<PollPlan>,
}

impl IcomCivProfile {
    pub const fn id(self) -> &'static str {
        self.descriptor.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcomCivOptions {
    pub radio_address: u8,
    pub controller_address: u8,
    pub mode_filter: u8,
    pub poll_interval: Duration,
}

impl IcomCivOptions {
    pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
    pub const MIN_POLL_INTERVAL: Duration = Duration::from_millis(50);
    pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(5);

    pub fn defaults(profile: &IcomCivProfile) -> Self {
        Self {
            radio_address: profile.default_radio_address,
            controller_address: profile.default_controller_address,
            mode_filter: 1,
            poll_interval: Self::DEFAULT_POLL_INTERVAL,
        }
    }

    pub fn parse(profile: &IcomCivProfile, options: &str) -> Result<Self> {
        let mut parsed = Self::defaults(profile);

        for part in options.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| RadioError::InvalidValue {
                    field: "options",
                    message: format!("expected key=value option, got {part:?}"),
                })?;
            let key = key.trim().replace('-', "_").to_ascii_lowercase();
            let value = value.trim();

            match key.as_str() {
                "radio_address" | "radio_addr" | "addr" => {
                    parsed.radio_address = parse_u8(value, "radio_address")?;
                }
                "controller_address" | "controller_addr" => {
                    parsed.controller_address = parse_u8(value, "controller_address")?;
                }
                "mode_filter" | "filter" => {
                    parsed.mode_filter = parse_filter(value)?;
                }
                "poll_interval" | "poll_interval_s" | "poll" => {
                    parsed.poll_interval = parse_poll_interval(value)?;
                }
                _ => {
                    return Err(RadioError::InvalidValue {
                        field: "options",
                        message: format!("unknown ICOM CI-V option {key:?}"),
                    });
                }
            }
        }

        Ok(parsed)
    }
}

const RW: Capability = Capability::ReadWrite;
const WO: Capability = Capability::WriteOnly;
const UNSUPPORTED: Capability = Capability::Unsupported;

const MAIN_RF: ReceiverRfCapabilities = ReceiverRfCapabilities::new(RW, RW, RW, RW, RW);
const NO_RF: ReceiverRfCapabilities = ReceiverRfCapabilities::new(
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
);

const IC705_MAIN_RX: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, RW, UNSUPPORTED, MAIN_RF);
const IC705_SUB_RX: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, UNSUPPORTED, UNSUPPORTED, NO_RF);
const IC705_TX: TransmitterCapabilities = TransmitterCapabilities::new(RW, RW, RW, RW, RW);
const IC705_RIT_XIT: RitXitCapabilities =
    RitXitCapabilities::new(RW, UNSUPPORTED, RW, RW, UNSUPPORTED);
const IC705_KEYER: KeyerCapabilities = KeyerCapabilities::new(RW, UNSUPPORTED, WO, WO);

const IC705_STARTUP: &[StartupStep] = &[
    StartupStep::Query("freq-main"),
    StartupStep::Query("freq-sub"),
    StartupStep::Query("mode-main"),
    StartupStep::Query("mode-sub"),
    StartupStep::Query("tx-frequency"),
    StartupStep::Query("ptt"),
    StartupStep::Query("split"),
    StartupStep::Query("rit-offset"),
    StartupStep::Query("rit"),
    StartupStep::Query("xit"),
    StartupStep::Query("filter-bandwidth"),
    StartupStep::Query("preamp"),
    StartupStep::Query("attenuator"),
    StartupStep::Query("noise-blanker"),
    StartupStep::Query("noise-reduction"),
    StartupStep::Query("auto-notch"),
    StartupStep::Query("tx-power"),
    StartupStep::Query("keyer-speed"),
];

const IC705_POLL_QUERIES: &[&str] = &[
    "freq-main",
    "freq-sub",
    "mode-main",
    "mode-sub",
    "tx-frequency",
    "ptt",
    "split",
    "rit-offset",
    "rit",
    "xit",
    "filter-bandwidth",
    "preamp",
    "attenuator",
    "noise-blanker",
    "noise-reduction",
    "auto-notch",
    "tx-power",
    "keyer-speed",
];

const fn descriptor(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> DriverDescriptor {
    DriverDescriptor {
        id,
        display_name,
        description,
    }
}

pub const SUPPORTED_PROFILES: &[IcomCivProfile] = &[IcomCivProfile {
    descriptor: descriptor(
        "icom-ic705",
        "Icom IC-705",
        "Icom CI-V profile for IC-705 radios.",
    ),
    default_radio_address: 0xa4,
    default_controller_address: 0xe0,
    max_tx_power_watts: 10,
    capabilities: RadioCapabilities::new(
        ReceiverKind::DualVfo,
        IC705_MAIN_RX,
        Some(IC705_SUB_RX),
        Some(IC705_TX),
        IC705_RIT_XIT,
        Some(IC705_KEYER),
        StateUpdateCapability::Polling,
    ),
    startup: IC705_STARTUP,
    poll: Some(PollPlan {
        queries: IC705_POLL_QUERIES,
    }),
}];

pub fn profile_by_id(id: &str) -> Option<&'static IcomCivProfile> {
    SUPPORTED_PROFILES
        .iter()
        .find(|profile| profile.descriptor.id.eq_ignore_ascii_case(id))
}

fn parse_u8(value: &str, field: &'static str) -> Result<u8> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u8::from_str_radix(hex, 16)
    } else {
        value.parse::<u8>()
    };

    parsed.map_err(|error| RadioError::InvalidValue {
        field,
        message: error.to_string(),
    })
}

fn parse_filter(value: &str) -> Result<u8> {
    match parse_u8(value, "mode_filter")? {
        filter @ 1..=3 => Ok(filter),
        other => Err(RadioError::InvalidValue {
            field: "mode_filter",
            message: format!("expected 1, 2, or 3, got {other}"),
        }),
    }
}

fn parse_poll_interval(value: &str) -> Result<Duration> {
    let seconds = value
        .parse::<f64>()
        .map_err(|error| RadioError::InvalidValue {
            field: "poll_interval",
            message: error.to_string(),
        })?;
    if !seconds.is_finite() {
        return Err(RadioError::InvalidValue {
            field: "poll_interval",
            message: "expected a finite second value".to_string(),
        });
    }

    let interval = Duration::from_secs_f64(seconds);
    if !(IcomCivOptions::MIN_POLL_INTERVAL..=IcomCivOptions::MAX_POLL_INTERVAL).contains(&interval)
    {
        return Err(RadioError::InvalidValue {
            field: "poll_interval",
            message: "expected value from 0.05 through 5 seconds".to_string(),
        });
    }

    Ok(interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_ic705_values() {
        let profile = profile_by_id("icom-ic705").unwrap();
        let options = IcomCivOptions::parse(profile, "").unwrap();

        assert_eq!(options.radio_address, 0xa4);
        assert_eq!(options.controller_address, 0xe0);
        assert_eq!(options.mode_filter, 1);
        assert_eq!(options.poll_interval, Duration::from_millis(200));
    }

    #[test]
    fn options_parse_hex_addresses_and_poll_interval() {
        let profile = profile_by_id("icom-ic705").unwrap();
        let options = IcomCivOptions::parse(
            profile,
            "radio_address=0x94,controller_address=0xe1,mode_filter=2,poll_interval=0.5",
        )
        .unwrap();

        assert_eq!(options.radio_address, 0x94);
        assert_eq!(options.controller_address, 0xe1);
        assert_eq!(options.mode_filter, 2);
        assert_eq!(options.poll_interval, Duration::from_millis(500));
    }

    #[test]
    fn options_reject_out_of_range_poll_interval() {
        let profile = profile_by_id("icom-ic705").unwrap();
        assert!(IcomCivOptions::parse(profile, "poll_interval=0.01").is_err());
        assert!(IcomCivOptions::parse(profile, "poll_interval=6").is_err());
    }
}
