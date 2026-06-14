#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Unsupported,
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl Capability {
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    pub const fn can_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn can_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioCapabilities {
    pub main_rx: ReceiverCapabilities,
    pub sub_rx: Option<ReceiverCapabilities>,
    pub tx: Option<TransmitterCapabilities>,
    pub rit_xit: RitXitCapabilities,
    pub keyer: Option<KeyerCapabilities>,
    pub state_updates: StateUpdateCapability,
}

impl RadioCapabilities {
    pub const fn dummy_all() -> Self {
        Self {
            main_rx: ReceiverCapabilities::all(),
            sub_rx: Some(ReceiverCapabilities::all()),
            tx: Some(TransmitterCapabilities::all()),
            rit_xit: RitXitCapabilities::all(),
            keyer: Some(KeyerCapabilities::all()),
            state_updates: StateUpdateCapability::Native,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverCapabilities {
    pub frequency: Capability,
    pub mode: Capability,
    pub filter_bandwidth: Capability,
    pub filter_shift: Capability,
    pub rf: ReceiverRfCapabilities,
}

impl ReceiverCapabilities {
    pub const fn all() -> Self {
        Self {
            frequency: Capability::ReadWrite,
            mode: Capability::ReadWrite,
            filter_bandwidth: Capability::ReadWrite,
            filter_shift: Capability::ReadWrite,
            rf: ReceiverRfCapabilities::all(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverRfCapabilities {
    pub preamp: Capability,
    pub attenuator: Capability,
    pub noise_blanker: Capability,
    pub noise_reduction: Capability,
    pub auto_notch: Capability,
}

impl ReceiverRfCapabilities {
    pub const fn all() -> Self {
        Self {
            preamp: Capability::ReadWrite,
            attenuator: Capability::ReadWrite,
            noise_blanker: Capability::ReadWrite,
            noise_reduction: Capability::ReadWrite,
            auto_notch: Capability::ReadWrite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransmitterCapabilities {
    pub frequency: Capability,
    pub mode: Capability,
    pub power: Capability,
    pub ptt: Capability,
    pub split: Capability,
}

impl TransmitterCapabilities {
    pub const fn all() -> Self {
        Self {
            frequency: Capability::ReadWrite,
            mode: Capability::ReadWrite,
            power: Capability::ReadWrite,
            ptt: Capability::ReadWrite,
            split: Capability::ReadWrite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RitXitCapabilities {
    pub rit_enabled: Capability,
    pub xit_enabled: Capability,
    pub offset: Capability,
}

impl RitXitCapabilities {
    pub const fn all() -> Self {
        Self {
            rit_enabled: Capability::ReadWrite,
            xit_enabled: Capability::ReadWrite,
            offset: Capability::ReadWrite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyerCapabilities {
    pub speed_wpm: Capability,
    pub sending: Capability,
    pub send_cw: Capability,
    pub stop_cw: Capability,
}

impl KeyerCapabilities {
    pub const fn all() -> Self {
        Self {
            speed_wpm: Capability::ReadWrite,
            sending: Capability::ReadWrite,
            send_cw: Capability::WriteOnly,
            stop_cw: Capability::WriteOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateUpdateCapability {
    Native,
    Polling,
    Hybrid,
}
