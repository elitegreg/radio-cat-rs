use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};

use crate::{
    actor::{send_command, CommandEnvelope, RadioTask},
    command::{RadioCommand, ReceiverPath},
    drivers,
    drivers::{DummyRadioDriver, KenwoodAsciiDriver},
    error::{RadioError, Result},
    transport::{
        boxed_transport, open_transport, BoxedCatTransport, CatTransport, TransportConfig,
    },
    update::{SharedRadioState, StateUpdate},
    DriverDescriptor, Frequency, LeveledSetting, Mode, Power, RadioCapabilities, RitXitOffsetHz,
};

#[derive(Debug, Clone)]
pub struct RadioConfig {
    pub driver: String,
    pub transport: TransportConfig,
    pub options: String,
    pub command_channel_capacity: usize,
    pub update_channel_capacity: usize,
}

impl RadioConfig {
    pub fn new(driver: impl Into<String>) -> Self {
        Self {
            driver: driver.into(),
            transport: TransportConfig::None,
            options: String::new(),
            command_channel_capacity: 64,
            update_channel_capacity: 128,
        }
    }

    pub fn dummy() -> Self {
        Self::new("dummy")
    }

    pub fn with_transport(mut self, transport: TransportConfig) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_serial_transport(mut self, path: impl Into<String>, baud_rate: u32) -> Self {
        self.transport = TransportConfig::serial(path, baud_rate);
        self
    }

    pub fn with_tcp_transport(mut self, address: impl Into<String>) -> Self {
        self.transport = TransportConfig::tcp(address);
        self
    }

    pub fn with_tcp_socket(mut self, host: impl AsRef<str>, port: u16) -> Self {
        self.transport = TransportConfig::tcp_socket(host, port);
        self
    }

    pub fn with_options(mut self, options: impl Into<String>) -> Self {
        self.options = options.into();
        self
    }

    pub fn options(&self) -> &str {
        &self.options
    }

    pub fn with_command_channel_capacity(mut self, capacity: usize) -> Self {
        self.command_channel_capacity = capacity;
        self
    }

    pub fn with_update_channel_capacity(mut self, capacity: usize) -> Self {
        self.update_channel_capacity = capacity;
        self
    }
}

#[derive(Clone)]
pub struct Radio {
    command_tx: mpsc::Sender<CommandEnvelope>,
    state_rx: watch::Receiver<SharedRadioState>,
    update_tx: broadcast::Sender<StateUpdate>,
    capabilities: Arc<RadioCapabilities>,
    driver: DriverDescriptor,
}

impl Radio {
    pub async fn connect(config: RadioConfig) -> Result<Self> {
        tracing::info!(
            driver = %config.driver,
            transport = ?config.transport,
            "connecting radio"
        );

        let (radio, task) = Self::build(config).await?;
        let descriptor = radio.driver_descriptor();
        let task_driver = descriptor;

        tokio::spawn(async move {
            tracing::info!(driver = %task_driver.id, "radio task spawned");
            if let Err(error) = task.run().await {
                tracing::error!(driver = %task_driver.id, ?error, "radio task stopped with error");
            } else {
                tracing::info!(driver = %task_driver.id, "radio task stopped");
            }
        });

        tracing::info!(
            driver = %descriptor.id,
            display_name = %descriptor.display_name,
            "radio connected"
        );

        Ok(radio)
    }

    pub async fn build(config: RadioConfig) -> Result<(Self, RadioTask)> {
        tracing::debug!(
            driver = %config.driver,
            transport = ?config.transport,
            "opening transport for radio"
        );
        let transport = open_transport(&config.transport).await?;
        Self::build_inner(config, transport).await
    }

    pub async fn connect_with_transport<T>(config: RadioConfig, transport: T) -> Result<Self>
    where
        T: CatTransport + 'static,
    {
        tracing::info!(
            driver = %config.driver,
            "connecting radio with caller-provided transport"
        );

        let (radio, task) = Self::build_with_transport(config, transport).await?;
        let descriptor = radio.driver_descriptor();
        let task_driver = descriptor;

        tokio::spawn(async move {
            tracing::info!(driver = %task_driver.id, "radio task spawned");
            if let Err(error) = task.run().await {
                tracing::error!(driver = %task_driver.id, ?error, "radio task stopped with error");
            } else {
                tracing::info!(driver = %task_driver.id, "radio task stopped");
            }
        });

        tracing::info!(
            driver = %descriptor.id,
            display_name = %descriptor.display_name,
            "radio connected"
        );

        Ok(radio)
    }

