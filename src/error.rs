use std::{io, num::ParseIntError, string::FromUtf8Error};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, RadioError>;

#[derive(Debug, Error)]
pub enum RadioError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("serial error: {0}")]
    Serial(#[from] tokio_serial::Error),

    #[error("connection closed while waiting for a CAT response")]
    ConnectionClosed,

    #[error("invalid UTF-8 in CAT response: {0}")]
    Utf8(#[from] FromUtf8Error),

    #[error("failed to parse integer field in `{response}`: {source}")]
    ParseInt {
        response: String,
        #[source]
        source: ParseIntError,
    },

    #[error("unexpected CAT response for {command}: `{response}`")]
    InvalidResponse {
        command: &'static str,
        response: String,
    },

    #[error("unsupported radio kind `{0}`")]
    UnknownRadio(String),

    #[error("unsupported Elecraft mode code `{0}`")]
    UnsupportedMode(u8),

    #[error("unsupported mode `{0}`")]
    InvalidMode(String),

    #[error("frequency {0} Hz is outside the supported range of 100000-54000000 Hz")]
    FrequencyOutOfRange(u64),

    #[error("CW speed {0} WPM is outside the supported range of 8-100 WPM")]
    CwSpeedOutOfRange(u16),

    #[error("CW text may be at most 60 bytes, got {0}")]
    CwTextTooLong(usize),

    #[error("CW text must be ASCII and may not contain `;`, carriage returns, or line feeds")]
    InvalidCwText,
}

impl RadioError {
    pub(crate) fn parse_int(response: &str, source: ParseIntError) -> Self {
        Self::ParseInt {
            response: response.to_string(),
            source,
        }
    }
}
