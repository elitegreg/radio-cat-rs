use async_trait::async_trait;

use crate::{
    command::RadioCommand,
    driver::{DriverCommandOutcome, DriverDescriptor, RadioDriver},
    error::{RadioError, Result},
    protocol::smartsdr::{encode, profile_by_id, SmartSdrProfile},
    transport::TransportConfig,
    ConnectionState, KeyerState, RadioCapabilities, RadioState, ReceiverState, RitXitState,
    TransmitterState,
};

#[derive(Debug, Clone)]
pub struct FlexRadioSmartSdrDriver {
    profile: &'static SmartSdrProfile,
    options: String,
}

impl FlexRadioSmartSdrDriver {
    pub fn new(profile: &'static SmartSdrProfile, options: impl Into<String>) -> Self {
        Self {
            profile,
            options: options.into(),
        }
    }

    pub fn from_driver_id(id: &str, options: impl Into<String>) -> Option<Self> {
        profile_by_id(id).map(|profile| Self::new(profile, options))
    }

    pub fn validate_transport_config(config: &TransportConfig) -> Result<()> {
        match config {
            TransportConfig::Tcp { .. } => Ok(()),
            TransportConfig::Serial { .. } => Err(RadioError::InvalidValue {
                field: "transport",
                message: "flexradio-smartsdr supports TCP transport only".to_string(),
            }),
            TransportConfig::None => Err(RadioError::InvalidValue {
                field: "transport",
                message: "flexradio-smartsdr requires a TCP transport configuration".to_string(),
            }),
        }
    }
}

#[async_trait]
impl RadioDriver for FlexRadioSmartSdrDriver {
    fn descriptor(&self) -> DriverDescriptor {
        self.profile.descriptor
    }

    fn capabilities(&self) -> RadioCapabilities {
        self.profile.capabilities
    }

    fn initial_state(&self) -> RadioState {
        RadioState {
            connection: ConnectionState::Connecting,
            main_rx: ReceiverState::default(),
            sub_rx: None,
            tx: Some(TransmitterState::default()),
            rit_xit: RitXitState::default(),
            keyer: Some(KeyerState::default()),
        }
    }

    async fn start(&mut self) -> Result<Vec<crate::StatePatch>> {
        tracing::info!(
            driver = %self.profile.id(),
            slice = self.profile.slice,
            options = %self.options,
            "flexradio smartsdr driver start"
        );

        Ok(vec![crate::StatePatch::Connection(
            ConnectionState::Identifying,
        )])
    }

    async fn handle_command(
        &mut self,
        command: RadioCommand,
        current_state: &RadioState,
    ) -> Result<DriverCommandOutcome> {
        if matches!(command, RadioCommand::Refresh) {
            return Ok(DriverCommandOutcome::manual_refresh(Vec::new()));
        }

        let encoded = encode(self.profile, &command, current_state)?.ok_or(
            RadioError::UnsupportedCapability {
                capability: "command",
            },
        )?;

        tracing::debug!(
            driver = %self.profile.id(),
            ?command,
            command_count = encoded.commands.len(),
            optimistic_patch_count = encoded.optimistic.len(),
            "encoded SmartSDR command"
        );

        Ok(DriverCommandOutcome::command_response(encoded.optimistic))
    }
}
