use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};

use crate::{
    actor::{send_command, CommandEnvelope, RadioTask},
    command::{RadioCommand, ReceiverPath},
    driver::RadioSession,
    drivers,
    error::Result,
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
        let session =
            drivers::create_session(&config.driver, &config.options, &config.transport, false)?;

        tracing::debug!(
            driver = %config.driver,
            transport = ?config.transport,
            "opening transport for radio"
        );
        let transport = open_transport(&config.transport).await?;
        Self::build_from_session(config, session, transport)
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
        let session =
            drivers::create_session(&config.driver, &config.options, &config.transport, true)?;
        Self::build_from_session(config, session, Some(boxed_transport(transport)))
    }

    fn build_from_session(
        config: RadioConfig,
        session: Box<dyn RadioSession>,
        transport: Option<BoxedCatTransport>,
    ) -> Result<(Self, RadioTask)> {
        tracing::debug!(
            driver = %config.driver,
            options = %config.options,
            has_transport = transport.is_some(),
            command_channel_capacity = config.command_channel_capacity,
            update_channel_capacity = config.update_channel_capacity,
            "building radio internals"
        );

        let descriptor = session.descriptor();
        let capabilities = Arc::new(session.capabilities());
        let initial_state = session.initial_state();
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
            session,
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

    /// Submit a command and wait until its protocol session reaches its
    /// supported completion stage: written, acknowledged, or decoded state.
    /// A successful call never publishes a predicted state before that stage;
    /// use state updates to observe the resulting accepted radio state.
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

    pub async fn set_data_ptt(&self, transmitting: bool) -> Result<()> {
        self.command(RadioCommand::SetDataPtt(transmitting)).await
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

    pub async fn set_rit_offset(
        &self,
        receiver: ReceiverPath,
        offset: RitXitOffsetHz,
    ) -> Result<()> {
        self.command(RadioCommand::SetRitOffset { receiver, offset })
            .await
    }

    pub async fn set_main_rit_offset(&self, offset: RitXitOffsetHz) -> Result<()> {
        self.set_rit_offset(ReceiverPath::Main, offset).await
    }

    pub async fn set_sub_rit_offset(&self, offset: RitXitOffsetHz) -> Result<()> {
        self.set_rit_offset(ReceiverPath::Sub, offset).await
    }

    pub async fn set_main_xit_offset(&self, offset: RitXitOffsetHz) -> Result<()> {
        self.command(RadioCommand::SetXitOffset(offset)).await
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
    use crate::{Capability, ChangeFlags, RadioError};

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
    async fn supported_driver_list_contains_dummy_kenwood_icom_and_flex_profiles() {
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "dummy"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "kenwood-ts590"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "icom-ic705"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "icom-ic7300"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "icom-ic7100"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "icom-ic7610"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "icom-ic7760"));
        assert!(supported_drivers()
            .iter()
            .any(|driver| driver.id == "flexradio-smartsdr"));

        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        assert_eq!(
            radio.capabilities().main_rx.frequency,
            Capability::ReadWrite
        );

        let kenwood = Radio::connect(RadioConfig::new("kenwood-ts590"))
            .await
            .unwrap();
        assert_eq!(kenwood.driver_descriptor().id, "kenwood-ts590");

        let icom = Radio::connect(RadioConfig::new("icom-ic705"))
            .await
            .unwrap();
        assert_eq!(icom.driver_descriptor().id, "icom-ic705");
    }

    #[tokio::test]
    async fn flexradio_build_rejects_non_tcp_transport_configs() {
        let serial_error = match Radio::build(
            RadioConfig::new("flexradio-smartsdr").with_serial_transport("/dev/null", 38_400),
        )
        .await
        {
            Ok(_) => panic!("expected serial SmartSDR build to fail"),
            Err(error) => error,
        };
        assert!(matches!(
            serial_error,
            RadioError::InvalidValue {
                field: "transport",
                ..
            }
        ));

        let none_error = match Radio::build(RadioConfig::new("flexradio-smartsdr")).await {
            Ok(_) => panic!("expected transport-less SmartSDR build to fail"),
            Err(error) => error,
        };
        assert!(matches!(
            none_error,
            RadioError::InvalidValue {
                field: "transport",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn registry_resolution_and_option_parsing_happen_before_transport_open() {
        let unsupported =
            Radio::build(RadioConfig::new("not-a-radio").with_tcp_transport("127.0.0.1:0")).await;
        assert!(matches!(
            unsupported,
            Err(RadioError::UnsupportedDriver { .. })
        ));

        let invalid_options = Radio::build(
            RadioConfig::new("icom-ic705")
                .with_options("poll_interval=invalid")
                .with_tcp_transport("127.0.0.1:0"),
        )
        .await;
        assert!(matches!(
            invalid_options,
            Err(RadioError::InvalidValue { .. })
        ));
    }

    #[tokio::test]
    async fn transportless_native_session_validates_and_applies_local_state() {
        let radio = Radio::connect(RadioConfig::new("ELECRAFT-K2"))
            .await
            .unwrap();
        let frequency = Frequency::from_hz(7_100_000);

        radio.set_main_frequency(frequency).await.unwrap();

        assert_eq!(radio.latest_state().main_rx.frequency, Some(frequency));
        assert!(matches!(
            radio.set_main_mode(Mode::Am).await,
            Err(RadioError::InvalidValue { field: "mode", .. })
        ));
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
