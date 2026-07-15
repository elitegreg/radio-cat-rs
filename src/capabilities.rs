use crate::Power;

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
    pub power: PowerCapability,
    pub ptt: Capability,
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
            mode: Capability::ReadWrite,
            power: PowerCapability::new(Capability::ReadWrite, DEFAULT_POWER_RANGES),
            ptt: Capability::ReadWrite,
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
            mode,
            power,
            ptt,
            split,
        }
    }
}

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
    pub offset: Capability,
    pub sub_offset: Capability,
    pub offset_type: RitXitOffsetType,
}

impl RitXitCapabilities {
    pub const fn all() -> Self {
        Self {
            main_rit_enabled: Capability::ReadWrite,
            sub_rit_enabled: Capability::ReadWrite,
            xit_enabled: Capability::ReadWrite,
            offset: Capability::ReadWrite,
            sub_offset: Capability::ReadWrite,
            offset_type: RitXitOffsetType::Shared,
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
            offset,
            sub_offset,
            offset_type,
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
