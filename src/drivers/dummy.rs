use std::time::Duration;

use async_trait::async_trait;

use crate::{
    command::{RadioCommand, ReceiverPath},
    driver::{CommandCompletion, DriverDescriptor, RadioSession, StateSink, TransportRequirement},
    error::Result,
    transport::CatTransport,
    update::StatePatch,
    ConnectionState, Frequency, LeveledSetting, Mode, Power, RadioCapabilities, RadioState,
    ReceiverFilterState, ReceiverRfState, ReceiverState, RitXitOffsetHz, RitXitState,
    TransmitterState, UpdateSource,
};

pub(crate) const DUMMY_DRIVER: DriverDescriptor = DriverDescriptor {
    id: "dummy",
    display_name: "Dummy Radio",
    description: "In-memory radio driver that exposes every normalized v1 capability.",
    transport_requirement: TransportRequirement::None,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct DummyRadioSession {
    options: String,
}

impl DummyRadioSession {
    pub(crate) fn with_options(options: impl Into<String>) -> Self {
        Self {
            options: options.into(),
        }
    }
}

#[async_trait]
impl RadioSession for DummyRadioSession {
    fn descriptor(&self) -> DriverDescriptor {
        DUMMY_DRIVER
    }

    fn capabilities(&self) -> RadioCapabilities {
        RadioCapabilities::dummy_all()
    }

    fn initial_state(&self) -> RadioState {
        dummy_state(ConnectionState::Connecting)
    }

    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    async fn startup(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        sink: &mut dyn StateSink,
    ) -> Result<()> {
        tracing::info!(options = %self.options, "dummy session start");
        sink.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Ready)],
            UpdateSource::Native,
        );
        Ok(())
    }

    async fn refresh(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        sink: &mut dyn StateSink,
    ) -> Result<()> {
        sink.publish_patches(Vec::new(), UpdateSource::ManualRefresh);
        Ok(())
    }

    async fn execute(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        command: RadioCommand,
        _state_before: &RadioState,
        sink: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        tracing::debug!(?command, "dummy session handling command");
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
            RadioCommand::SetPtt(transmitting) | RadioCommand::SetDataPtt(transmitting) => {
                vec![StatePatch::Transmitting(transmitting)]
            }
            RadioCommand::SetSplit(enabled) => vec![StatePatch::Split(enabled)],
            RadioCommand::SetRitEnabled { receiver, enabled } => receiver_patch(
                receiver,
                StatePatch::MainRitEnabled(enabled),
                StatePatch::SubRitEnabled(enabled),
            ),
            RadioCommand::SetXitEnabled(enabled) => vec![StatePatch::XitEnabled(enabled)],
            RadioCommand::SetRitXitOffset(offset) => vec![StatePatch::RitXitOffset(offset)],
            RadioCommand::SetRitOffset { receiver, offset } => receiver_patch(
                receiver,
                StatePatch::RitOffset(offset),
                StatePatch::SubRitOffset(offset),
            ),
            RadioCommand::SetXitOffset(offset) => vec![StatePatch::XitOffset(offset)],
            RadioCommand::SetKeyerSpeed(wpm) => vec![StatePatch::KeyerSpeed(wpm)],
            RadioCommand::SendCw(_) => vec![StatePatch::KeyerSending(true)],
            RadioCommand::StopCw => vec![StatePatch::KeyerSending(false)],
            RadioCommand::Refresh => {
                unreachable!("refresh is dispatched through RadioSession::refresh")
            }
        };
        sink.publish_patches(patches, UpdateSource::CommandResponse);
        Ok(CommandCompletion::Accepted)
    }

    async fn process_incoming(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        _wait_timeout: Duration,
        _default_source: UpdateSource,
        _sink: &mut dyn StateSink,
    ) -> Result<bool> {
        Ok(false)
    }

    async fn poll_one(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        _sink: &mut dyn StateSink,
    ) -> Result<bool> {
        Ok(true)
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
            sub_xit_enabled: Some(false),
            offset_hz: Some(RitXitOffsetHz::new(0).expect("zero is valid")),
            xit_offset_hz: Some(RitXitOffsetHz::new(0).expect("zero is valid")),
            sub_offset_hz: Some(RitXitOffsetHz::new(0).expect("zero is valid")),
            sub_xit_offset_hz: Some(RitXitOffsetHz::new(0).expect("zero is valid")),
        },
        keyer: Some(crate::KeyerState {
            speed_wpm: Some(20),
            sending: Some(false),
        }),
    }
}
