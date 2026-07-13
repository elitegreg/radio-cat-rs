use std::io;

use thiserror::Error;

pub type Result<T, E = RadioError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum RadioError {
    #[error("unsupported radio driver: {driver}")]
    UnsupportedDriver { driver: String },

    #[error("unsupported capability: {capability}")]
    UnsupportedCapability { capability: &'static str },

    #[error("radio rejected command via {protocol}: {reason}")]
    CommandRejected {
        protocol: &'static str,
        reason: &'static str,
    },

    #[error("invalid value for {field}: {message}")]
    InvalidValue {
        field: &'static str,
        message: String,
    },

    #[error("protocol syntax error{command_suffix}")]
    ProtocolSyntax { command_suffix: String },

    #[error("protocol communication error")]
    ProtocolCommunication,

    #[error("protocol busy")]
    ProtocolBusy,

    #[error("protocol decode error for {command}: {message}")]
    Decode {
        command: &'static str,
        message: String,
    },

    #[error("protocol timeout while waiting for {command}")]
    Timeout { command: &'static str },

    #[error("radio task has stopped")]
    TaskStopped,

    #[error("radio command response channel was canceled")]
    CommandCanceled,

    /// A transport failure with no typed source (kept for caller-created errors).
    #[error("transport error: {0}")]
    Transport(String),

    #[error("I/O transport error")]
    Io {
        #[source]
        source: io::Error,
    },

    #[error("serial transport error")]
    Serial {
        #[source]
        source: tokio_serial::Error,
    },
}

impl From<std::io::Error> for RadioError {
    fn from(value: std::io::Error) -> Self {
        Self::Io { source: value }
    }
}

impl From<tokio_serial::Error> for RadioError {
    fn from(value: tokio_serial::Error) -> Self {
        Self::Serial { source: value }
    }
}

impl RadioError {
    pub fn is_transport(&self) -> bool {
        matches!(
            self,
            Self::Transport(_) | Self::Io { .. } | Self::Serial { .. }
        )
    }

    pub fn protocol_syntax(command: Option<&str>) -> Self {
        let command_suffix = match command {
            Some(command) => format!(" for {command}"),
            None => String::new(),
        };

        Self::ProtocolSyntax { command_suffix }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn io_errors_retain_their_source_and_kind() {
        let error = RadioError::from(io::Error::from(io::ErrorKind::TimedOut));
        assert!(error.is_transport());
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .map(io::Error::kind),
            Some(io::ErrorKind::TimedOut)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("value {value} is outside supported range {min}..={max}")]
pub struct RangeError {
    pub value: i16,
    pub min: i16,
    pub max: i16,
}
