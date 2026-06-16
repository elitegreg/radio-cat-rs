use std::sync::Arc;

use bitflags::bitflags;
use smallvec::SmallVec;

use crate::{
    ConnectionState, Frequency, KeyerState, LeveledSetting, Mode, Power, RadioState, ReceiverState,
    RitXitOffsetHz, TransmitterState,
};

pub type SharedRadioState = Arc<RadioState>;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ChangeFlags: u32 {
        const NONE                  = 0;

        const MAIN_RX_FREQ          = 1 << 0;
        const MAIN_RX_MODE          = 1 << 1;
        const MAIN_RX_FILTER_BW     = 1 << 2;
        const MAIN_RX_FILTER_SHIFT  = 1 << 3;
        const MAIN_RX_RF            = 1 << 4;

        const SUB_RX                = 1 << 5;
        const SUB_RX_FREQ           = 1 << 6;
        const SUB_RX_MODE           = 1 << 7;
        const SUB_RX_FILTER_BW      = 1 << 8;
        const SUB_RX_FILTER_SHIFT   = 1 << 9;
        const SUB_RX_RF             = 1 << 10;

        const TX                    = 1 << 11;
        const TX_FREQ               = 1 << 12;
        const TX_MODE               = 1 << 13;
        const TX_POWER              = 1 << 14;
        const PTT                   = 1 << 15;
        const SPLIT                 = 1 << 16;

        const RIT_XIT               = 1 << 17;
        const KEYER                 = 1 << 18;
        const CONNECTION            = 1 << 19;

        const OTHER                 = 1 << 31;

        const FREQUENCY =
            Self::MAIN_RX_FREQ.bits()
            | Self::SUB_RX_FREQ.bits()
            | Self::TX_FREQ.bits();

        const MODE =
            Self::MAIN_RX_MODE.bits()
            | Self::SUB_RX_MODE.bits()
            | Self::TX_MODE.bits();

        const FILTER =
            Self::MAIN_RX_FILTER_BW.bits()
            | Self::MAIN_RX_FILTER_SHIFT.bits()
            | Self::SUB_RX_FILTER_BW.bits()
            | Self::SUB_RX_FILTER_SHIFT.bits();

        const RECEIVER =
            Self::MAIN_RX_FREQ.bits()
            | Self::MAIN_RX_MODE.bits()
            | Self::MAIN_RX_FILTER_BW.bits()
            | Self::MAIN_RX_FILTER_SHIFT.bits()
            | Self::MAIN_RX_RF.bits()
            | Self::SUB_RX.bits()
            | Self::SUB_RX_FREQ.bits()
            | Self::SUB_RX_MODE.bits()
            | Self::SUB_RX_FILTER_BW.bits()
            | Self::SUB_RX_FILTER_SHIFT.bits()
            | Self::SUB_RX_RF.bits();

        const TRANSMITTER =
            Self::TX.bits()
            | Self::TX_FREQ.bits()
            | Self::TX_MODE.bits()
            | Self::TX_POWER.bits()
            | Self::PTT.bits()
            | Self::SPLIT.bits();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateField {
    MainRxFrequency,
    MainRxMode,
    MainRxFilterBandwidth,
    MainRxFilterShift,
    MainRxPreamp,
    MainRxAttenuator,
    MainRxNoiseBlanker,
    MainRxNoiseReduction,
    MainRxAutoNotch,

    SubRxPresent,
    SubRxFrequency,
    SubRxMode,
    SubRxFilterBandwidth,
    SubRxFilterShift,
    SubRxPreamp,
    SubRxAttenuator,
    SubRxNoiseBlanker,
    SubRxNoiseReduction,
    SubRxAutoNotch,

    TxPresent,
    TxFrequency,
    TxMode,
    TxPower,
    Transmitting,
    Split,

    RitEnabled,
    XitEnabled,
    RitXitOffset,

    KeyerPresent,
    KeyerSpeed,
    KeyerSending,

    Connection,
    Other(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateSource {
    Native,
    Poll,
    CommandResponse,
    ManualRefresh,
    Optimistic,
}

#[derive(Debug, Clone)]
pub struct StateUpdate {
    pub changes: ChangeFlags,
    pub fields: SmallVec<[StateField; 4]>,
    pub source: UpdateSource,
    pub state: SharedRadioState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    pub flags: ChangeFlags,
    pub fields: SmallVec<[StateField; 4]>,
}

impl Default for ChangeSet {
    fn default() -> Self {
        Self {
            flags: ChangeFlags::NONE,
            fields: SmallVec::new(),
        }
    }
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    fn add(&mut self, flags: ChangeFlags, field: StateField) {
        self.flags |= flags;
        self.fields.push(field);
    }

    fn extend(&mut self, other: ChangeSet) {
        self.flags |= other.flags;
        self.fields.extend(other.fields);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatePatch {
    MainRxFrequency(Frequency),
    MainRxMode(Mode),
    MainRxFilterBandwidth(u16),
    MainRxFilterShift(i16),
    MainRxPreamp(LeveledSetting),
    MainRxAttenuator(LeveledSetting),
    MainRxNoiseBlanker(LeveledSetting),
    MainRxNoiseReduction(LeveledSetting),
    MainRxAutoNotch(bool),

    SubRxPresent(bool),
    SubRxFrequency(Frequency),
    SubRxMode(Mode),
    SubRxFilterBandwidth(u16),
    SubRxFilterShift(i16),
    SubRxPreamp(LeveledSetting),
    SubRxAttenuator(LeveledSetting),
    SubRxNoiseBlanker(LeveledSetting),
    SubRxNoiseReduction(LeveledSetting),
    SubRxAutoNotch(bool),

    TxPresent(bool),
    TxFrequency(Frequency),
    TxMode(Mode),
    TxPower(Power),
    Transmitting(bool),
    Split(bool),

    RitEnabled(bool),
    XitEnabled(bool),
    RitXitOffset(RitXitOffsetHz),

    KeyerPresent(bool),
    KeyerSpeed(u8),
    KeyerSending(bool),

    Connection(ConnectionState),
}

#[derive(Debug, Clone)]
pub struct StateReducer {
    state: RadioState,
}

impl StateReducer {
    pub fn new(state: RadioState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> &RadioState {
        &self.state
    }

    pub fn snapshot(&self) -> SharedRadioState {
        Arc::new(self.state.clone())
    }

    pub fn apply_patches<I>(&mut self, patches: I) -> ChangeSet
    where
        I: IntoIterator<Item = StatePatch>,
    {
        let mut combined = ChangeSet::default();
        for patch in patches {
            combined.extend(self.apply_patch(patch));
        }
        combined
    }

    pub fn apply_patch(&mut self, patch: StatePatch) -> ChangeSet {
        let mut changes = ChangeSet::default();

        match patch {
            StatePatch::MainRxFrequency(value) => {
                if set_option(&mut self.state.main_rx.frequency, value) {
                    changes.add(ChangeFlags::MAIN_RX_FREQ, StateField::MainRxFrequency);
                }
            }
            StatePatch::MainRxMode(value) => {
                if set_option(&mut self.state.main_rx.mode, value) {
                    changes.add(ChangeFlags::MAIN_RX_MODE, StateField::MainRxMode);
                }
            }
            StatePatch::MainRxFilterBandwidth(value) => {
                if set_option(&mut self.state.main_rx.filter.bandwidth_hz, value) {
                    changes.add(
                        ChangeFlags::MAIN_RX_FILTER_BW,
                        StateField::MainRxFilterBandwidth,
                    );
                }
            }
            StatePatch::MainRxFilterShift(value) => {
                if set_option(&mut self.state.main_rx.filter.shift_hz, value) {
                    changes.add(
                        ChangeFlags::MAIN_RX_FILTER_SHIFT,
                        StateField::MainRxFilterShift,
                    );
                }
            }
            StatePatch::MainRxPreamp(value) => {
                if set_option(&mut self.state.main_rx.rf.preamp, value) {
                    changes.add(ChangeFlags::MAIN_RX_RF, StateField::MainRxPreamp);
                }
            }
            StatePatch::MainRxAttenuator(value) => {
                if set_option(&mut self.state.main_rx.rf.attenuator, value) {
                    changes.add(ChangeFlags::MAIN_RX_RF, StateField::MainRxAttenuator);
                }
            }
            StatePatch::MainRxNoiseBlanker(value) => {
                if set_option(&mut self.state.main_rx.rf.noise_blanker, value) {
                    changes.add(ChangeFlags::MAIN_RX_RF, StateField::MainRxNoiseBlanker);
                }
            }
            StatePatch::MainRxNoiseReduction(value) => {
                if set_option(&mut self.state.main_rx.rf.noise_reduction, value) {
                    changes.add(ChangeFlags::MAIN_RX_RF, StateField::MainRxNoiseReduction);
                }
            }
            StatePatch::MainRxAutoNotch(value) => {
                if set_option(&mut self.state.main_rx.rf.auto_notch, value) {
                    changes.add(ChangeFlags::MAIN_RX_RF, StateField::MainRxAutoNotch);
                }
            }

            StatePatch::SubRxPresent(present) => match (present, self.state.sub_rx.is_some()) {
                (true, false) => {
                    self.state.sub_rx = Some(ReceiverState::default());
                    changes.add(ChangeFlags::SUB_RX, StateField::SubRxPresent);
                }
                (false, true) => {
                    self.state.sub_rx = None;
                    changes.add(ChangeFlags::SUB_RX, StateField::SubRxPresent);
                }
                _ => {}
            },
            StatePatch::SubRxFrequency(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.frequency, value) {
                    changes.add(ChangeFlags::SUB_RX_FREQ, StateField::SubRxFrequency);
                }
            }
            StatePatch::SubRxMode(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.mode, value) {
                    changes.add(ChangeFlags::SUB_RX_MODE, StateField::SubRxMode);
                }
            }
            StatePatch::SubRxFilterBandwidth(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.filter.bandwidth_hz, value) {
                    changes.add(
                        ChangeFlags::SUB_RX_FILTER_BW,
                        StateField::SubRxFilterBandwidth,
                    );
                }
            }
            StatePatch::SubRxFilterShift(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.filter.shift_hz, value) {
                    changes.add(
                        ChangeFlags::SUB_RX_FILTER_SHIFT,
                        StateField::SubRxFilterShift,
                    );
                }
            }
            StatePatch::SubRxPreamp(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.rf.preamp, value) {
                    changes.add(ChangeFlags::SUB_RX_RF, StateField::SubRxPreamp);
                }
            }
            StatePatch::SubRxAttenuator(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.rf.attenuator, value) {
                    changes.add(ChangeFlags::SUB_RX_RF, StateField::SubRxAttenuator);
                }
            }
            StatePatch::SubRxNoiseBlanker(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.rf.noise_blanker, value) {
                    changes.add(ChangeFlags::SUB_RX_RF, StateField::SubRxNoiseBlanker);
                }
            }
            StatePatch::SubRxNoiseReduction(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.rf.noise_reduction, value) {
                    changes.add(ChangeFlags::SUB_RX_RF, StateField::SubRxNoiseReduction);
                }
            }
            StatePatch::SubRxAutoNotch(value) => {
                let rx = ensure_sub_rx(&mut self.state, &mut changes);
                if set_option(&mut rx.rf.auto_notch, value) {
                    changes.add(ChangeFlags::SUB_RX_RF, StateField::SubRxAutoNotch);
                }
            }

            StatePatch::TxPresent(present) => match (present, self.state.tx.is_some()) {
                (true, false) => {
                    self.state.tx = Some(TransmitterState::default());
                    changes.add(ChangeFlags::TX, StateField::TxPresent);
                }
                (false, true) => {
                    self.state.tx = None;
                    changes.add(ChangeFlags::TX, StateField::TxPresent);
                }
                _ => {}
            },
            StatePatch::TxFrequency(value) => {
                let tx = ensure_tx(&mut self.state, &mut changes);
                if set_option(&mut tx.frequency, value) {
                    changes.add(ChangeFlags::TX_FREQ, StateField::TxFrequency);
                }
            }
            StatePatch::TxMode(value) => {
                let tx = ensure_tx(&mut self.state, &mut changes);
                if set_option(&mut tx.mode, value) {
                    changes.add(ChangeFlags::TX_MODE, StateField::TxMode);
                }
            }
            StatePatch::TxPower(value) => {
                let tx = ensure_tx(&mut self.state, &mut changes);
                if set_option(&mut tx.power, value) {
                    changes.add(ChangeFlags::TX_POWER, StateField::TxPower);
                }
            }
            StatePatch::Transmitting(value) => {
                let tx = ensure_tx(&mut self.state, &mut changes);
                if set_option(&mut tx.transmitting, value) {
                    changes.add(ChangeFlags::PTT, StateField::Transmitting);
                }
            }
            StatePatch::Split(value) => {
                let tx = ensure_tx(&mut self.state, &mut changes);
                if set_option(&mut tx.split, value) {
                    changes.add(ChangeFlags::SPLIT, StateField::Split);
                }
            }

            StatePatch::RitEnabled(value) => {
                if set_option(&mut self.state.rit_xit.rit_enabled, value) {
                    changes.add(ChangeFlags::RIT_XIT, StateField::RitEnabled);
                }
            }
            StatePatch::XitEnabled(value) => {
                if set_option(&mut self.state.rit_xit.xit_enabled, value) {
                    changes.add(ChangeFlags::RIT_XIT, StateField::XitEnabled);
                }
            }
            StatePatch::RitXitOffset(value) => {
                if set_option(&mut self.state.rit_xit.offset_hz, value) {
                    changes.add(ChangeFlags::RIT_XIT, StateField::RitXitOffset);
                }
            }

            StatePatch::KeyerPresent(present) => match (present, self.state.keyer.is_some()) {
                (true, false) => {
                    self.state.keyer = Some(KeyerState::default());
                    changes.add(ChangeFlags::KEYER, StateField::KeyerPresent);
                }
                (false, true) => {
                    self.state.keyer = None;
                    changes.add(ChangeFlags::KEYER, StateField::KeyerPresent);
                }
                _ => {}
            },
            StatePatch::KeyerSpeed(value) => {
                let keyer = ensure_keyer(&mut self.state, &mut changes);
                if set_option(&mut keyer.speed_wpm, value) {
                    changes.add(ChangeFlags::KEYER, StateField::KeyerSpeed);
                }
            }
            StatePatch::KeyerSending(value) => {
                let keyer = ensure_keyer(&mut self.state, &mut changes);
                if set_option(&mut keyer.sending, value) {
                    changes.add(ChangeFlags::KEYER, StateField::KeyerSending);
                }
            }

            StatePatch::Connection(value) => {
                if set_value(&mut self.state.connection, value) {
                    changes.add(ChangeFlags::CONNECTION, StateField::Connection);
                }
            }
        }

        changes
    }
}

