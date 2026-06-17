use async_trait::async_trait;

use crate::{
    capabilities::RadioCapabilities,
    command::{RadioCommand, ReceiverPath},
    driver::{DriverCommandOutcome, DriverDescriptor, RadioDriver},
    error::Result,
    update::StatePatch,
    ConnectionState, Frequency, LeveledSetting, Mode, Power, RadioState, ReceiverFilterState,
    ReceiverRfState, ReceiverState, RitXitOffsetHz, RitXitState, TransmitterState,
};

pub const DUMMY_DRIVER: DriverDescriptor = DriverDescriptor {
    id: "dummy",
    display_name: "Dummy Radio",
    description: "In-memory radio driver that exposes every normalized v1 capability.",
};

#[derive(Debug, Clone, Default)]
pub struct DummyRadioDriver {
    options: String,
}

impl DummyRadioDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: impl Into<String>) -> Self {
        Self {
            options: options.into(),
        }
    }

    pub fn options(&self) -> &str {
        &self.options
    }
}

#[async_trait]
impl RadioDriver for DummyRadioDriver {
    fn descriptor(&self) -> DriverDescriptor {
        DUMMY_DRIVER
    }

    fn capabilities(&self) -> RadioCapabilities {
        RadioCapabilities::dummy_all()
    }

    fn initial_state(&self) -> RadioState {
        dummy_state(ConnectionState::Connecting)
    }

    async fn start(&mut self) -> Result<Vec<StatePatch>> {
        tracing::info!(options = %self.options, "dummy driver start");
        Ok(vec![StatePatch::Connection(ConnectionState::Ready)])
    }

    async fn handle_command(
        &mut self,
        command: RadioCommand,
        _current_state: &RadioState,
    ) -> Result<DriverCommandOutcome> {
        tracing::debug!(?command, "dummy driver handling command");
        let is_refresh = matches!(command, RadioCommand::Refresh);
        let patches = match command {
            RadioCommand::SetReceiverFrequency {
                receiver,
                frequency,
            } => receiver_patch(
                receiver,
                StatePatch::MainRxFrequency(frequency),
                StatePatch::SubRxFrequency(frequency),
            ),
            RadioCommand::SetReceiverMode { receiver, mode } => receiver_patch(
                receiver,
                StatePatch::MainRxMode(mode),
                StatePatch::SubRxMode(mode),
            ),
            RadioCommand::SetReceiverFilterBandwidth {
                receiver,
                bandwidth_hz,
            } => receiver_patch(
                receiver,
                StatePatch::MainRxFilterBandwidth(bandwidth_hz),
                StatePatch::SubRxFilterBandwidth(bandwidth_hz),
            ),
            RadioCommand::SetReceiverFilterShift { receiver, shift_hz } => receiver_patch(
                receiver,
                StatePatch::MainRxFilterShift(shift_hz),
                StatePatch::SubRxFilterShift(shift_hz),
            ),
            RadioCommand::SetReceiverPreamp { receiver, setting } => receiver_patch(
                receiver,
                StatePatch::MainRxPreamp(setting),
                StatePatch::SubRxPreamp(setting),
            ),
            RadioCommand::SetReceiverAttenuator { receiver, setting } => receiver_patch(
                receiver,
                StatePatch::MainRxAttenuator(setting),
                StatePatch::SubRxAttenuator(setting),
            ),
            RadioCommand::SetReceiverNoiseBlanker { receiver, setting } => receiver_patch(
                receiver,
                StatePatch::MainRxNoiseBlanker(setting),
                StatePatch::SubRxNoiseBlanker(setting),
            ),
            RadioCommand::SetReceiverNoiseReduction { receiver, setting } => receiver_patch(
                receiver,
                StatePatch::MainRxNoiseReduction(setting),
                StatePatch::SubRxNoiseReduction(setting),
            ),
            RadioCommand::SetReceiverAutoNotch { receiver, enabled } => receiver_patch(
                receiver,
                StatePatch::MainRxAutoNotch(enabled),
                StatePatch::SubRxAutoNotch(enabled),
            ),

            RadioCommand::SetTxFrequency(frequency) => vec![StatePatch::TxFrequency(frequency)],
            RadioCommand::SetTxMode(mode) => vec![StatePatch::TxMode(mode)],
            RadioCommand::SetTxPower(power) => vec![StatePatch::TxPower(power)],
            RadioCommand::SetPtt(transmitting) => vec![StatePatch::Transmitting(transmitting)],
            RadioCommand::SetSplit(split) => vec![StatePatch::Split(split)],

            RadioCommand::SetRitEnabled { receiver, enabled } => receiver_patch(
                receiver,
                StatePatch::MainRitEnabled(enabled),
                StatePatch::SubRitEnabled(enabled),
            ),
            RadioCommand::SetXitEnabled(enabled) => vec![StatePatch::XitEnabled(enabled)],
            RadioCommand::SetRitXitOffset(offset) => vec![StatePatch::RitXitOffset(offset)],

            RadioCommand::SetKeyerSpeed(wpm) => vec![StatePatch::KeyerSpeed(wpm)],
            RadioCommand::SendCw(_text) => vec![StatePatch::KeyerSending(true)],
            RadioCommand::StopCw => vec![StatePatch::KeyerSending(false)],

            RadioCommand::Refresh => Vec::new(),
        };

        tracing::debug!(
            patch_count = patches.len(),
            is_refresh,
            "dummy driver produced patches"
        );

        if is_refresh {
            Ok(DriverCommandOutcome::manual_refresh(patches))
        } else {
            Ok(DriverCommandOutcome::command_response(patches))
        }
    }
}

