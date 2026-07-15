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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartSdrProfile {
    pub descriptor: DriverDescriptor,
    pub slice: u8,
    pub capabilities: RadioCapabilities,
}

impl SmartSdrProfile {
    pub const fn id(self) -> &'static str {
        self.descriptor.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartSdrOptions {
    pub slice: u8,
}

impl SmartSdrOptions {
    pub fn defaults(profile: &SmartSdrProfile) -> Self {
        Self {
            slice: profile.slice,
        }
    }

    pub fn parse(profile: &SmartSdrProfile, options: &str) -> Result<Self> {
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
                "slice" | "slice_index" => {
                    parsed.slice =
                        value
                            .parse::<u8>()
                            .map_err(|error| RadioError::InvalidValue {
                                field: "slice",
                                message: error.to_string(),
                            })?;
                }
                _ => {
                    return Err(RadioError::InvalidValue {
                        field: "options",
                        message: format!("unknown SmartSDR option {key:?}"),
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

const SMARTSDR_RX_RF: ReceiverRfCapabilities =
    ReceiverRfCapabilities::new(UNSUPPORTED, UNSUPPORTED, RW, RW, RW);
const SMARTSDR_RX: ReceiverCapabilities = ReceiverCapabilities::new(RW, RW, RW, RW, SMARTSDR_RX_RF);
const SMARTSDR_POWER: &[PowerRange] = &[PowerRange::fixed(
    Power::from_microwatts(0),
    Power::from_microwatts(100_000_000),
    Power::from_microwatts(1_000_000),
)];
const SMARTSDR_TX: TransmitterCapabilities = TransmitterCapabilities::new(
    RW,
    RW,
    PowerCapability::new(RW, SMARTSDR_POWER),
    RW,
    UNSUPPORTED,
);
const SMARTSDR_RIT_XIT: RitXitCapabilities = RitXitCapabilities::new(
    RW,
    UNSUPPORTED,
    RW,
    RW,
    UNSUPPORTED,
    RitXitOffsetType::Independent,
);
const SMARTSDR_KEYER: KeyerCapabilities = KeyerCapabilities::new(RW, EMULATED, WO, WO);

pub const FLEXRADIO_SMARTSDR: SmartSdrProfile = SmartSdrProfile {
    descriptor: DriverDescriptor {
        id: "flexradio-smartsdr",
        display_name: "FlexRadio SmartSDR",
        description:
            "FlexRadio SmartSDR TCP slice control (default slice 0; configurable via options).",
        transport_requirement: TransportRequirement::Tcp,
    },
    slice: 0,
    capabilities: RadioCapabilities::new(
        ReceiverKind::SingleVfo,
        SMARTSDR_RX,
        None,
        Some(SMARTSDR_TX),
        SMARTSDR_RIT_XIT,
        Some(SMARTSDR_KEYER),
        StateUpdateCapability::Native,
    ),
};

pub const SUPPORTED_PROFILES: &[SmartSdrProfile] = &[FLEXRADIO_SMARTSDR];

pub fn profile_by_id(id: &str) -> Option<&'static SmartSdrProfile> {
    SUPPORTED_PROFILES.iter().find(|profile| {
        profile.id().eq_ignore_ascii_case(id) || id.eq_ignore_ascii_case("flexradio-smarthdr")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smartsdr_keyer_supports_speed_and_emulated_sending() {
        let profile = profile_by_id("flexradio-smartsdr").unwrap();
        let keyer = profile.capabilities.keyer.unwrap();

        assert_eq!(keyer.speed_wpm, Capability::ReadWrite);
        assert_eq!(keyer.sending, Capability::Emulated);
        assert_eq!(keyer.send_cw, Capability::WriteOnly);
        assert_eq!(keyer.stop_cw, Capability::WriteOnly);
    }

    #[test]
    fn smartsdr_options_default_to_profile_slice() {
        let profile = profile_by_id("flexradio-smartsdr").unwrap();
        let options = SmartSdrOptions::parse(profile, "").unwrap();

        assert_eq!(options.slice, 0);
    }

    #[test]
    fn smartsdr_options_parse_slice_override() {
        let profile = profile_by_id("flexradio-smartsdr").unwrap();
        let options = SmartSdrOptions::parse(profile, "slice=2").unwrap();

        assert_eq!(options.slice, 2);
    }
}