    pub async fn build_with_transport<T>(
        config: RadioConfig,
        transport: T,
    ) -> Result<(Self, RadioTask)>
    where
        T: CatTransport + 'static,
    {
        Self::build_inner(config, Some(boxed_transport(transport))).await
    }

    async fn build_inner(
        config: RadioConfig,
        transport: Option<BoxedCatTransport>,
    ) -> Result<(Self, RadioTask)> {
        let driver_id = config.driver.trim();
        tracing::debug!(
            driver = %driver_id,
            options = %config.options,
            has_transport = transport.is_some(),
            command_channel_capacity = config.command_channel_capacity,
            update_channel_capacity = config.update_channel_capacity,
            "building radio internals"
        );

        let driver: Box<dyn crate::RadioDriver> = match driver_id.to_ascii_lowercase().as_str() {
            "dummy" => Box::new(DummyRadioDriver::with_options(config.options.clone())),
            _ => match KenwoodAsciiDriver::from_driver_id(driver_id, config.options.clone()) {
                Some(driver) => Box::new(driver),
                None => {
                    tracing::error!(driver = %config.driver, "unsupported radio driver requested");
                    return Err(RadioError::UnsupportedDriver {
                        driver: config.driver,
                    });
                }
            },
        };

        let descriptor = driver.descriptor();
        let capabilities = Arc::new(driver.capabilities());
        let initial_state = driver.initial_state();
        let initial_snapshot = Arc::new(initial_state.clone());

        let (command_tx, command_rx) = mpsc::channel(config.command_channel_capacity.max(1));
        let (state_tx, state_rx) = watch::channel(initial_snapshot);
        let (update_tx, _) = broadcast::channel(config.update_channel_capacity.max(1));

        tracing::info!(
            driver = %descriptor.id,
            display_name = %descriptor.display_name,
            "radio internals built"
        );

        let radio = Self {
            command_tx,
            state_rx,
            update_tx: update_tx.clone(),
            capabilities,
            driver: descriptor,
        };

        let task = RadioTask::new(
            driver,
            initial_state,
            command_rx,
            state_tx,
            update_tx,
            transport,
        );

        Ok((radio, task))
    }

    pub fn subscribe_state(&self) -> watch::Receiver<SharedRadioState> {
        self.state_rx.clone()
    }

    pub fn subscribe_updates(&self) -> broadcast::Receiver<StateUpdate> {
        self.update_tx.subscribe()
    }

    pub fn latest_state(&self) -> SharedRadioState {
        self.state_rx.borrow().clone()
    }

    pub fn capabilities(&self) -> &RadioCapabilities {
        &self.capabilities
    }

    pub fn capabilities_arc(&self) -> Arc<RadioCapabilities> {
        self.capabilities.clone()
    }

    pub fn driver_descriptor(&self) -> DriverDescriptor {
        self.driver
    }

