use crate::{
    capabilities::{
        Capability, KeyerCapabilities, RadioCapabilities, ReceiverCapabilities, ReceiverKind,
        ReceiverRfCapabilities, RitXitCapabilities, RitXitOffsetType, StateUpdateCapability,
        TransmitterCapabilities,
    },
    driver::DriverDescriptor,
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

const RW: Capability = Capability::ReadWrite;
const WO: Capability = Capability::WriteOnly;
const EMULATED: Capability = Capability::Emulated;
const UNSUPPORTED: Capability = Capability::Unsupported;

const SMARTSDR_RX_RF: ReceiverRfCapabilities =
    ReceiverRfCapabilities::new(UNSUPPORTED, UNSUPPORTED, RW, RW, RW);
const SMARTSDR_RX: ReceiverCapabilities = ReceiverCapabilities::new(RW, RW, RW, RW, SMARTSDR_RX_RF);
const SMARTSDR_TX: TransmitterCapabilities =
    TransmitterCapabilities::new(RW, RW, RW, RW, UNSUPPORTED);
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
        description: "FlexRadio SmartSDR TCP slice control (fixed slice 0).",
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
}
