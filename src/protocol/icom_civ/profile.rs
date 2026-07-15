use std::time::Duration;

use crate::{
    capabilities::{
        Capability, KeyerCapabilities, PowerCapability, PowerRange, RadioCapabilities,
        ReceiverCapabilities, ReceiverKind, ReceiverRfCapabilities, RitXitCapabilities,
        RitXitOffsetType, StateUpdateCapability, TransmitterCapabilities,
    },
    driver::{DriverDescriptor, TransportRequirement},
    error::{RadioError, Result},
    Power,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IcomArchitecture {
    DualVfo,
    DualRx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcomCivProfile {
    pub descriptor: DriverDescriptor,
    pub architecture: IcomArchitecture,
    pub default_radio_address: u8,
    pub default_controller_address: u8,
    pub max_tx_power_watts: u16,
    pub mode_map: &'static [(u8, crate::Mode)],
    pub attenuator_values_db: &'static [u8],
    pub supports_command_29: bool,
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
const EMULATED: Capability = Capability::Emulated;
const UNSUPPORTED: Capability = Capability::Unsupported;

const MAIN_RF: ReceiverRfCapabilities = ReceiverRfCapabilities::new(RW, RW, RW, RW, RW);
const RX_WITH_RF_NO_FILTER_SHIFT: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, RW, UNSUPPORTED, MAIN_RF);
const RX_WITH_RF_NO_BANDWIDTH: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, UNSUPPORTED, UNSUPPORTED, MAIN_RF);
const RX_FREQ_MODE_ONLY: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, UNSUPPORTED, UNSUPPORTED, NO_RF);
const NO_RF: ReceiverRfCapabilities = ReceiverRfCapabilities::new(
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
);

const IC705_MAIN_RX: ReceiverCapabilities = RX_WITH_RF_NO_FILTER_SHIFT;
const IC705_SUB_RX: ReceiverCapabilities = RX_FREQ_MODE_ONLY;
const ICOM_POWER_10W: &[PowerRange] = &[PowerRange::linear(
    Power::from_microwatts(0),
    Power::from_microwatts(10_000_000),
    255,
)];
const ICOM_POWER_100W: &[PowerRange] = &[PowerRange::linear(
    Power::from_microwatts(0),
    Power::from_microwatts(100_000_000),
    255,
)];
const ICOM_POWER_200W: &[PowerRange] = &[PowerRange::linear(
    Power::from_microwatts(0),
    Power::from_microwatts(200_000_000),
    255,
)];

const fn icom_tx(ranges: &'static [PowerRange]) -> TransmitterCapabilities {
    TransmitterCapabilities::new(RW, RW, PowerCapability::new(RW, ranges), RW, RW)
}
const ICOM_SHARED_RIT_XIT: RitXitCapabilities = RitXitCapabilities::new(
    RW,
    UNSUPPORTED,
    RW,
    RW,
    UNSUPPORTED,
    RitXitOffsetType::Shared,
);
const ICOM_SHARED_RIT_ONLY: RitXitCapabilities = RitXitCapabilities::new(
    RW,
    UNSUPPORTED,
    UNSUPPORTED,
    RW,
    UNSUPPORTED,
    RitXitOffsetType::Shared,
);
const IC705_KEYER: KeyerCapabilities = KeyerCapabilities::new(RW, EMULATED, WO, WO);

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

const IC7100_STARTUP: &[StartupStep] = &[
    StartupStep::Query("freq-main"),
    StartupStep::Query("freq-sub"),
    StartupStep::Query("mode-main"),
    StartupStep::Query("mode-sub"),
    StartupStep::Query("tx-frequency"),
    StartupStep::Query("ptt"),
    StartupStep::Query("split"),
    StartupStep::Query("rit-offset"),
    StartupStep::Query("rit"),
    StartupStep::Query("filter-bandwidth"),
    StartupStep::Query("preamp-main"),
    StartupStep::Query("attenuator-main"),
    StartupStep::Query("noise-blanker-main"),
    StartupStep::Query("noise-reduction-main"),
    StartupStep::Query("auto-notch-main"),
    StartupStep::Query("tx-power"),
    StartupStep::Query("keyer-speed"),
];

