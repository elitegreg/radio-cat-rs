use async_trait::async_trait;

use crate::{
    command::RadioCommand,
    driver::{DriverCommandOutcome, DriverDescriptor, RadioDriver},
    error::{RadioError, Result},
    protocol::icom_civ::{self, IcomCivOptions, IcomCivProfile},
    update::UpdateSource,
    ConnectionState, KeyerState, RadioCapabilities, RadioState, ReceiverState, RitXitState,
    TransmitterState,
};

#[derive(Debug, Clone)]
pub struct IcomCivDriver {
    profile: &'static IcomCivProfile,
    options: String,
    parsed_options: IcomCivOptions,
}

impl IcomCivDriver {
    pub fn new(profile: &'static IcomCivProfile, options: impl Into<String>) -> Result<Self> {
        let options = options.into();
        let parsed_options = IcomCivOptions::parse(profile, &options)?;
        Ok(Self {
            profile,
            options,
            parsed_options,
        })
    }

    pub fn profile(&self) -> &'static IcomCivProfile {
        self.profile
    }

    pub fn options(&self) -> &str {
        &self.options
    }

    pub fn parsed_options(&self) -> IcomCivOptions {
        self.parsed_options
    }

    pub fn from_driver_id(id: &str, options: impl Into<String>) -> Result<Option<Self>> {
        match icom_civ::profile_by_id(id) {
            Some(profile) => Self::new(profile, options).map(Some),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl RadioDriver for IcomCivDriver {
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
            sub_rx: self
                .profile
                .capabilities
                .sub_rx
                .map(|_| ReceiverState::default()),
            tx: self
                .profile
                .capabilities
                .tx
                .map(|_| TransmitterState::default()),
            rit_xit: RitXitState::default(),
            keyer: self
                .profile
                .capabilities
                .keyer
                .map(|_| KeyerState::default()),
        }
    }

    async fn start(&mut self) -> Result<Vec<crate::StatePatch>> {
        tracing::info!(
            driver = %self.profile.id(),
            radio_address = format_args!("{:02X}", self.parsed_options.radio_address),
            controller_address = format_args!("{:02X}", self.parsed_options.controller_address),
            mode_filter = self.parsed_options.mode_filter,
            poll_interval_ms = self.parsed_options.poll_interval.as_millis(),
            options = %self.options,
            "icom-civ driver start"
        );

        Ok(vec![
            crate::StatePatch::Connection(ConnectionState::Identifying),
            crate::StatePatch::Connection(ConnectionState::Ready),
        ])
    }

    async fn handle_command(
        &mut self,
        command: RadioCommand,
        current_state: &RadioState,
    ) -> Result<DriverCommandOutcome> {
        if matches!(command, RadioCommand::Refresh) {
            tracing::debug!(driver = %self.profile.id(), "icom-civ refresh command");
            return Ok(DriverCommandOutcome::manual_refresh(Vec::new()));
        }

        let encoded = icom_civ::encode(self.profile, self.parsed_options, &command, current_state)?
            .ok_or(RadioError::UnsupportedCapability {
                capability: "command",
            })?;

        tracing::debug!(
            driver = %self.profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            optimistic_patch_count = encoded.optimistic.len(),
            "encoded ICOM CI-V command"
        );

        Ok(DriverCommandOutcome {
            patches: encoded.optimistic,
            source: UpdateSource::Optimistic,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capability, Frequency, Mode, ReceiverPath, StatePatch};

    #[test]
    fn driver_can_be_resolved_by_profile_id() {
        let driver = IcomCivDriver::from_driver_id("icom-ic705", "")
            .unwrap()
            .unwrap();
        assert_eq!(driver.descriptor().id, "icom-ic705");
        assert_eq!(driver.parsed_options().radio_address, 0xa4);

        assert!(IcomCivDriver::from_driver_id("unknown", "")
            .unwrap()
            .is_none());
    }

    #[test]
    fn capabilities_mark_ic705_as_polling_radio() {
        let driver = IcomCivDriver::from_driver_id("icom-ic705", "")
            .unwrap()
            .unwrap();
        let caps = driver.capabilities();

        assert_eq!(caps.state_updates, crate::StateUpdateCapability::Polling);
        assert_eq!(caps.main_rx.mode, Capability::ReadWrite);
        assert_eq!(caps.sub_rx.unwrap().rf.preamp, Capability::Unsupported);
    }

    #[tokio::test]
    async fn driver_routes_commands_through_profile_codecs() {
        let mut driver = IcomCivDriver::from_driver_id("icom-ic705", "")
            .unwrap()
            .unwrap();
        let state = driver.initial_state();

        let outcome = driver
            .handle_command(
                RadioCommand::SetReceiverMode {
                    receiver: ReceiverPath::Main,
                    mode: Mode::DigitalVoice,
                },
                &state,
            )
            .await
            .unwrap();

        assert_eq!(outcome.source, UpdateSource::Optimistic);
        assert!(outcome
            .patches
            .contains(&StatePatch::MainRxMode(Mode::DigitalVoice)));

        let outcome = driver
            .handle_command(
                RadioCommand::SetReceiverFrequency {
                    receiver: ReceiverPath::Main,
                    frequency: Frequency::from_hz(14_074_000),
                },
                &state,
            )
            .await
            .unwrap();
        assert!(outcome
            .patches
            .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000))));
    }
}
