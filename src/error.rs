use thiserror::Error;

pub type Result<T, E = RadioError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum RadioError {
    #[error("unsupported radio driver: {driver}")]
    UnsupportedDriver { driver: String },

    #[error("unsupported capability: {capability}")]
    UnsupportedCapability { capability: &'static str },

    #[error("invalid value for {field}: {message}")]
    InvalidValue {
        field: &'static str,
        message: String,
    },

    #[error("radio task has stopped")]
    TaskStopped,

    #[error("radio command response channel was canceled")]
    CommandCanceled,

    #[error("transport error: {0}")]
    Transport(String),
}

impl From<std::io::Error> for RadioError {
    fn from(value: std::io::Error) -> Self {
        Self::Transport(value.to_string())
    }
}

impl From<tokio_serial::Error> for RadioError {
    fn from(value: tokio_serial::Error) -> Self {
        Self::Transport(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("value {value} is outside supported range {min}..={max}")]
pub struct RangeError {
    pub value: i16,
    pub min: i16,
    pub max: i16,
}