fn ensure_sub_rx<'a>(state: &'a mut RadioState, changes: &mut ChangeSet) -> &'a mut ReceiverState {
    if state.sub_rx.is_none() {
        state.sub_rx = Some(ReceiverState::default());
        changes.add(ChangeFlags::SUB_RX, StateField::SubRxPresent);
    }

    state.sub_rx.as_mut().expect("sub receiver just created")
}

fn ensure_tx<'a>(state: &'a mut RadioState, changes: &mut ChangeSet) -> &'a mut TransmitterState {
    if state.tx.is_none() {
        state.tx = Some(TransmitterState::default());
        changes.add(ChangeFlags::TX, StateField::TxPresent);
    }

    state.tx.as_mut().expect("transmitter just created")
}

fn ensure_keyer<'a>(state: &'a mut RadioState, changes: &mut ChangeSet) -> &'a mut KeyerState {
    if state.keyer.is_none() {
        state.keyer = Some(KeyerState::default());
        changes.add(ChangeFlags::KEYER, StateField::KeyerPresent);
    }

    state.keyer.as_mut().expect("keyer just created")
}

fn set_option<T>(slot: &mut Option<T>, value: T) -> bool
where
    T: PartialEq,
{
    if slot.as_ref() != Some(&value) {
        *slot = Some(value);
        true
    } else {
        false
    }
}

