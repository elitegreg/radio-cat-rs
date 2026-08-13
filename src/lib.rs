//! Async CAT control for amateur radios.
//!
//! The default API consists of [`Radio`], [`RadioConfig`], driver descriptors
//! and capabilities, state and update types, commands, transports, and errors.
//! Protocol encoders, decoders, and profile tables are available through the
//! `advanced-protocol-api` feature.

mod actor;
mod api;
/// Capability metadata and value-domain definitions for supported radios.
pub mod capabilities;
/// Commands submitted to a connected [`Radio`].
pub mod command;
mod driver;
mod drivers;
/// Errors and result aliases returned by this crate.
pub mod error;
mod frequency;
mod keyer_emulation;
/// Radio operating modes and parsing support.
pub mod mode;
#[cfg(feature = "advanced-protocol-api")]
pub mod protocol;
#[cfg(not(feature = "advanced-protocol-api"))]
#[allow(dead_code, unused_imports)]
mod protocol;
/// Serial-port discovery helpers.
pub mod serial_ports;
/// Read-only state snapshots and normalized radio values.
pub mod state;
/// Built-in and caller-provided CAT transports.
pub mod transport;
/// State update events and reduction utilities.
pub mod update;
/// Optional flrig-compatible XML-RPC control server.
#[cfg(feature = "xml-rpc")]
pub mod xml_rpc;

pub use api::{Radio, RadioConfig, supported_drivers};
pub use capabilities::{
    Capability, ControlCapability, DataPttRelationship, IndexedControl, KeyerCapabilities,
    ModeValueDomain, PowerCapability, PowerRange, PowerStep, RadioCapabilities, RadioRegion,
    ReceiverCapabilities, ReceiverKind, ReceiverRfCapabilities, RitXitCapabilities,
    RitXitOffsetType, StateUpdateCapability, SteppedValue, TransmitterCapabilities, ValueDomain,
    ValueRange,
};
pub use command::{CommandOutcome, RadioCommand, ReceiverPath};
pub use driver::{DriverDescriptor, TransportRequirement};
pub use error::{RadioError, RangeError, Result};
pub use frequency::Frequency;
pub use mode::{Mode, ParseModeError};
pub use serial_ports::{SerialPortListEntry, SerialPortListError, list_serial_ports};
pub use state::{
    ConnectionState, KeyerState, LeveledSetting, Power, RadioState, ReceiverFilterState,
    ReceiverRfState, ReceiverState, RitXitOffsetHz, RitXitState, TransmitterState,
};
pub use transport::{
    AsyncIoTransport, BoxedCatTransport, CatTransport, SerialTransport, TcpTransport,
    TransportConfig, boxed_transport, open_transport,
};
pub use update::{
    ChangeFlags, ChangeSet, SharedRadioState, StateField, StatePatch, StateReducer, StateUpdate,
    UpdateSource,
};
