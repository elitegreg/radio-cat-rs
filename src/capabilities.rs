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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiverKind {
    SingleVfo,
    DualVfo,
    DualRx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadioCapabilities {
    pub receiver_kind: ReceiverKind,
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
            receiver_kind: ReceiverKind::DualVfo,
            main_rx: ReceiverCapabilities::all(),
            sub_rx: Some(ReceiverCapabilities::all()),
            tx: Some(TransmitterCapabilities::all()),
            rit_xit: RitXitCapabilities::all(),
            keyer: Some(KeyerCapabilities::all()),
            state_updates: StateUpdateCapability::Native,
        }
    }

    pub const fn new(
        receiver_kind: ReceiverKind,
        main_rx: ReceiverCapabilities,
        sub_rx: Option<ReceiverCapabilities>,
        tx: Option<TransmitterCapabilities>,
        rit_xit: RitXitCapabilities,
        keyer: Option<KeyerCapabilities>,
        state_updates: StateUpdateCapability,
    ) -> Self {
        Self {
            receiver_kind,
            main_rx,
            sub_rx,
            tx,
            rit_xit,
            keyer,
            state_updates,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub const fn new(
        frequency: Capability,
        mode: Capability,
        filter_bandwidth: Capability,
        filter_shift: Capability,
        rf: ReceiverRfCapabilities,
    ) -> Self {
        Self {
            frequency,
            mode,
            filter_bandwidth,
            filter_shift,
            rf,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub const fn new(
        preamp: Capability,
        attenuator: Capability,
        noise_blanker: Capability,
        noise_reduction: Capability,
        auto_notch: Capability,
    ) -> Self {
        Self {
            preamp,
            attenuator,
            noise_blanker,
            noise_reduction,
            auto_notch,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub const fn new(
        frequency: Capability,
        mode: Capability,
        power: Capability,
        ptt: Capability,
        split: Capability,
    ) -> Self {
        Self {
            frequency,
            mode,
            power,
            ptt,
            split,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RitXitCapabilities {
    pub main_rit_enabled: Capability,
    pub sub_rit_enabled: Capability,
    pub xit_enabled: Capability,
    pub offset: Capability,
    pub sub_offset: Capability,
}

impl RitXitCapabilities {
    pub const fn all() -> Self {
        Self {
            main_rit_enabled: Capability::ReadWrite,
            sub_rit_enabled: Capability::ReadWrite,
            xit_enabled: Capability::ReadWrite,
            offset: Capability::ReadWrite,
            sub_offset: Capability::ReadWrite,
        }
    }

    pub const fn new(
        main_rit_enabled: Capability,
        sub_rit_enabled: Capability,
        xit_enabled: Capability,
        offset: Capability,
        sub_offset: Capability,
    ) -> Self {
        Self {
            main_rit_enabled,
            sub_rit_enabled,
            xit_enabled,
            offset,
            sub_offset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub const fn new(
        speed_wpm: Capability,
        sending: Capability,
        send_cw: Capability,
        stop_cw: Capability,
    ) -> Self {
        Self {
            speed_wpm,
            sending,
            send_cw,
            stop_cw,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateUpdateCapability {
    Native,
    Polling,
    Hybrid,
}
