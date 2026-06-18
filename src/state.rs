use crate::{error::RangeError, Frequency, Mode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioState {
    pub connection: ConnectionState,
    pub main_rx: ReceiverState,
    pub sub_rx: Option<ReceiverState>,
    pub tx: Option<TransmitterState>,
    pub rit_xit: RitXitState,
    pub keyer: Option<KeyerState>,
}

impl Default for RadioState {
    fn default() -> Self {
        Self {
            connection: ConnectionState::Disconnected,
            main_rx: ReceiverState::default(),
            sub_rx: None,
            tx: None,
            rit_xit: RitXitState::default(),
            keyer: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Identifying,
    Ready,
    Error { message: String },
    Reconnecting,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverState {
    pub frequency: Option<Frequency>,
    pub mode: Option<Mode>,
    pub filter: ReceiverFilterState,
    pub rf: ReceiverRfState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverFilterState {
    pub bandwidth_hz: Option<u16>,
    pub shift_hz: Option<i16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverRfState {
    pub preamp: Option<LeveledSetting>,
    pub attenuator: Option<LeveledSetting>,
    pub noise_blanker: Option<LeveledSetting>,
    pub noise_reduction: Option<LeveledSetting>,
    pub auto_notch: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransmitterState {
    pub frequency: Option<Frequency>,
    pub mode: Option<Mode>,
    pub power: Option<Power>,
    pub transmitting: Option<bool>,
    pub split: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerUnit {
    Watts,
    Milliwatts,
    Microwatts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Power {
    value: u16,
    unit: PowerUnit,
}

impl Power {
    pub const fn new(value: u16, unit: PowerUnit) -> Self {
        Self { value, unit }
    }

    pub const fn from_watts(value: u16) -> Self {
        Self::new(value, PowerUnit::Watts)
    }

    pub const fn from_milliwatts(value: u16) -> Self {
        Self::new(value, PowerUnit::Milliwatts)
    }

    pub const fn from_microwatts(value: u16) -> Self {
        Self::new(value, PowerUnit::Microwatts)
    }

    pub const fn value(self) -> u16 {
        self.value
    }

    pub const fn unit(self) -> PowerUnit {
        self.unit
    }

    pub const fn as_microwatts(self) -> u64 {
        match self.unit {
            PowerUnit::Watts => self.value as u64 * 1_000_000,
            PowerUnit::Milliwatts => self.value as u64 * 1_000,
            PowerUnit::Microwatts => self.value as u64,
        }
    }

    pub const fn as_milliwatts(self) -> u64 {
        self.as_microwatts() / 1_000
    }

    pub const fn as_watts(self) -> f64 {
        self.as_microwatts() as f64 / 1_000_000.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RitXitState {
    pub main_rit_enabled: Option<bool>,
    pub sub_rit_enabled: Option<bool>,
    pub xit_enabled: Option<bool>,
    pub offset_hz: Option<RitXitOffsetHz>,
    pub xit_offset_hz: Option<RitXitOffsetHz>,
    pub sub_offset_hz: Option<RitXitOffsetHz>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RitXitOffsetHz(i16);

impl RitXitOffsetHz {
    pub const MIN: i16 = -9_999;
    pub const MAX: i16 = 9_999;

    pub fn new(value: i16) -> Result<Self, RangeError> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(RangeError {
                value,
                min: Self::MIN,
                max: Self::MAX,
            })
        }
    }

    pub const fn as_hz(self) -> i16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyerState {
    pub speed_wpm: Option<u8>,
    pub sending: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct LeveledSetting {
    pub enabled: Option<bool>,
    pub level: Option<u8>,
}

impl LeveledSetting {
    pub const fn new(enabled: Option<bool>, level: Option<u8>) -> Self {
        Self { enabled, level }
    }

    pub const fn enabled(level: u8) -> Self {
        Self {
            enabled: Some(true),
            level: Some(level),
        }
    }

    pub const fn disabled() -> Self {
        Self {
            enabled: Some(false),
            level: Some(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Power, PowerUnit, RitXitOffsetHz};

    #[test]
    fn rit_xit_offset_accepts_normalized_range() {
        assert_eq!(RitXitOffsetHz::new(-9_999).unwrap().as_hz(), -9_999);
        assert_eq!(RitXitOffsetHz::new(0).unwrap().as_hz(), 0);
        assert_eq!(RitXitOffsetHz::new(9_999).unwrap().as_hz(), 9_999);
    }

    #[test]
    fn rit_xit_offset_rejects_out_of_range_values() {
        assert!(RitXitOffsetHz::new(-10_000).is_err());
        assert!(RitXitOffsetHz::new(10_000).is_err());
    }

    #[test]
    fn power_converts_between_supported_units() {
        let watts = Power::from_watts(5);
        assert_eq!(watts.value(), 5);
        assert_eq!(watts.unit(), PowerUnit::Watts);
        assert_eq!(watts.as_milliwatts(), 5_000);
        assert_eq!(watts.as_microwatts(), 5_000_000);

        let milliwatts = Power::from_milliwatts(250);
        assert_eq!(milliwatts.as_microwatts(), 250_000);

        let microwatts = Power::from_microwatts(125);
        assert_eq!(microwatts.as_microwatts(), 125);
        assert!((microwatts.as_watts() - 0.000_125).abs() < f64::EPSILON);
    }
}