    pub fn supported_drivers() -> &'static [DriverDescriptor] {
        drivers::supported_drivers()
    }

    pub async fn command(&self, command: RadioCommand) -> Result<()> {
        tracing::debug!(driver = %self.driver.id, ?command, "queueing radio command");
        let result = send_command(&self.command_tx, command).await;
        match &result {
            Ok(()) => tracing::trace!(driver = %self.driver.id, "radio command completed"),
            Err(error) => {
                tracing::debug!(driver = %self.driver.id, ?error, "radio command failed")
            }
        }
        result
    }

    pub async fn refresh(&self) -> Result<()> {
        self.command(RadioCommand::Refresh).await
    }

    pub async fn set_receiver_frequency(
        &self,
        receiver: ReceiverPath,
        frequency: Frequency,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverFrequency {
            receiver,
            frequency,
        })
        .await
    }

    pub async fn set_main_frequency(&self, frequency: Frequency) -> Result<()> {
        self.set_receiver_frequency(ReceiverPath::Main, frequency)
            .await
    }

    pub async fn set_sub_frequency(&self, frequency: Frequency) -> Result<()> {
        self.set_receiver_frequency(ReceiverPath::Sub, frequency)
            .await
    }

    pub async fn set_receiver_mode(&self, receiver: ReceiverPath, mode: Mode) -> Result<()> {
        self.command(RadioCommand::SetReceiverMode { receiver, mode })
            .await
    }

    pub async fn set_main_mode(&self, mode: Mode) -> Result<()> {
        self.set_receiver_mode(ReceiverPath::Main, mode).await
    }

    pub async fn set_sub_mode(&self, mode: Mode) -> Result<()> {
        self.set_receiver_mode(ReceiverPath::Sub, mode).await
    }

    pub async fn set_receiver_filter_bandwidth(
        &self,
        receiver: ReceiverPath,
        bandwidth_hz: u16,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverFilterBandwidth {
            receiver,
            bandwidth_hz,
        })
        .await
    }

    pub async fn set_main_filter_bandwidth(&self, bandwidth_hz: u16) -> Result<()> {
        self.set_receiver_filter_bandwidth(ReceiverPath::Main, bandwidth_hz)
            .await
    }

    pub async fn set_sub_filter_bandwidth(&self, bandwidth_hz: u16) -> Result<()> {
        self.set_receiver_filter_bandwidth(ReceiverPath::Sub, bandwidth_hz)
            .await
    }

    pub async fn set_receiver_filter_shift(
        &self,
        receiver: ReceiverPath,
        shift_hz: i16,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverFilterShift { receiver, shift_hz })
            .await
    }

    pub async fn set_main_filter_shift(&self, shift_hz: i16) -> Result<()> {
        self.set_receiver_filter_shift(ReceiverPath::Main, shift_hz)
            .await
    }

    pub async fn set_sub_filter_shift(&self, shift_hz: i16) -> Result<()> {
        self.set_receiver_filter_shift(ReceiverPath::Sub, shift_hz)
            .await
    }

    pub async fn set_receiver_preamp(
        &self,
        receiver: ReceiverPath,
        setting: LeveledSetting,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverPreamp { receiver, setting })
            .await
    }

    pub async fn set_main_preamp(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_preamp(ReceiverPath::Main, setting).await
    }

    pub async fn set_sub_preamp(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_preamp(ReceiverPath::Sub, setting).await
    }

    pub async fn set_receiver_attenuator(
        &self,
        receiver: ReceiverPath,
        setting: LeveledSetting,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverAttenuator { receiver, setting })
            .await
    }

    pub async fn set_main_attenuator(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_attenuator(ReceiverPath::Main, setting)
            .await
    }

    pub async fn set_sub_attenuator(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_attenuator(ReceiverPath::Sub, setting)
            .await
    }

    pub async fn set_receiver_noise_blanker(
        &self,
        receiver: ReceiverPath,
        setting: LeveledSetting,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverNoiseBlanker { receiver, setting })
            .await
    }

    pub async fn set_main_noise_blanker(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_noise_blanker(ReceiverPath::Main, setting)
            .await
    }

    pub async fn set_sub_noise_blanker(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_noise_blanker(ReceiverPath::Sub, setting)
            .await
    }

    pub async fn set_receiver_noise_reduction(
        &self,
        receiver: ReceiverPath,
        setting: LeveledSetting,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverNoiseReduction { receiver, setting })
            .await
    }

    pub async fn set_main_noise_reduction(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_noise_reduction(ReceiverPath::Main, setting)
            .await
    }

    pub async fn set_sub_noise_reduction(&self, setting: LeveledSetting) -> Result<()> {
        self.set_receiver_noise_reduction(ReceiverPath::Sub, setting)
            .await
    }

    pub async fn set_receiver_auto_notch(
        &self,
        receiver: ReceiverPath,
        enabled: bool,
    ) -> Result<()> {
        self.command(RadioCommand::SetReceiverAutoNotch { receiver, enabled })
            .await
    }

    pub async fn set_main_auto_notch(&self, enabled: bool) -> Result<()> {
        self.set_receiver_auto_notch(ReceiverPath::Main, enabled)
            .await
    }

    pub async fn set_sub_auto_notch(&self, enabled: bool) -> Result<()> {
        self.set_receiver_auto_notch(ReceiverPath::Sub, enabled)
            .await
    }

    pub async fn set_tx_frequency(&self, frequency: Frequency) -> Result<()> {
        self.command(RadioCommand::SetTxFrequency(frequency)).await
    }

    pub async fn set_tx_mode(&self, mode: Mode) -> Result<()> {
        self.command(RadioCommand::SetTxMode(mode)).await
    }

    pub async fn set_tx_power(&self, power: Power) -> Result<()> {
        self.command(RadioCommand::SetTxPower(power)).await
    }

    pub async fn set_ptt(&self, transmitting: bool) -> Result<()> {
        self.command(RadioCommand::SetPtt(transmitting)).await
    }

    pub async fn set_split(&self, split: bool) -> Result<()> {
        self.command(RadioCommand::SetSplit(split)).await
    }

    pub async fn set_rit_enabled(&self, receiver: ReceiverPath, enabled: bool) -> Result<()> {
        self.command(RadioCommand::SetRitEnabled { receiver, enabled })
            .await
    }

    pub async fn set_main_rit_enabled(&self, enabled: bool) -> Result<()> {
        self.set_rit_enabled(ReceiverPath::Main, enabled).await
    }

    pub async fn set_sub_rit_enabled(&self, enabled: bool) -> Result<()> {
        self.set_rit_enabled(ReceiverPath::Sub, enabled).await
    }

    pub async fn set_xit_enabled(&self, enabled: bool) -> Result<()> {
        self.command(RadioCommand::SetXitEnabled(enabled)).await
    }

    pub async fn set_rit_xit_offset(&self, offset: RitXitOffsetHz) -> Result<()> {
        self.command(RadioCommand::SetRitXitOffset(offset)).await
    }

    pub async fn set_keyer_speed(&self, wpm: u8) -> Result<()> {
        self.command(RadioCommand::SetKeyerSpeed(wpm)).await
    }

    pub async fn send_cw(&self, text: impl Into<String>) -> Result<()> {
        self.command(RadioCommand::SendCw(text.into())).await
    }

    pub async fn stop_cw(&self) -> Result<()> {
        self.command(RadioCommand::StopCw).await
    }
}

