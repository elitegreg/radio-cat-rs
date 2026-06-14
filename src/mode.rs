use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Lsb,
    Usb,
    Cw,
    CwReverse,
    Am,
    Fm,
    Rtty,
    RttyReverse,
    DataLsb,
    DataUsb,
    DataFm,
    Digital,
}

impl Mode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lsb => "LSB",
            Self::Usb => "USB",
            Self::Cw => "CW",
            Self::CwReverse => "CW-R",
            Self::Am => "AM",
            Self::Fm => "FM",
            Self::Rtty => "RTTY",
            Self::RttyReverse => "RTTY-R",
            Self::DataLsb => "DATA-LSB",
            Self::DataUsb => "DATA-USB",
            Self::DataFm => "DATA-FM",
            Self::Digital => "DIGITAL",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseModeError;

impl fmt::Display for ParseModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown radio mode")
    }
}

impl std::error::Error for ParseModeError {}

impl FromStr for Mode {
    type Err = ParseModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s
            .trim()
            .to_ascii_lowercase()
            .replace('_', "-")
            .replace(' ', "-");

        match normalized.as_str() {
            "lsb" => Ok(Self::Lsb),
            "usb" => Ok(Self::Usb),
            "cw" => Ok(Self::Cw),
            "cwr" | "cw-r" | "cw-reverse" => Ok(Self::CwReverse),
            "am" => Ok(Self::Am),
            "fm" => Ok(Self::Fm),
            "rtty" => Ok(Self::Rtty),
            "rttyr" | "rtty-r" | "rtty-reverse" => Ok(Self::RttyReverse),
            "data-lsb" | "datal" | "data-l" => Ok(Self::DataLsb),
            "data-usb" | "datau" | "data-u" => Ok(Self::DataUsb),
            "data-fm" | "datafm" => Ok(Self::DataFm),
            "digital" | "dig" => Ok(Self::Digital),
            _ => Err(ParseModeError),
        }
    }
}