fn receiver_patch(receiver: ReceiverPath, main: StatePatch, sub: StatePatch) -> Vec<StatePatch> {
    match receiver {
        ReceiverPath::Main => vec![main],
        ReceiverPath::Sub => vec![sub],
    }
}

fn dummy_state(connection: ConnectionState) -> RadioState {
    RadioState {
        connection,
        main_rx: ReceiverState {
            frequency: Some(Frequency::from_hz(14_074_000)),
            mode: Some(Mode::Usb),
            filter: ReceiverFilterState {
                bandwidth_hz: Some(2_400),
                shift_hz: Some(0),
            },
            rf: ReceiverRfState {
                preamp: Some(LeveledSetting::disabled()),
                attenuator: Some(LeveledSetting::disabled()),
                noise_blanker: Some(LeveledSetting::disabled()),
                noise_reduction: Some(LeveledSetting::disabled()),
                auto_notch: Some(false),
            },
        },
        sub_rx: Some(ReceiverState {
            frequency: Some(Frequency::from_hz(7_074_000)),
            mode: Some(Mode::Lsb),
            filter: ReceiverFilterState {
                bandwidth_hz: Some(2_400),
                shift_hz: Some(0),
            },
            rf: ReceiverRfState {
                preamp: Some(LeveledSetting::disabled()),
                attenuator: Some(LeveledSetting::disabled()),
                noise_blanker: Some(LeveledSetting::disabled()),
                noise_reduction: Some(LeveledSetting::disabled()),
                auto_notch: Some(false),
            },
        }),
        tx: Some(TransmitterState {
            frequency: Some(Frequency::from_hz(14_074_000)),
            mode: Some(Mode::Usb),
            power: Some(Power::from_watts(100)),
            transmitting: Some(false),
            split: Some(false),
        }),
        rit_xit: RitXitState {
            main_rit_enabled: Some(false),
            sub_rit_enabled: Some(false),
            xit_enabled: Some(false),
            offset_hz: Some(RitXitOffsetHz::new(0).expect("zero is a valid RIT/XIT offset")),
        },
        keyer: Some(crate::KeyerState {
            speed_wpm: Some(20),
            sending: Some(false),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Capability;

    #[test]
    fn dummy_reports_all_normalized_capabilities() {
        let caps = DummyRadioDriver::new().capabilities();

        assert_eq!(caps.main_rx.frequency, Capability::ReadWrite);
        assert_eq!(
            caps.sub_rx.unwrap().rf.noise_reduction,
            Capability::ReadWrite
        );
        assert_eq!(caps.tx.unwrap().split, Capability::ReadWrite);
        assert_eq!(caps.rit_xit.offset, Capability::ReadWrite);
        assert!(caps.keyer.unwrap().send_cw.is_supported());
    }

    #[test]
    fn dummy_receives_passthrough_options() {
        let driver = DummyRadioDriver::with_options("foo=bar,answer=42");
        assert_eq!(driver.options(), "foo=bar,answer=42");
    }
}
