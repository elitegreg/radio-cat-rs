use async_trait::async_trait;

use crate::{
    capabilities::RadioCapabilities, command::RadioCommand, error::Result, update::StatePatch,
    RadioState, UpdateSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DriverDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct DriverCommandOutcome {
    pub patches: Vec<StatePatch>,
    pub source: UpdateSource,
}

impl DriverCommandOutcome {
    pub fn command_response(patches: impl Into<Vec<StatePatch>>) -> Self {
        Self {
            patches: patches.into(),
            source: UpdateSource::CommandResponse,
        }
    }

    pub fn manual_refresh(patches: impl Into<Vec<StatePatch>>) -> Self {
        Self {
            patches: patches.into(),
            source: UpdateSource::ManualRefresh,
        }
    }
}

#[async_trait]
pub trait RadioDriver: Send + 'static {
    fn descriptor(&self) -> DriverDescriptor;
    fn capabilities(&self) -> RadioCapabilities;
    fn initial_state(&self) -> RadioState;

    async fn start(&mut self) -> Result<Vec<StatePatch>>;
    async fn handle_command(
        &mut self,
        command: RadioCommand,
        current_state: &RadioState,
    ) -> Result<DriverCommandOutcome>;
}
