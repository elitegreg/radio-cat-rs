//! Async CAT control for amateur radios.
//!
//! The default API consists of [`Radio`], [`RadioConfig`], driver descriptors
//! and capabilities, state and update types, commands, transports, and errors.
//! Protocol encoders, decoders, and profile tables are available through the
//! `advanced-protocol-api` feature.

mod actor;
mod api;
pub mod capabilities;
pub mod command;
mod driver;
mod drivers;
pub mod error;
mod frequency;
mod keyer_emulation;
pub mod mode;
#[cfg(feature = "advanced-protocol-api")]
pub mod protocol;
#[cfg(not(feature = "advanced-protocol-api"))]
#[allow(dead_code, unused_imports)]
mod protocol;
pub mod serial_ports;
pub mod state;
pub mod transport;
pub mod update;

pub use api::{supported_drivers, Radio, RadioConfig};
pub use capabilities::{
    Capability, KeyerCapabilities, RadioCapabilities, ReceiverCapabilities, ReceiverKind,
    ReceiverRfCapabilities, RitXitCapabilities, RitXitOffsetType, StateUpdateCapability,
    TransmitterCapabilities,
};
pub use command::{RadioCommand, ReceiverPath};
pub use driver::{DriverDescriptor, TransportRequirement};
pub use error::{RadioError, RangeError, Result};
pub use frequency::Frequency;
pub use mode::{Mode, ParseModeError};
pub use serial_ports::{list_serial_ports, SerialPortListEntry, SerialPortListError};
pub use state::{
    ConnectionState, KeyerState, LeveledSetting, Power, PowerUnit, RadioState, ReceiverFilterState,
    ReceiverRfState, ReceiverState, RitXitOffsetHz, RitXitState, TransmitterState,
};
pub use transport::{
    boxed_transport, open_transport, AsyncIoTransport, BoxedCatTransport, CatTransport,
    SerialTransport, TcpTransport, TransportConfig,
};
pub use update::{
    ChangeFlags, ChangeSet, SharedRadioState, StateField, StatePatch, StateReducer, StateUpdate,
    UpdateSource,
};