pub fn supported_drivers() -> &'static [DriverDescriptor] {
    drivers::supported_drivers()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capability, ChangeFlags};

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn radio_handle_is_send_sync() {
        assert_send_sync::<Radio>();
    }

    #[test]
    fn radio_config_supports_transport_and_options() {
        let serial = RadioConfig::new("dummy")
            .with_serial_transport("/dev/ttyUSB0", 38_400)
            .with_options("foo=bar");
        assert_eq!(serial.options(), "foo=bar");
        assert!(matches!(serial.transport, TransportConfig::Serial { .. }));

        let tcp = RadioConfig::new("dummy").with_tcp_socket("127.0.0.1", 4532);
        assert_eq!(
            tcp.transport,
            TransportConfig::Tcp {
                address: "127.0.0.1:4532".to_string()
            }
        );
    }

    #[tokio::test]
    async fn dummy_radio_updates_state_from_commands() {
        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        let mut updates = radio.subscribe_updates();

        radio
            .set_main_frequency(Frequency::from_hz(7_030_000))
            .await
            .unwrap();

        let mut saw_frequency_update = false;
        for _ in 0..4 {
            let update = tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
                .await
                .unwrap()
                .unwrap();
            if update.changes.contains(ChangeFlags::MAIN_RX_FREQ) {
                saw_frequency_update = true;
                break;
            }
        }

        assert!(saw_frequency_update);
        assert_eq!(
            radio.latest_state().main_rx.frequency,
            Some(Frequency::from_hz(7_030_000))
        );
    }

    #[tokio::test]
    async fn supported_driver_list_contains_dummy_and_kenwood_profiles() {
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "dummy"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "kenwood-ts590"));

        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        assert_eq!(
            radio.capabilities().main_rx.frequency,
            Capability::ReadWrite
        );

        let kenwood = Radio::connect(RadioConfig::new("kenwood-ts590"))
            .await
            .unwrap();
        assert_eq!(kenwood.driver_descriptor().id, "kenwood-ts590");
    }

    #[tokio::test]
    async fn dummy_can_build_with_provided_bidirectional_transport() {
        let (client_io, _other_end) = tokio::io::duplex(64);
        let transport = crate::AsyncIoTransport::new(client_io);
        let (radio, task) = Radio::build_with_transport(RadioConfig::dummy(), transport)
            .await
            .unwrap();

        tokio::spawn(async move {
            let _ = task.run().await;
        });

        radio.set_ptt(true).await.unwrap();
        assert_eq!(
            radio
                .latest_state()
                .tx
                .as_ref()
                .and_then(|tx| tx.transmitting),
            Some(true)
        );
    }
}
