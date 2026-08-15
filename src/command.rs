use crate::{Frequency, LeveledSetting, Mode, Power, RitXitOffsetHz};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiverPath {
    Main,
    Sub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioCommand {
    SetReceiverFrequency {
        receiver: ReceiverPath,
        frequency: Frequency,
    },
    SetReceiverMode {
        receiver: ReceiverPath,
        mode: Mode,
    },
    SetReceiverFilterBandwidth {
        receiver: ReceiverPath,
        bandwidth_hz: u16,
    },
    SetReceiverFilterShift {
        receiver: ReceiverPath,
        shift_hz: i16,
    },
    SetReceiverPreamp {
        receiver: ReceiverPath,
        setting: LeveledSetting,
    },
    SetReceiverAttenuator {
        receiver: ReceiverPath,
        setting: LeveledSetting,
    },
    SetReceiverNoiseBlanker {
        receiver: ReceiverPath,
        setting: LeveledSetting,
    },
    SetReceiverNoiseReduction {
        receiver: ReceiverPath,
        setting: LeveledSetting,
    },
    SetReceiverAutoNotch {
        receiver: ReceiverPath,
        enabled: bool,
    },

    SetTxFrequency(Frequency),
    SetTxMode(Mode),
    SetTxPower(Power),
    SetPtt(bool),
    SetDataPtt(bool),
    SetSplit(bool),

    SetRitEnabled {
        receiver: ReceiverPath,
        enabled: bool,
    },
    SetXitEnabled {
        receiver: ReceiverPath,
        enabled: bool,
    },
    SetRitOffset {
        receiver: ReceiverPath,
        offset: RitXitOffsetHz,
    },
    SetXitOffset {
        receiver: ReceiverPath,
        offset: RitXitOffsetHz,
    },
    SetRitXitOffset {
        receiver: ReceiverPath,
        offset: RitXitOffsetHz,
    },

    SetKeyerSpeed(u8),
    SendCw(String),
    StopCw,
    SendData(String),
    StopData,

    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    Completed,
    TxPower { accepted: Power },
}