const IC7100_POLL_QUERIES: &[&str] = &[
    "freq-main",
    "freq-sub",
    "mode-main",
    "mode-sub",
    "tx-frequency",
    "ptt",
    "split",
    "rit-offset",
    "rit",
    "filter-bandwidth",
    "preamp-main",
    "attenuator-main",
    "noise-blanker-main",
    "noise-reduction-main",
    "auto-notch-main",
    "tx-power",
    "keyer-speed",
];

const IC7610_STARTUP: &[StartupStep] = &[
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
    StartupStep::Query("preamp-main"),
    StartupStep::Query("attenuator-main"),
    StartupStep::Query("noise-blanker-main"),
    StartupStep::Query("noise-reduction-main"),
    StartupStep::Query("auto-notch-main"),
    StartupStep::Query("tx-power"),
    StartupStep::Query("keyer-speed"),
];

const IC7610_POLL_QUERIES: &[&str] = &[
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
    "preamp-main",
    "attenuator-main",
    "noise-blanker-main",
    "noise-reduction-main",
    "auto-notch-main",
    "tx-power",
    "keyer-speed",
];

const IC7760_STARTUP: &[StartupStep] = &[
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
    StartupStep::Query("preamp-main"),
    StartupStep::Query("preamp-sub"),
    StartupStep::Query("attenuator-main"),
    StartupStep::Query("attenuator-sub"),
    StartupStep::Query("noise-blanker-main"),
    StartupStep::Query("noise-blanker-sub"),
    StartupStep::Query("noise-reduction-main"),
    StartupStep::Query("noise-reduction-sub"),
    StartupStep::Query("auto-notch-main"),
    StartupStep::Query("auto-notch-sub"),
    StartupStep::Query("tx-power"),
    StartupStep::Query("keyer-speed"),
];

const IC7760_POLL_QUERIES: &[&str] = &[
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
    "preamp-main",
    "preamp-sub",
    "attenuator-main",
    "attenuator-sub",
    "noise-blanker-main",
    "noise-blanker-sub",
    "noise-reduction-main",
    "noise-reduction-sub",
    "auto-notch-main",
    "auto-notch-sub",
    "tx-power",
    "keyer-speed",
];

const IC705_MODES: &[(u8, crate::Mode)] = &[
    (0x00, crate::Mode::Lsb),
    (0x01, crate::Mode::Usb),
    (0x02, crate::Mode::Am),
    (0x03, crate::Mode::Cw),
    (0x04, crate::Mode::Rtty),
    (0x05, crate::Mode::Fm),
    (0x06, crate::Mode::Wfm),
    (0x07, crate::Mode::CwReverse),
    (0x08, crate::Mode::RttyReverse),
    (0x17, crate::Mode::DigitalVoice),
];

const IC7300_MODES: &[(u8, crate::Mode)] = &[
    (0x00, crate::Mode::Lsb),
    (0x01, crate::Mode::Usb),
    (0x02, crate::Mode::Am),
    (0x03, crate::Mode::Cw),
    (0x04, crate::Mode::Rtty),
    (0x05, crate::Mode::Fm),
    (0x07, crate::Mode::CwReverse),
    (0x08, crate::Mode::RttyReverse),
];

const IC7100_MODES: &[(u8, crate::Mode)] = &[
    (0x00, crate::Mode::Lsb),
    (0x01, crate::Mode::Usb),
    (0x02, crate::Mode::Am),
    (0x03, crate::Mode::Cw),
    (0x04, crate::Mode::Rtty),
    (0x05, crate::Mode::Fm),
    (0x06, crate::Mode::Wfm),
    (0x07, crate::Mode::CwReverse),
    (0x08, crate::Mode::RttyReverse),
    (0x17, crate::Mode::DigitalVoice),
];

const IC7610_MODES: &[(u8, crate::Mode)] = &[
    (0x00, crate::Mode::Lsb),
    (0x01, crate::Mode::Usb),
    (0x02, crate::Mode::Am),
    (0x03, crate::Mode::Cw),
    (0x04, crate::Mode::Rtty),
    (0x05, crate::Mode::Fm),
    (0x07, crate::Mode::CwReverse),
    (0x08, crate::Mode::RttyReverse),
    (0x12, crate::Mode::Psk),
    (0x17, crate::Mode::PskReverse),
];

