use std::{fmt, str::FromStr};

use crate::{Frequency, Mode, Power, RitXitOffsetHz};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Unsupported,
    ReadOnly,
    WriteOnly,
    ReadWrite,
    Emulated,
}

impl Capability {
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    pub const fn can_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite | Self::Emulated)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadioRegion {
    IaruRegion1,
    IaruRegion2,
    IaruRegion3,
}

impl RadioRegion {
    pub const ALL: &'static [Self] = &[Self::IaruRegion1, Self::IaruRegion2, Self::IaruRegion3];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IaruRegion1 => "IARURegion1",
            Self::IaruRegion2 => "IARURegion2",
            Self::IaruRegion3 => "IARURegion3",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::IaruRegion1 => "Europe, Africa, and northern Asia",
            Self::IaruRegion2 => "the Americas",
            Self::IaruRegion3 => "the Far East, Southeast Asia, Japan, Australia, and Oceania",
        }
    }
}

impl fmt::Display for RadioRegion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RadioRegion {
    type Err = crate::RadioError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ' '], "");
        match normalized.as_str() {
            "iaruregion1" | "region1" | "1" => Ok(Self::IaruRegion1),
            "iaruregion2" | "region2" | "2" => Ok(Self::IaruRegion2),
            "iaruregion3" | "region3" | "3" => Ok(Self::IaruRegion3),
            _ => Err(crate::RadioError::InvalidValue {
                field: "region",
                message: format!(
                    "expected IARURegion1, IARURegion2, or IARURegion3, got {value:?}"
                ),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueRange<T> {
    pub min: T,
    pub max: T,
    pub step: T,
}

impl<T> ValueRange<T> {
    pub const fn new(min: T, max: T, step: T) -> Self {
        Self { min, max, step }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueDomain<T: 'static> {
    Ranges(&'static [ValueRange<T>]),
    Values(&'static [T]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlCapability<T: 'static> {
    pub access: Capability,
    pub domain: ValueDomain<T>,
}

impl<T> ControlCapability<T> {
    pub const fn ranges(access: Capability, ranges: &'static [ValueRange<T>]) -> Self {
        Self {
            access,
            domain: ValueDomain::Ranges(ranges),
        }
    }

    pub const fn values(access: Capability, values: &'static [T]) -> Self {
        Self {
            access,
            domain: ValueDomain::Values(values),
        }
    }

    pub const fn can_read(self) -> bool {
        self.access.can_read()
    }
    pub const fn can_write(self) -> bool {
        self.access.can_write()
    }
}

pub trait SteppedValue: PartialEq + PartialOrd {
    fn is_step_aligned(value: &Self, min: &Self, step: &Self) -> bool;
}

macro_rules! impl_unsigned_step {
    ($($ty:ty),+ $(,)?) => {$(
        impl SteppedValue for $ty {
            fn is_step_aligned(value: &Self, min: &Self, step: &Self) -> bool {
                *step != 0 && value.checked_sub(*min).is_some_and(|offset| offset % *step == 0)
            }
        }
    )+};
}

impl_unsigned_step!(u8, u16, u32, u64);

impl SteppedValue for i16 {
    fn is_step_aligned(value: &Self, min: &Self, step: &Self) -> bool {
        *step > 0 && (i32::from(*value) - i32::from(*min)) % i32::from(*step) == 0
    }
}

impl SteppedValue for Frequency {
    fn is_step_aligned(value: &Self, min: &Self, step: &Self) -> bool {
        step.hz() != 0
            && value
                .hz()
                .checked_sub(min.hz())
                .is_some_and(|offset| offset % step.hz() == 0)
    }
}

impl<T: SteppedValue> ValueDomain<T> {
    pub fn contains(&self, value: &T) -> bool {
        match self {
            Self::Ranges(ranges) => ranges.iter().any(|range| {
                value >= &range.min
                    && value <= &range.max
                    && T::is_step_aligned(value, &range.min, &range.step)
            }),
            Self::Values(values) => values.contains(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeValueDomain<T: 'static> {
    pub modes: &'static [Mode],
    pub domain: ValueDomain<T>,
}

pub type IndexedControl = ControlCapability<u8>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataPttRelationship {
    Shared,
    Distinct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerStep {
    Fixed(Power),
    Linear { intervals: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PowerRange {
    pub min: Power,
    pub max: Power,
    pub step: PowerStep,
}

impl PowerRange {
    pub const fn fixed(min: Power, max: Power, step: Power) -> Self {
        Self {
            min,
            max,
            step: PowerStep::Fixed(step),
        }
    }

    pub const fn linear(min: Power, max: Power, intervals: u16) -> Self {
        Self {
            min,
            max,
            step: PowerStep::Linear { intervals },
        }
    }

    fn quantize(self, requested: Power) -> Option<(Power, u128, u128)> {
        let requested = requested.as_microwatts();
        let min = self.min.as_microwatts();
        let max = self.max.as_microwatts();
        if requested < min || requested > max || min > max {
            return None;
        }

        let span = u128::from(max - min);
        let offset = u128::from(requested - min);
        match self.step {
            PowerStep::Fixed(step) => {
                let step = u128::from(step.as_microwatts());
                if step == 0 {
                    return None;
                }
                let max_index = span / step;
                let index = ((offset + step / 2) / step).min(max_index);
                let accepted = u128::from(min) + index * step;
                Some((Power::from_microwatts(accepted as u64), step, 1))
            }
            PowerStep::Linear { intervals } => {
                let intervals = u128::from(intervals);
                if intervals == 0 || span == 0 {
                    return None;
                }
                let index = ((offset * intervals + span / 2) / span).min(intervals);
                let accepted = u128::from(min) + (index * span + intervals / 2) / intervals;
                Some((Power::from_microwatts(accepted as u64), span, intervals))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerCapability {
    pub access: Capability,
    pub ranges: &'static [PowerRange],
}

impl PowerCapability {
    pub const fn new(access: Capability, ranges: &'static [PowerRange]) -> Self {
        Self { access, ranges }
    }

    pub const fn unsupported() -> Self {
        Self::new(Capability::Unsupported, &[])
    }

    pub const fn is_supported(self) -> bool {
        self.access.is_supported()
    }

    pub const fn can_read(self) -> bool {
        self.access.can_read()
    }

    pub const fn can_write(self) -> bool {
        self.access.can_write()
    }

    pub fn quantize(self, requested: Power) -> Option<Power> {
        self.ranges
            .iter()
            .filter_map(|range| range.quantize(requested))
            .min_by(|left, right| (left.1 * right.2).cmp(&(right.1 * left.2)))
            .map(|candidate| candidate.0)
    }
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
            rit_xit: RitXitCapabilities {
                offset_type: RitXitOffsetType::Independent,
                ..RitXitCapabilities::all()
            },
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

    pub fn validate(&self) -> crate::Result<()> {
        if matches!(self.receiver_kind, ReceiverKind::SingleVfo) && self.sub_rx.is_some() {
            return invalid_capabilities("single-VFO profiles cannot expose a sub receiver");
        }
        if !matches!(self.receiver_kind, ReceiverKind::SingleVfo) && self.sub_rx.is_none() {
            return invalid_capabilities("dual receiver profiles must expose a sub receiver");
        }
        validate_receiver(&self.main_rx, "main_rx")?;
        if let Some(receiver) = &self.sub_rx {
            validate_receiver(receiver, "sub_rx")?;
        }
        if let Some(tx) = &self.tx {
            if tx.frequency.is_supported() && tx.frequency_ranges.is_empty() {
                return invalid_capabilities("writable/readable TX frequency requires ranges");
            }
            if tx.mode.is_supported() && tx.modes.is_empty() {
                return invalid_capabilities("writable/readable TX mode requires values");
            }
            validate_frequency_ranges(tx.frequency_ranges, "TX frequency")?;
            for range in tx.power.ranges {
                if range.min > range.max {
                    return invalid_capabilities("TX power range minimum exceeds maximum");
                }
                match range.step {
                    PowerStep::Fixed(step) if step.as_microwatts() == 0 => {
                        return invalid_capabilities("TX power step must be non-zero");
                    }
                    PowerStep::Linear { intervals: 0 } => {
                        return invalid_capabilities("TX power intervals must be non-zero");
                    }
                    PowerStep::Fixed(_) | PowerStep::Linear { .. } => {}
                }
            }
        }
        if let Some(keyer) = self.keyer {
            if keyer.speed_wpm.is_supported() && keyer.speed_range_wpm.is_none() {
                return invalid_capabilities("supported keyer speed requires a range");
            }
            if let Some(range) = keyer.speed_range_wpm {
                if range.min > range.max || range.step == 0 {
                    return invalid_capabilities("keyer speed range is invalid");
                }
            }
        }
        let offset = self.rit_xit.offset_range;
        if offset.min > offset.max || offset.step.as_hz() <= 0 {
            return invalid_capabilities("RIT/XIT offset range is invalid");
        }
        Ok(())
    }

    pub fn validate_command(
        &self,
        command: &crate::RadioCommand,
        state: &crate::RadioState,
    ) -> crate::Result<()> {
        use crate::{RadioCommand as C, ReceiverPath};

        match command {
            C::SetReceiverFrequency {
                receiver,
                frequency,
            } => {
                let caps = self.receiver(*receiver)?;
                require_write(caps.frequency, "receiver.frequency")?;
                require_in_ranges(frequency, caps.frequency_ranges, "receiver.frequency")
            }
            C::SetReceiverMode { receiver, mode } => {
                let caps = self.receiver(*receiver)?;
                require_write(caps.mode, "receiver.mode")?;
                require_value(mode, caps.modes, "receiver.mode")
            }
            C::SetReceiverFilterBandwidth {
                receiver,
                bandwidth_hz,
            } => {
                let caps = self.receiver(*receiver)?;
                require_write(caps.filter_bandwidth, "receiver.filter_bandwidth")?;
                validate_mode_domain(
                    *bandwidth_hz,
                    caps.filter_bandwidths,
                    receiver_mode(state, *receiver),
                    "receiver.filter_bandwidth",
                )
            }
            C::SetReceiverFilterShift { receiver, shift_hz } => {
                let caps = self.receiver(*receiver)?;
                require_write(caps.filter_shift, "receiver.filter_shift")?;
                validate_mode_domain(
                    *shift_hz,
                    caps.filter_shifts,
                    receiver_mode(state, *receiver),
                    "receiver.filter_shift",
                )
            }
            C::SetReceiverPreamp { receiver, setting } => validate_indexed(
                self.receiver(*receiver)?.rf.preamp,
                self.receiver(*receiver)?.rf.preamp_values,
                *setting,
                "receiver.preamp",
            ),
            C::SetReceiverAttenuator { receiver, setting } => validate_indexed(
                self.receiver(*receiver)?.rf.attenuator,
                self.receiver(*receiver)?.rf.attenuator_values,
                *setting,
                "receiver.attenuator",
            ),
            C::SetReceiverNoiseBlanker { receiver, setting } => validate_indexed(
                self.receiver(*receiver)?.rf.noise_blanker,
                self.receiver(*receiver)?.rf.noise_blanker_values,
                *setting,
                "receiver.noise_blanker",
            ),
            C::SetReceiverNoiseReduction { receiver, setting } => validate_indexed(
                self.receiver(*receiver)?.rf.noise_reduction,
                self.receiver(*receiver)?.rf.noise_reduction_values,
                *setting,
                "receiver.noise_reduction",
            ),
            C::SetReceiverAutoNotch { receiver, .. } => require_write(
                self.receiver(*receiver)?.rf.auto_notch,
                "receiver.auto_notch",
            ),
            C::SetTxFrequency(frequency) => {
                let tx = self.transmitter()?;
                require_write(tx.frequency, "tx.frequency")?;
                require_in_ranges(frequency, tx.frequency_ranges, "tx.frequency")
            }
            C::SetTxMode(mode) => {
                let tx = self.transmitter()?;
                require_write(tx.mode, "tx.mode")?;
                require_value(mode, tx.modes, "tx.mode")
            }
            C::SetTxPower(_) => require_write(self.transmitter()?.power.access, "tx.power"),
            C::SetPtt(_) => require_write(self.transmitter()?.ptt, "tx.ptt"),
            C::SetDataPtt(_) => require_write(self.transmitter()?.data_ptt, "tx.data_ptt"),
            C::SetSplit(_) => require_write(self.transmitter()?.split, "tx.split"),
            C::SetRitEnabled { receiver, .. } => require_write(
                match receiver {
                    ReceiverPath::Main => self.rit_xit.main_rit_enabled,
                    ReceiverPath::Sub => self.rit_xit.sub_rit_enabled,
                },
                "rit.enabled",
            ),
            C::SetXitEnabled { receiver, .. } => require_write(
                match receiver {
                    ReceiverPath::Main => self.rit_xit.xit_enabled,
                    ReceiverPath::Sub => self.rit_xit.sub_xit_enabled,
                },
                "xit.enabled",
            ),
            C::SetRitOffset { receiver, offset }
            | C::SetXitOffset { receiver, offset }
            | C::SetRitXitOffset { receiver, offset } => {
                let access = match receiver {
                    ReceiverPath::Main => self.rit_xit.offset,
                    ReceiverPath::Sub => self.rit_xit.sub_offset,
                };
                require_write(access, "rit_xit.offset")?;
                let value = offset.as_hz();
                let range = self.rit_xit.offset_range;
                if value < range.min.as_hz() || value > range.max.as_hz() {
                    return invalid_value("rit_xit.offset", "outside advertised range");
                }
                Ok(())
            }
            C::SetKeyerSpeed(wpm) => {
                let keyer = self.keyer.ok_or(crate::RadioError::UnsupportedCapability {
                    capability: "keyer",
                })?;
                require_write(keyer.speed_wpm, "keyer.speed_wpm")?;
                let range = keyer.speed_range_wpm.expect("validated keyer range");
                if *wpm < range.min || *wpm > range.max {
                    return invalid_value("keyer.speed_wpm", "outside advertised range");
                }
                Ok(())
            }
            C::SendCw(_) => require_write(
                self.keyer
                    .ok_or(crate::RadioError::UnsupportedCapability {
                        capability: "keyer",
                    })?
                    .send_cw,
                "keyer.send_cw",
            ),
            C::StopCw => require_write(
                self.keyer
                    .ok_or(crate::RadioError::UnsupportedCapability {
                        capability: "keyer",
                    })?
                    .stop_cw,
                "keyer.stop_cw",
            ),
            C::Refresh => Ok(()),
        }
    }

    fn receiver(&self, receiver: crate::ReceiverPath) -> crate::Result<&ReceiverCapabilities> {
        match receiver {
            crate::ReceiverPath::Main => Ok(&self.main_rx),
            crate::ReceiverPath::Sub => {
                self.sub_rx
                    .as_ref()
                    .ok_or(crate::RadioError::UnsupportedCapability {
                        capability: "sub_rx",
                    })
            }
        }
    }

    fn transmitter(&self) -> crate::Result<&TransmitterCapabilities> {
        self.tx
            .as_ref()
            .ok_or(crate::RadioError::UnsupportedCapability { capability: "tx" })
    }
}

fn receiver_mode(state: &crate::RadioState, receiver: crate::ReceiverPath) -> Option<Mode> {
    match receiver {
        crate::ReceiverPath::Main => state.main_rx().mode(),
        crate::ReceiverPath::Sub => state.sub_rx().and_then(|rx| rx.mode()),
    }
}

fn require_write(access: Capability, field: &'static str) -> crate::Result<()> {
    if access.can_write() {
        Ok(())
    } else {
        Err(crate::RadioError::UnsupportedCapability { capability: field })
    }
}

fn require_in_ranges<T: SteppedValue>(
    value: &T,
    ranges: &[ValueRange<T>],
    field: &'static str,
) -> crate::Result<()> {
    if ranges.iter().any(|range| {
        value >= &range.min
            && value <= &range.max
            && T::is_step_aligned(value, &range.min, &range.step)
    }) {
        Ok(())
    } else {
        invalid_value(field, "outside advertised ranges")
    }
}

fn require_value<T: PartialEq>(value: &T, values: &[T], field: &'static str) -> crate::Result<()> {
    if values.contains(value) {
        Ok(())
    } else {
        invalid_value(field, "not in advertised values")
    }
}

fn validate_mode_domain<T: Copy + SteppedValue>(
    value: T,
    domains: &[ModeValueDomain<T>],
    mode: Option<Mode>,
    field: &'static str,
) -> crate::Result<()> {
    let accepted = domains.iter().any(|constraint| {
        mode.is_none_or(|mode| constraint.modes.contains(&mode))
            && constraint.domain.contains(&value)
    });
    if accepted {
        Ok(())
    } else {
        invalid_value(field, "outside advertised domain for the current mode")
    }
}

fn validate_indexed(
    access: Capability,
    values: &[u8],
    setting: crate::LeveledSetting,
    field: &'static str,
) -> crate::Result<()> {
    require_write(access, field)?;
    match setting.level() {
        None => Ok(()),
        Some(level) => require_value(&level, values, field),
    }
}

fn invalid_value(field: &'static str, message: &str) -> crate::Result<()> {
    Err(crate::RadioError::InvalidValue {
        field,
        message: message.to_string(),
    })
}

fn validate_receiver(receiver: &ReceiverCapabilities, name: &str) -> crate::Result<()> {
    if receiver.frequency.is_supported() && receiver.frequency_ranges.is_empty() {
        return invalid_capabilities(&format!("{name} frequency requires ranges"));
    }
    if receiver.mode.is_supported() && receiver.modes.is_empty() {
        return invalid_capabilities(&format!("{name} mode requires values"));
    }
    validate_frequency_ranges(receiver.frequency_ranges, name)?;
    if receiver.filter_bandwidth.is_supported() && receiver.filter_bandwidths.is_empty() {
        return invalid_capabilities(&format!("{name} filter bandwidth requires domains"));
    }
    if receiver.filter_shift.is_supported() && receiver.filter_shifts.is_empty() {
        return invalid_capabilities(&format!("{name} filter shift requires domains"));
    }
    for (access, values, control) in [
        (receiver.rf.preamp, receiver.rf.preamp_values, "preamp"),
        (
            receiver.rf.attenuator,
            receiver.rf.attenuator_values,
            "attenuator",
        ),
        (
            receiver.rf.noise_blanker,
            receiver.rf.noise_blanker_values,
            "noise blanker",
        ),
        (
            receiver.rf.noise_reduction,
            receiver.rf.noise_reduction_values,
            "noise reduction",
        ),
    ] {
        if access.is_supported() && values.is_empty() {
            return invalid_capabilities(&format!("{name} {control} requires values"));
        }
        if !access.is_supported() && !values.is_empty() {
            return invalid_capabilities(&format!("unsupported {name} {control} has values"));
        }
    }
    Ok(())
}

fn validate_frequency_ranges(ranges: &[ValueRange<Frequency>], name: &str) -> crate::Result<()> {
    if ranges
        .iter()
        .any(|range| range.min > range.max || range.step.hz() == 0)
    {
        return invalid_capabilities(&format!("{name} frequency range is invalid"));
    }
    Ok(())
}

fn invalid_capabilities(message: &str) -> crate::Result<()> {
    Err(crate::RadioError::InvalidValue {
        field: "capabilities",
        message: message.to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverCapabilities {
    pub frequency: Capability,
    pub frequency_ranges: &'static [ValueRange<Frequency>],
    pub mode: Capability,
    pub modes: &'static [Mode],
    pub filter_bandwidth: Capability,
    pub filter_bandwidths: &'static [ModeValueDomain<u16>],
    pub filter_shift: Capability,
    pub filter_shifts: &'static [ModeValueDomain<i16>],
    pub rf: ReceiverRfCapabilities,
}

impl ReceiverCapabilities {
    pub const fn all() -> Self {
        Self {
            frequency: Capability::ReadWrite,
            frequency_ranges: DEFAULT_RX_FREQUENCY_RANGES,
            mode: Capability::ReadWrite,
            modes: Mode::ALL,
            filter_bandwidth: Capability::ReadWrite,
            filter_bandwidths: DEFAULT_FILTER_BANDWIDTHS,
            filter_shift: Capability::ReadWrite,
            filter_shifts: DEFAULT_FILTER_SHIFTS,
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
            frequency_ranges: if frequency.is_supported() {
                DEFAULT_RX_FREQUENCY_RANGES
            } else {
                &[]
            },
            mode,
            modes: if mode.is_supported() { Mode::ALL } else { &[] },
            filter_bandwidth,
            filter_bandwidths: if filter_bandwidth.is_supported() {
                DEFAULT_FILTER_BANDWIDTHS
            } else {
                &[]
            },
            filter_shift,
            filter_shifts: if filter_shift.is_supported() {
                DEFAULT_FILTER_SHIFTS
            } else {
                &[]
            },
            rf,
        }
    }

    pub const fn with_constraints(
        mut self,
        frequency_ranges: &'static [ValueRange<Frequency>],
        modes: &'static [Mode],
        filter_bandwidths: &'static [ModeValueDomain<u16>],
        filter_shifts: &'static [ModeValueDomain<i16>],
    ) -> Self {
        self.frequency_ranges = frequency_ranges;
        self.modes = modes;
        self.filter_bandwidths = filter_bandwidths;
        self.filter_shifts = filter_shifts;
        self
    }
}

const DEFAULT_RX_FREQUENCY_RANGES: &[ValueRange<Frequency>] = &[ValueRange::new(
    Frequency::from_hz(0),
    Frequency::from_hz(1_300_000_000),
    Frequency::from_hz(1),
)];
const DEFAULT_FILTER_BANDWIDTH_RANGES: &[ValueRange<u16>] = &[ValueRange::new(50, 12_000, 10)];
const DEFAULT_FILTER_SHIFT_RANGES: &[ValueRange<i16>] = &[ValueRange::new(-9_999, 9_999, 1)];
const DEFAULT_FILTER_BANDWIDTHS: &[ModeValueDomain<u16>] = &[ModeValueDomain {
    modes: Mode::ALL,
    domain: ValueDomain::Ranges(DEFAULT_FILTER_BANDWIDTH_RANGES),
}];
const DEFAULT_FILTER_SHIFTS: &[ModeValueDomain<i16>] = &[ModeValueDomain {
    modes: Mode::ALL,
    domain: ValueDomain::Ranges(DEFAULT_FILTER_SHIFT_RANGES),
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverRfCapabilities {
    pub preamp: Capability,
    pub preamp_values: &'static [u8],
    pub attenuator: Capability,
    pub attenuator_values: &'static [u8],
    pub noise_blanker: Capability,
    pub noise_blanker_values: &'static [u8],
    pub noise_reduction: Capability,
    pub noise_reduction_values: &'static [u8],
    pub auto_notch: Capability,
}

impl ReceiverRfCapabilities {
    pub const fn all() -> Self {
        Self {
            preamp: Capability::ReadWrite,
            preamp_values: &[1, 2],
            attenuator: Capability::ReadWrite,
            attenuator_values: &LEVELS_1_24,
            noise_blanker: Capability::ReadWrite,
            noise_blanker_values: &LEVELS_1_100,
            noise_reduction: Capability::ReadWrite,
            noise_reduction_values: &LEVELS_1_100,
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
            preamp_values: if preamp.is_supported() { &[1, 2] } else { &[] },
            attenuator,
            attenuator_values: if attenuator.is_supported() {
                &LEVELS_1_24
            } else {
                &[]
            },
            noise_blanker,
            noise_blanker_values: if noise_blanker.is_supported() {
                &LEVELS_1_100
            } else {
                &[]
            },
            noise_reduction,
            noise_reduction_values: if noise_reduction.is_supported() {
                &LEVELS_1_100
            } else {
                &[]
            },
            auto_notch,
        }
    }

    pub const fn with_values(
        mut self,
        preamp: &'static [u8],
        attenuator: &'static [u8],
        noise_blanker: &'static [u8],
        noise_reduction: &'static [u8],
    ) -> Self {
        self.preamp_values = preamp;
        self.attenuator_values = attenuator;
        self.noise_blanker_values = noise_blanker;
        self.noise_reduction_values = noise_reduction;
        self
    }
}

const LEVELS_1_24: [u8; 24] = sequential_levels::<24>();
const LEVELS_1_100: [u8; 100] = sequential_levels::<100>();

const fn sequential_levels<const N: usize>() -> [u8; N] {
    let mut values = [0; N];
    let mut index = 0;
    while index < N {
        values[index] = (index + 1) as u8;
        index += 1;
    }
    values
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransmitterCapabilities {
    pub frequency: Capability,
    pub frequency_ranges: &'static [ValueRange<Frequency>],
    pub mode: Capability,
    pub modes: &'static [Mode],
    pub power: PowerCapability,
    pub ptt: Capability,
    pub data_ptt: Capability,
    pub data_ptt_relationship: DataPttRelationship,
    pub split: Capability,
}

const DEFAULT_POWER_RANGES: &[PowerRange] = &[PowerRange::fixed(
    Power::from_microwatts(0),
    Power::from_microwatts(100_000_000),
    Power::from_microwatts(1_000_000),
)];

impl TransmitterCapabilities {
    pub const fn all() -> Self {
        Self {
            frequency: Capability::ReadWrite,
            frequency_ranges: DEFAULT_TX_FREQUENCY_RANGES,
            mode: Capability::ReadWrite,
            modes: Mode::ALL,
            power: PowerCapability::new(Capability::ReadWrite, DEFAULT_POWER_RANGES),
            ptt: Capability::ReadWrite,
            data_ptt: Capability::ReadWrite,
            data_ptt_relationship: DataPttRelationship::Shared,
            split: Capability::ReadWrite,
        }
    }

    pub const fn new(
        frequency: Capability,
        mode: Capability,
        power: PowerCapability,
        ptt: Capability,
        split: Capability,
    ) -> Self {
        Self {
            frequency,
            frequency_ranges: if frequency.is_supported() {
                DEFAULT_TX_FREQUENCY_RANGES
            } else {
                &[]
            },
            mode,
            modes: if mode.is_supported() { Mode::ALL } else { &[] },
            power,
            ptt,
            data_ptt: ptt,
            data_ptt_relationship: DataPttRelationship::Shared,
            split,
        }
    }

    pub const fn with_constraints(
        mut self,
        frequency_ranges: &'static [ValueRange<Frequency>],
        modes: &'static [Mode],
    ) -> Self {
        self.frequency_ranges = frequency_ranges;
        self.modes = modes;
        self
    }

    pub const fn with_data_ptt(
        mut self,
        data_ptt: Capability,
        relationship: DataPttRelationship,
    ) -> Self {
        self.data_ptt = data_ptt;
        self.data_ptt_relationship = relationship;
        self
    }
}

const DEFAULT_TX_FREQUENCY_RANGES: &[ValueRange<Frequency>] = &[ValueRange::new(
    Frequency::from_hz(1_800_000),
    Frequency::from_hz(54_000_000),
    Frequency::from_hz(1),
)];

#[cfg(test)]
mod power_tests {
    use super::*;

    const SEGMENTS: &[PowerRange] = &[
        PowerRange::fixed(
            Power::from_microwatts(100_000),
            Power::from_microwatts(10_000_000),
            Power::from_microwatts(100_000),
        ),
        PowerRange::fixed(
            Power::from_microwatts(1_000_000),
            Power::from_microwatts(110_000_000),
            Power::from_microwatts(1_000_000),
        ),
    ];
    const CAPABILITY: PowerCapability = PowerCapability::new(Capability::ReadWrite, SEGMENTS);

    #[test]
    fn fixed_ranges_quantize_up_at_midpoints_and_prefer_finer_overlaps() {
        assert_eq!(
            CAPABILITY.quantize(Power::from_microwatts(1_050_000)),
            Some(Power::from_microwatts(1_100_000))
        );
        assert_eq!(
            CAPABILITY.quantize(Power::from_microwatts(1_040_000)),
            Some(Power::from_microwatts(1_000_000))
        );
    }

    #[test]
    fn gaps_and_out_of_range_values_are_rejected() {
        assert_eq!(CAPABILITY.quantize(Power::from_microwatts(99_999)), None);
        assert_eq!(
            CAPABILITY.quantize(Power::from_microwatts(110_000_001)),
            None
        );
    }

    #[test]
    fn linear_ranges_quantize_to_exact_interval_values() {
        const RANGES: &[PowerRange] = &[PowerRange::linear(
            Power::from_microwatts(0),
            Power::from_microwatts(10_000_000),
            255,
        )];
        let capability = PowerCapability::new(Capability::ReadWrite, RANGES);
        assert_eq!(
            capability.quantize(Power::from_microwatts(5_000_000)),
            Some(Power::from_microwatts(5_019_608))
        );
        assert_eq!(
            capability.quantize(Power::from_microwatts(10_000_001)),
            None
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RitXitOffsetType {
    Shared,
    Independent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RitXitCapabilities {
    pub main_rit_enabled: Capability,
    pub sub_rit_enabled: Capability,
    pub xit_enabled: Capability,
    pub sub_xit_enabled: Capability,
    pub offset: Capability,
    pub sub_offset: Capability,
    pub offset_type: RitXitOffsetType,
    pub offset_range: ValueRange<RitXitOffsetHz>,
}

impl RitXitCapabilities {
    pub const fn all() -> Self {
        Self {
            main_rit_enabled: Capability::ReadWrite,
            sub_rit_enabled: Capability::ReadWrite,
            xit_enabled: Capability::ReadWrite,
            sub_xit_enabled: Capability::ReadWrite,
            offset: Capability::ReadWrite,
            sub_offset: Capability::ReadWrite,
            offset_type: RitXitOffsetType::Shared,
            offset_range: DEFAULT_RIT_XIT_RANGE,
        }
    }

    pub const fn new(
        main_rit_enabled: Capability,
        sub_rit_enabled: Capability,
        xit_enabled: Capability,
        offset: Capability,
        sub_offset: Capability,
        offset_type: RitXitOffsetType,
    ) -> Self {
        Self {
            main_rit_enabled,
            sub_rit_enabled,
            xit_enabled,
            sub_xit_enabled: sub_rit_enabled,
            offset,
            sub_offset,
            offset_type,
            offset_range: DEFAULT_RIT_XIT_RANGE,
        }
    }
}

const DEFAULT_RIT_XIT_RANGE: ValueRange<RitXitOffsetHz> = ValueRange::new(
    RitXitOffsetHz::MIN_VALUE,
    RitXitOffsetHz::MAX_VALUE,
    RitXitOffsetHz::ONE_HZ,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyerCapabilities {
    pub speed_wpm: Capability,
    pub speed_range_wpm: Option<ValueRange<u8>>,
    pub sending: Capability,
    pub send_cw: Capability,
    pub stop_cw: Capability,
}

impl KeyerCapabilities {
    pub const fn all() -> Self {
        Self {
            speed_wpm: Capability::ReadWrite,
            speed_range_wpm: Some(ValueRange::new(4, 60, 1)),
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
            speed_range_wpm: if speed_wpm.is_supported() {
                Some(ValueRange::new(4, 60, 1))
            } else {
                None
            },
            sending,
            send_cw,
            stop_cw,
        }
    }

    pub const fn with_speed_range(mut self, min: u8, max: u8) -> Self {
        self.speed_range_wpm = Some(ValueRange::new(min, max, 1));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateUpdateCapability {
    Native,
    Polling,
    Hybrid,
}

#[cfg(test)]
mod constraint_tests {
    use super::*;

    #[test]
    fn iaru_regions_have_stable_names_and_descriptions() {
        assert_eq!(
            "1".parse::<RadioRegion>().unwrap(),
            RadioRegion::IaruRegion1
        );
        assert_eq!(
            "IARU-Region-2".parse::<RadioRegion>().unwrap(),
            RadioRegion::IaruRegion2
        );
        assert_eq!(RadioRegion::IaruRegion3.to_string(), "IARURegion3");
        assert!(RadioRegion::IaruRegion1.description().contains("Europe"));
        assert!("general".parse::<RadioRegion>().is_err());
    }

    #[test]
    fn value_domains_support_ranges_and_discrete_values() {
        const RANGES: &[ValueRange<u8>] = &[ValueRange::new(4, 60, 1)];
        const VALUES: &[u8] = &[1, 3, 7];
        assert!(ValueDomain::Ranges(RANGES).contains(&30));
        assert!(!ValueDomain::Ranges(RANGES).contains(&61));
        assert!(ValueDomain::Values(VALUES).contains(&3));
        assert!(!ValueDomain::Values(VALUES).contains(&2));
    }

    #[test]
    fn built_in_constraint_shape_is_valid() {
        RadioCapabilities::dummy_all().validate().unwrap();
        assert!(!RadioCapabilities::dummy_all().main_rx.modes.is_empty());
        assert!(!RadioCapabilities::dummy_all()
            .main_rx
            .frequency_ranges
            .is_empty());
    }
}
