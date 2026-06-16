use async_trait::async_trait;

use crate::{
    command::RadioCommand,
    driver::{DriverCommandOutcome, DriverDescriptor, RadioDriver},
    error::{RadioError, Result},
    protocol::kenwood_ascii::{
        filter, frequency, keyer, mode, profile_by_id, rf, rit_xit, split, tx, KenwoodAsciiProfile,
        ReceiverKind,
    },
    update::UpdateSource,
    ConnectionState, KeyerState, RadioCapabilities, RadioState, ReceiverState, RitXitState,
    TransmitterState,
};

#[derive(Debug, Clone)]
pub struct KenwoodAsciiDriver {
    profile: &'static KenwoodAsciiProfile,
    options: String,
}

impl KenwoodAsciiDriver {
    pub fn new(profile: &'static KenwoodAsciiProfile, options: impl Into<String>) -> Self {
        Self {
            profile,
            options: options.into(),
        }
    }

    pub fn profile(&self) -> &'static KenwoodAsciiProfile {
        self.profile
    }

    pub fn options(&self) -> &str {
        &self.options
    }

    pub fn from_driver_id(id: &str, options: impl Into<String>) -> Option<Self> {
        profile_by_id(id).map(|profile| Self::new(profile, options))
    }

    fn encode_command(
        &self,
        command: &RadioCommand,
        current_state: &RadioState,
    ) -> Result<Option<crate::protocol::kenwood_ascii::EncodedCommand>> {
        if let Some(encoded) = frequency::encode(self.profile, command, current_state)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = mode::encode(self.profile, command, current_state)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = split::encode(self.profile, command, current_state)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = rit_xit::encode(self.profile, command, current_state)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = filter::encode(self.profile, command, current_state)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = rf::encode(self.profile, command)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = tx::encode(self.profile, command)? {
            return Ok(Some(encoded));
        }
        if let Some(encoded) = keyer::encode(self.profile, command)? {
            return Ok(Some(encoded));
        }

        Ok(None)
    }
}

#[async_trait]
impl RadioDriver for KenwoodAsciiDriver {
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
            sub_rx: match self.profile.receiver_kind {
                ReceiverKind::SingleVfo => None,
                ReceiverKind::DualVfo | ReceiverKind::DualRx => Some(ReceiverState::default()),
            },
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
            return Ok(DriverCommandOutcome::manual_refresh(Vec::new()));
        }

        let encoded = self.encode_command(&command, current_state)?.ok_or(
            RadioError::UnsupportedCapability {
                capability: "command",
            },
        )?;

        Ok(DriverCommandOutcome {
            patches: encoded.optimistic,
            source: UpdateSource::Optimistic,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::kenwood_ascii::profile_by_id, Frequency, ReceiverPath};

    #[test]
    fn driver_can_be_resolved_by_profile_id() {
        let driver = KenwoodAsciiDriver::from_driver_id("kenwood-ts590", "").unwrap();
        assert_eq!(driver.descriptor().id, "kenwood-ts590");

        assert!(KenwoodAsciiDriver::from_driver_id("unknown", "").is_none());
    }

    #[tokio::test]
    async fn driver_routes_commands_through_profile_codecs() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut driver = KenwoodAsciiDriver::new(profile, "");
        let mut state = driver.initial_state();
        state.tx = Some(TransmitterState::default());

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

        assert_eq!(outcome.source, UpdateSource::Optimistic);
        assert!(outcome
            .patches
            .contains(&crate::StatePatch::MainRxFrequency(Frequency::from_hz(
                14_074_000
            ))));
    }

    #[tokio::test]
    async fn start_marks_driver_ready() {
        let profile = profile_by_id("yaesu-ftdx10").unwrap();
        let mut driver = KenwoodAsciiDriver::new(profile, "");
        let patches = driver.start().await.unwrap();

        assert_eq!(patches.len(), 2);
        assert!(matches!(
            patches[0],
            crate::StatePatch::Connection(ConnectionState::Identifying)
        ));
        assert!(matches!(
            patches[1],
            crate::StatePatch::Connection(ConnectionState::Ready)
        ));
    }
}
