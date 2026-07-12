use std::time::Duration;

use async_trait::async_trait;

use crate::{
    error::Result, transport::CatTransport, RadioCapabilities, RadioCommand, RadioState,
    StatePatch, UpdateSource,
};

/// Transport types a driver can use for a direct connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportRequirement {
    None,
    SerialOrTcp,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DriverDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub transport_requirement: TransportRequirement,
}

impl DriverDescriptor {
    /// Capability metadata is available without opening a transport or
    /// constructing a radio connection.
    pub fn capabilities(self) -> RadioCapabilities {
        crate::drivers::capabilities_for(self.id)
            .expect("supported driver descriptors always have capabilities")
    }
}

/// The furthest protocol stage reached by a successful command.
///
/// `Written` means the command was sent without a protocol acknowledgement,
/// `Accepted` means the radio acknowledged it, and `Observed` means a decoded
/// radio response established the resulting state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandCompletion {
    Written,
    Accepted,
    Observed,
}

pub(crate) trait StateSink: Send {
    fn state(&self) -> &RadioState;
    fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource);
}

/// The complete, per-connection behavior for one supported radio.
///
/// This is intentionally crate-private. The supported extension point is the
/// built-in factory registry rather than downstream trait implementations.
#[async_trait]
pub(crate) trait RadioSession: Send {
    fn descriptor(&self) -> DriverDescriptor;
    fn capabilities(&self) -> RadioCapabilities;
    fn initial_state(&self) -> RadioState;
    fn poll_interval(&self) -> Option<Duration>;

    async fn startup(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        sink: &mut dyn StateSink,
    ) -> Result<()>;

    async fn execute(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        command: RadioCommand,
        state_before: &RadioState,
        sink: &mut dyn StateSink,
    ) -> Result<CommandCompletion>;

    async fn process_incoming(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        wait_timeout: Duration,
        default_source: UpdateSource,
        sink: &mut dyn StateSink,
    ) -> Result<bool>;

    async fn poll(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        sink: &mut dyn StateSink,
    ) -> Result<()>;
}
