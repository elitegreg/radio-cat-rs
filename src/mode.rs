use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Lsb,
    Usb,
    Cw,
    CwReverse,
    Am,
    Fm,
    Wfm,
    Rtty,
    RttyReverse,
    Psk,
    PskReverse,
    DataLsb,
    DataUsb,
    DataFm,
    DataAm,
    DigitalVoice,
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
            Self::Wfm => "WFM",
            Self::Rtty => "RTTY",
            Self::RttyReverse => "RTTY-R",
            Self::Psk => "PSK",
            Self::PskReverse => "PSK-R",
            Self::DataLsb => "DATA-LSB",
            Self::DataUsb => "DATA-USB",
            Self::DataFm => "DATA-FM",
            Self::DataAm => "DATA-AM",
            Self::DigitalVoice => "DIGITAL-VOICE",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseModeError {
    input: String,
}

impl ParseModeError {
    /// The input that could not be parsed.
    pub fn input(&self) -> &str {
        &self.input
    }
}

impl fmt::Display for ParseModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown radio mode: {:?}", self.input)
    }
}

impl std::error::Error for ParseModeError {}

impl FromStr for Mode {
    type Err = ParseModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase().replace(['_', ' '], "-");

        match normalized.as_str() {
            "lsb" => Ok(Self::Lsb),
            "usb" => Ok(Self::Usb),
            "cw" => Ok(Self::Cw),
            "cwr" | "cw-r" | "cw-reverse" => Ok(Self::CwReverse),
            "am" => Ok(Self::Am),
            "fm" => Ok(Self::Fm),
            "wfm" | "wide-fm" => Ok(Self::Wfm),
            "rtty" => Ok(Self::Rtty),
            "rttyr" | "rtty-r" | "rtty-reverse" => Ok(Self::RttyReverse),
            "psk" => Ok(Self::Psk),
            "pskr" | "psk-r" | "psk-reverse" => Ok(Self::PskReverse),
            "data-lsb" | "datal" | "data-l" => Ok(Self::DataLsb),
            "data-usb" | "datau" | "data-u" => Ok(Self::DataUsb),
            "data-fm" | "datafm" => Ok(Self::DataFm),
            "data-am" | "dataam" => Ok(Self::DataAm),
            "digital-voice" | "digitalvoice" | "dv" | "d-star" | "dstar" => Ok(Self::DigitalVoice),
            _ => Err(ParseModeError {
                input: s.trim().to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Mode;
    use std::str::FromStr;

    #[test]
    fn parse_error_retains_unknown_input() {
        let error = Mode::from_str("  mystery-mode ").unwrap_err();
        assert_eq!(error.input(), "mystery-mode");
        assert_eq!(error.to_string(), "unknown radio mode: \"mystery-mode\"");
    }
}
