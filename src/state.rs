use std::fmt;

use crate::{error::RangeError, Frequency, Mode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioState {
    pub(crate) connection: ConnectionState,
    pub(crate) main_rx: ReceiverState,
    pub(crate) sub_rx: Option<ReceiverState>,
    pub(crate) tx: Option<TransmitterState>,
    pub(crate) rit_xit: RitXitState,
    pub(crate) keyer: Option<KeyerState>,
}

impl RadioState {
    pub fn connection(&self) -> ConnectionState {
        self.connection.clone()
    }
    pub fn main_rx(&self) -> &ReceiverState {
        &self.main_rx
    }
    pub fn sub_rx(&self) -> Option<&ReceiverState> {
        self.sub_rx.as_ref()
    }
    pub fn tx(&self) -> Option<&TransmitterState> {
        self.tx.as_ref()
    }
    pub fn rit_xit(&self) -> &RitXitState {
        &self.rit_xit
    }
    pub fn keyer(&self) -> Option<&KeyerState> {
        self.keyer.as_ref()
    }
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
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverState {
    pub(crate) frequency: Option<Frequency>,
    pub(crate) mode: Option<Mode>,
    pub(crate) filter: ReceiverFilterState,
    pub(crate) rf: ReceiverRfState,
}

impl ReceiverState {
    pub fn frequency(&self) -> Option<Frequency> {
        self.frequency
    }
    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }
    pub fn filter(&self) -> &ReceiverFilterState {
        &self.filter
    }
    pub fn rf(&self) -> &ReceiverRfState {
        &self.rf
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverFilterState {
    pub(crate) bandwidth_hz: Option<u16>,
    pub(crate) shift_hz: Option<i16>,
}

impl ReceiverFilterState {
    pub fn bandwidth_hz(&self) -> Option<u16> {
        self.bandwidth_hz
    }
    pub fn shift_hz(&self) -> Option<i16> {
        self.shift_hz
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverRfState {
    pub(crate) preamp: Option<LeveledSetting>,
    pub(crate) attenuator: Option<LeveledSetting>,
    pub(crate) noise_blanker: Option<LeveledSetting>,
    pub(crate) noise_reduction: Option<LeveledSetting>,
    pub(crate) auto_notch: Option<bool>,
}

impl ReceiverRfState {
    pub fn preamp(&self) -> Option<LeveledSetting> {
        self.preamp
    }
    pub fn attenuator(&self) -> Option<LeveledSetting> {
        self.attenuator
    }
    pub fn noise_blanker(&self) -> Option<LeveledSetting> {
        self.noise_blanker
    }
    pub fn noise_reduction(&self) -> Option<LeveledSetting> {
        self.noise_reduction
    }
    pub fn auto_notch(&self) -> Option<bool> {
        self.auto_notch
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransmitterState {
    pub(crate) frequency: Option<Frequency>,
    pub(crate) mode: Option<Mode>,
    pub(crate) power: Option<Power>,
    pub(crate) transmitting: Option<bool>,
    pub(crate) split: Option<bool>,
}

impl TransmitterState {
    pub fn frequency(&self) -> Option<Frequency> {
        self.frequency
    }
    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }
    pub fn power(&self) -> Option<Power> {
        self.power
    }
    pub fn transmitting(&self) -> Option<bool> {
        self.transmitting
    }
    pub fn split(&self) -> Option<bool> {
        self.split
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Power(u64);

impl Power {
    pub const MICROWATTS_PER_MILLIWATT: u64 = 1_000;
    pub const MICROWATTS_PER_WATT: u64 = 1_000_000;

    pub const fn from_microwatts(value: u64) -> Self {
        Self(value)
    }

    pub const fn from_milliwatts(value: u32) -> Self {
        Self(value as u64 * Self::MICROWATTS_PER_MILLIWATT)
    }

    pub const fn from_watts(value: u32) -> Self {
        Self(value as u64 * Self::MICROWATTS_PER_WATT)
    }

    pub const fn checked_from_milliwatts(value: u64) -> Option<Self> {
        match value.checked_mul(Self::MICROWATTS_PER_MILLIWATT) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn checked_from_watts(value: u64) -> Option<Self> {
        match value.checked_mul(Self::MICROWATTS_PER_WATT) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn as_microwatts(self) -> u64 {
        self.0
    }

    pub const fn as_milliwatts(self) -> u64 {
        self.0 / Self::MICROWATTS_PER_MILLIWATT
    }

    pub const fn as_watts(self) -> f64 {
        self.0 as f64 / Self::MICROWATTS_PER_WATT as f64
    }
}

impl fmt::Display for Power {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, remainder, width, suffix) = if self.0 >= Self::MICROWATTS_PER_WATT {
            (
                self.0 / Self::MICROWATTS_PER_WATT,
                self.0 % Self::MICROWATTS_PER_WATT,
                6,
                "W",
            )
        } else if self.0 >= Self::MICROWATTS_PER_MILLIWATT {
            (
                self.0 / Self::MICROWATTS_PER_MILLIWATT,
                self.0 % Self::MICROWATTS_PER_MILLIWATT,
                3,
                "mW",
            )
        } else {
            return write!(formatter, "{}µW", self.0);
        };

        if remainder == 0 {
            return write!(formatter, "{whole}{suffix}");
        }

        let fraction = format!("{remainder:0width$}");
        write!(
            formatter,
            "{whole}.{}{suffix}",
            fraction.trim_end_matches('0')
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RitXitState {
    pub(crate) main_rit_enabled: Option<bool>,
    pub(crate) sub_rit_enabled: Option<bool>,
    pub(crate) xit_enabled: Option<bool>,
    pub(crate) sub_xit_enabled: Option<bool>,
    pub(crate) offset_hz: Option<RitXitOffsetHz>,
    pub(crate) xit_offset_hz: Option<RitXitOffsetHz>,
    pub(crate) sub_offset_hz: Option<RitXitOffsetHz>,
    pub(crate) sub_xit_offset_hz: Option<RitXitOffsetHz>,
}

impl RitXitState {
    pub fn rit_enabled(&self, receiver: crate::ReceiverPath) -> Option<bool> {
        match receiver {
            crate::ReceiverPath::Main => self.main_rit_enabled,
            crate::ReceiverPath::Sub => self.sub_rit_enabled,
        }
    }
    pub fn xit_enabled(&self, receiver: crate::ReceiverPath) -> Option<bool> {
        match receiver {
            crate::ReceiverPath::Main => self.xit_enabled,
            crate::ReceiverPath::Sub => self.sub_xit_enabled,
        }
    }
    pub fn rit_offset(&self, receiver: crate::ReceiverPath) -> Option<RitXitOffsetHz> {
        match receiver {
            crate::ReceiverPath::Main => self.offset_hz,
            crate::ReceiverPath::Sub => self.sub_offset_hz,
        }
    }
    pub fn xit_offset(&self, receiver: crate::ReceiverPath) -> Option<RitXitOffsetHz> {
        match receiver {
            crate::ReceiverPath::Main => self.xit_offset_hz,
            crate::ReceiverPath::Sub => self.sub_xit_offset_hz,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RitXitOffsetHz(i16);

impl RitXitOffsetHz {
    pub const MIN: i16 = -9_999;
    pub const MAX: i16 = 9_999;
    pub const MIN_VALUE: Self = Self(Self::MIN);
    pub const MAX_VALUE: Self = Self(Self::MAX);
    pub const ONE_HZ: Self = Self(1);

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
    pub(crate) speed_wpm: Option<u8>,
    pub(crate) sending: Option<bool>,
}

impl KeyerState {
    pub fn speed_wpm(&self) -> Option<u8> {
        self.speed_wpm
    }
    pub fn sending(&self) -> Option<bool> {
        self.sending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeveledSetting(LeveledSettingValue);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LeveledSettingValue {
    Disabled,
    Enabled(u8),
}

impl Default for LeveledSetting {
    fn default() -> Self {
        Self::disabled()
    }
}

impl LeveledSetting {
    pub const fn new(enabled: Option<bool>, level: Option<u8>) -> Self {
        match (enabled, level) {
            (Some(false), _) | (_, Some(0)) | (None, None) => Self::disabled(),
            (Some(true), Some(level)) | (None, Some(level)) => Self::enabled(level),
            (Some(true), None) => Self::enabled(1),
        }
    }

    pub const fn enabled(level: u8) -> Self {
        if level == 0 {
            Self::disabled()
        } else {
            Self(LeveledSettingValue::Enabled(level))
        }
    }

    pub const fn disabled() -> Self {
        Self(LeveledSettingValue::Disabled)
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self.0, LeveledSettingValue::Enabled(_))
    }

    pub const fn level(self) -> Option<u8> {
        match self.0 {
            LeveledSettingValue::Disabled => None,
            LeveledSettingValue::Enabled(level) => Some(level),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LeveledSetting, Power, RitXitOffsetHz};

    #[test]
    fn leveled_settings_normalize_contradictory_inputs() {
        assert_eq!(
            LeveledSetting::new(Some(false), Some(100)),
            LeveledSetting::disabled()
        );
        assert_eq!(
            LeveledSetting::new(Some(true), Some(37)),
            LeveledSetting::enabled(37)
        );
    }

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
        let watts = Power::checked_from_watts(5).unwrap();
        assert_eq!(watts.as_milliwatts(), 5_000);
        assert_eq!(watts.as_microwatts(), 5_000_000);

        let milliwatts = Power::checked_from_milliwatts(250).unwrap();
        assert_eq!(milliwatts.as_microwatts(), 250_000);

        let microwatts = Power::from_microwatts(125);
        assert_eq!(microwatts.as_microwatts(), 125);
        assert!((microwatts.as_watts() - 0.000_125).abs() < f64::EPSILON);
    }

    #[test]
    fn power_is_canonical_and_checked() {
        assert_eq!(
            Power::checked_from_watts(1),
            Power::checked_from_milliwatts(1_000)
        );
        assert_eq!(
            Power::checked_from_watts(1),
            Some(Power::from_microwatts(1_000_000))
        );
        assert!(Power::checked_from_watts(u64::MAX).is_none());
        assert!(Power::checked_from_milliwatts(u64::MAX).is_none());
    }

    #[test]
    fn power_display_uses_exact_engineering_units() {
        assert_eq!(Power::from_microwatts(7_500_000).to_string(), "7.5W");
        assert_eq!(Power::from_microwatts(7_000_001).to_string(), "7.000001W");
        assert_eq!(Power::from_microwatts(750_000).to_string(), "750mW");
        assert_eq!(Power::from_microwatts(1_250).to_string(), "1.25mW");
        assert_eq!(Power::from_microwatts(500).to_string(), "500µW");
    }
}
