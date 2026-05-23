use std::{fmt, str::FromStr};

use crate::RadioError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Cw,
    Usb,
    Lsb,
    Fm,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Cw => "CW",
            Self::Usb => "USB",
            Self::Lsb => "LSB",
            Self::Fm => "FM",
        };

        f.write_str(value)
    }
}

impl FromStr for Mode {
    type Err = RadioError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cw" => Ok(Self::Cw),
            "usb" => Ok(Self::Usb),
            "lsb" => Ok(Self::Lsb),
            "fm" => Ok(Self::Fm),
            _ => Err(RadioError::InvalidMode(value.to_string())),
        }
    }
}