const IC7760_MODES: &[(u8, crate::Mode)] = &[
    (0x00, crate::Mode::Lsb),
    (0x01, crate::Mode::Usb),
    (0x02, crate::Mode::Am),
    (0x03, crate::Mode::Cw),
    (0x04, crate::Mode::Rtty),
    (0x05, crate::Mode::Fm),
    (0x07, crate::Mode::CwReverse),
    (0x08, crate::Mode::RttyReverse),
    (0x12, crate::Mode::Psk),
    (0x13, crate::Mode::PskReverse),
];

const ATTENUATOR_20_DB: &[u8] = &[0, 20];
const ATTENUATOR_12_DB: &[u8] = &[0, 12];
const ATTENUATOR_3_TO_24_DB: &[u8] = &[0, 3, 6, 9, 12, 15, 18, 21, 24];
const ATTENUATOR_3_TO_45_DB: &[u8] = &[0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30, 33, 36, 39, 42, 45];

const fn descriptor(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> DriverDescriptor {
    DriverDescriptor {
        id,
        display_name,
        description,
        transport_requirement: TransportRequirement::SerialOrTcp,
    }
}

pub const SUPPORTED_PROFILES: &[IcomCivProfile] = &[
    IcomCivProfile {
        descriptor: descriptor(
            "icom-ic705",
            "Icom IC-705",
            "Icom CI-V profile for IC-705 radios.",
        ),
        architecture: IcomArchitecture::DualVfo,
        default_radio_address: 0xa4,
        default_controller_address: 0xe0,
        max_tx_power_watts: 10,
        mode_map: IC705_MODES,
        attenuator_values_db: ATTENUATOR_20_DB,
        supports_command_29: false,
        capabilities: RadioCapabilities::new(
            ReceiverKind::DualVfo,
            IC705_MAIN_RX,
            Some(IC705_SUB_RX),
            Some(icom_tx(ICOM_POWER_10W)),
            ICOM_SHARED_RIT_XIT,
            Some(IC705_KEYER),
            StateUpdateCapability::Polling,
        ),
        startup: IC705_STARTUP,
        poll: Some(PollPlan {
            queries: IC705_POLL_QUERIES,
        }),
    },
    IcomCivProfile {
        descriptor: descriptor(
            "icom-ic7300",
            "Icom IC-7300",
            "Icom CI-V profile for IC-7300 radios.",
        ),
        architecture: IcomArchitecture::DualVfo,
        default_radio_address: 0x94,
        default_controller_address: 0xe0,
        max_tx_power_watts: 100,
        mode_map: IC7300_MODES,
        attenuator_values_db: ATTENUATOR_20_DB,
        supports_command_29: false,
        capabilities: RadioCapabilities::new(
            ReceiverKind::DualVfo,
            IC705_MAIN_RX,
            Some(IC705_SUB_RX),
            Some(icom_tx(ICOM_POWER_100W)),
            ICOM_SHARED_RIT_XIT,
            Some(IC705_KEYER),
            StateUpdateCapability::Polling,
        ),
        startup: IC705_STARTUP,
        poll: Some(PollPlan {
            queries: IC705_POLL_QUERIES,
        }),
    },
    IcomCivProfile {
        descriptor: descriptor(
            "icom-ic7100",
            "Icom IC-7100",
            "Icom CI-V profile for IC-7100 radios.",
        ),
        architecture: IcomArchitecture::DualVfo,
        default_radio_address: 0x88,
        default_controller_address: 0xe0,
        max_tx_power_watts: 100,
        mode_map: IC7100_MODES,
        attenuator_values_db: ATTENUATOR_12_DB,
        supports_command_29: false,
        capabilities: RadioCapabilities::new(
            ReceiverKind::DualVfo,
            IC705_MAIN_RX,
            Some(IC705_SUB_RX),
            Some(icom_tx(ICOM_POWER_100W)),
            ICOM_SHARED_RIT_ONLY,
            Some(IC705_KEYER),
            StateUpdateCapability::Polling,
        ),
        startup: IC7100_STARTUP,
        poll: Some(PollPlan {
            queries: IC7100_POLL_QUERIES,
        }),
    },
    IcomCivProfile {
        descriptor: descriptor(
            "icom-ic7610",
            "Icom IC-7610",
            "Icom CI-V profile for IC-7610 radios.",
        ),
        architecture: IcomArchitecture::DualRx,
        default_radio_address: 0x98,
        default_controller_address: 0xe0,
        max_tx_power_watts: 100,
        mode_map: IC7610_MODES,
        attenuator_values_db: ATTENUATOR_3_TO_24_DB,
        supports_command_29: false,
        capabilities: RadioCapabilities::new(
            ReceiverKind::DualRx,
            RX_WITH_RF_NO_FILTER_SHIFT,
            Some(RX_FREQ_MODE_ONLY),
            Some(icom_tx(ICOM_POWER_100W)),
            ICOM_SHARED_RIT_XIT,
            Some(IC705_KEYER),
            StateUpdateCapability::Polling,
        ),
        startup: IC7610_STARTUP,
        poll: Some(PollPlan {
            queries: IC7610_POLL_QUERIES,
        }),
    },
    IcomCivProfile {
        descriptor: descriptor(
            "icom-ic7760",
            "Icom IC-7760",
            "Icom CI-V profile for IC-7760 radios.",
        ),
        architecture: IcomArchitecture::DualRx,
        default_radio_address: 0xb2,
        default_controller_address: 0xe0,
        max_tx_power_watts: 200,
        mode_map: IC7760_MODES,
        attenuator_values_db: ATTENUATOR_3_TO_45_DB,
        supports_command_29: true,
        capabilities: RadioCapabilities::new(
            ReceiverKind::DualRx,
            RX_WITH_RF_NO_FILTER_SHIFT,
            Some(RX_WITH_RF_NO_BANDWIDTH),
            Some(icom_tx(ICOM_POWER_200W)),
            ICOM_SHARED_RIT_XIT,
            Some(IC705_KEYER),
            StateUpdateCapability::Polling,
        ),
        startup: IC7760_STARTUP,
        poll: Some(PollPlan {
            queries: IC7760_POLL_QUERIES,
        }),
    },
];

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

    if !(0.05..=5.0).contains(&seconds) {
        return Err(RadioError::InvalidValue {
            field: "poll_interval",
            message: "expected value from 0.05 through 5 seconds".to_string(),
        });
    }

    Ok(Duration::from_secs_f64(seconds))
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

    #[test]
    fn options_reject_invalid_poll_intervals_without_panicking() {
        let profile = profile_by_id("icom-ic705").unwrap();
        for value in ["-1", "1e100", "NaN", "inf"] {
            let result = std::panic::catch_unwind(|| {
                IcomCivOptions::parse(profile, &format!("poll_interval={value}"))
            });
            assert!(result.is_ok());
            assert!(result.unwrap().is_err());
        }
    }

    #[test]
    fn profiles_cover_supported_icom_matrix() {
        assert_eq!(SUPPORTED_PROFILES.len(), 5);
        assert!(profile_by_id("icom-ic705").is_some());
        assert!(profile_by_id("icom-ic7300").is_some());
        assert!(profile_by_id("ICOM-IC7100").is_some());
        assert!(profile_by_id("icom-ic7610").is_some());
        assert!(profile_by_id("icom-ic7760").is_some());
    }

    #[test]
    fn profile_metadata_matches_target_radios() {
        let ic7100 = profile_by_id("icom-ic7100").unwrap();
        assert_eq!(ic7100.default_radio_address, 0x88);
        assert_eq!(
            ic7100.capabilities.rit_xit.xit_enabled,
            Capability::Unsupported
        );

        let ic7610 = profile_by_id("icom-ic7610").unwrap();
        assert_eq!(ic7610.architecture, IcomArchitecture::DualRx);
        assert_eq!(ic7610.capabilities.receiver_kind, ReceiverKind::DualRx);
        assert_eq!(
            ic7610.capabilities.main_rx.filter_bandwidth,
            Capability::ReadWrite
        );
        assert_eq!(ic7610.capabilities.main_rx.rf.preamp, Capability::ReadWrite);
        assert_eq!(
            ic7610.capabilities.keyer.unwrap().sending,
            Capability::Emulated
        );
        assert_eq!(ic7610.attenuator_values_db, ATTENUATOR_3_TO_24_DB);

        let ic7760 = profile_by_id("icom-ic7760").unwrap();
        assert!(ic7760.supports_command_29);
        assert_eq!(ic7760.max_tx_power_watts, 200);
        assert_eq!(
            ic7760.capabilities.main_rx.filter_bandwidth,
            Capability::ReadWrite
        );
        assert_eq!(
            ic7760.capabilities.sub_rx.unwrap().rf.preamp,
            Capability::ReadWrite
        );
    }
}