fn set_value<T>(slot: &mut T, value: T) -> bool
where
    T: PartialEq,
{
    if *slot != value {
        *slot = value;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Frequency, RadioState};

    #[test]
    fn reducer_emits_meaningful_changes_only() {
        let mut reducer = StateReducer::new(RadioState::default());
        let freq = Frequency::from_hz(14_074_000);

        let first = reducer.apply_patch(StatePatch::MainRxFrequency(freq));
        assert!(first.flags.contains(ChangeFlags::MAIN_RX_FREQ));
        assert_eq!(first.fields.as_slice(), &[StateField::MainRxFrequency]);

        let second = reducer.apply_patch(StatePatch::MainRxFrequency(freq));
        assert!(second.is_empty());
    }

    #[test]
    fn reducer_creates_optional_sub_receiver_when_patch_targets_it() {
        let mut reducer = StateReducer::new(RadioState::default());
        let changes = reducer.apply_patch(StatePatch::SubRxMode(Mode::Usb));

        assert!(reducer.state().sub_rx.is_some());
        assert!(changes.flags.contains(ChangeFlags::SUB_RX));
        assert!(changes.flags.contains(ChangeFlags::SUB_RX_MODE));
        assert_eq!(
            changes.fields.as_slice(),
            &[StateField::SubRxPresent, StateField::SubRxMode]
        );
    }
}
