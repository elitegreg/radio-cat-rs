use std::{fmt, str::FromStr};

use crate::RadioError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Mode {
    Lsb,
    Usb,
    Cw,
    Cwr,
    Fm,
    Am,
    Rtty,
    Rttyr,
    Psk,
    Pskr,
    PktLsb,
    PktUsb,
    PktFm,
    PktAm,
    LsbD1,
    UsbD1,
    LsbD2,
    UsbD2,
    LsbD3,
    UsbD3,
    Dsb,
    Sam,
    Tune,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Lsb => "LSB",
            Self::Usb => "USB",
            Self::Cw => "CW",
            Self::Cwr => "CWR",
            Self::Fm => "FM",
            Self::Am => "AM",
            Self::Rtty => "RTTY",
            Self::Rttyr => "RTTYR",
            Self::Psk => "PSK",
            Self::Pskr => "PSKR",
            Self::PktLsb => "PKTLSB",
            Self::PktUsb => "PKTUSB",
            Self::PktFm => "PKTFM",
            Self::PktAm => "PKTAM",
            Self::LsbD1 => "LSBD1",
            Self::UsbD1 => "USBD1",
            Self::LsbD2 => "LSBD2",
            Self::UsbD2 => "USBD2",
            Self::LsbD3 => "LSBD3",
            Self::UsbD3 => "USBD3",
            Self::Dsb => "DSB",
            Self::Sam => "SAM",
            Self::Tune => "TUNE",
        };

        f.write_str(value)
    }
}

impl FromStr for Mode {
    type Err = RadioError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = normalize_mode(value);

        match normalized.as_str() {
            "lsb" => Ok(Self::Lsb),
            "usb" => Ok(Self::Usb),
            "cw" => Ok(Self::Cw),
            "cwr" | "cwrev" | "cwreverse" => Ok(Self::Cwr),
            "fm" | "fmn" | "nfm" => Ok(Self::Fm),
            "am" => Ok(Self::Am),
            "rtty" => Ok(Self::Rtty),
            "rttyr" | "rttyrev" | "rttyreverse" => Ok(Self::Rttyr),
            "psk" => Ok(Self::Psk),
            "pskr" => Ok(Self::Pskr),
            "pktlsb" | "digl" => Ok(Self::PktLsb),
            "pktusb" | "digu" => Ok(Self::PktUsb),
            "pktfm" => Ok(Self::PktFm),
            "pktam" => Ok(Self::PktAm),
            "lsbd1" => Ok(Self::LsbD1),
            "usbd1" => Ok(Self::UsbD1),
            "lsbd2" => Ok(Self::LsbD2),
            "usbd2" => Ok(Self::UsbD2),
            "lsbd3" => Ok(Self::LsbD3),
            "usbd3" => Ok(Self::UsbD3),
            "dsb" => Ok(Self::Dsb),
            "sam" => Ok(Self::Sam),
            "tune" => Ok(Self::Tune),
            _ => Err(RadioError::InvalidMode(value.to_string())),
        }
    }
}

fn normalize_mode(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            !character.is_ascii_whitespace()
                && *character != '-'
                && *character != '_'
                && *character != '/'
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn parses_aliases() {
        assert_eq!("cw".parse::<Mode>().unwrap(), Mode::Cw);
        assert_eq!("cwr".parse::<Mode>().unwrap(), Mode::Cwr);
        assert_eq!("rtty-reverse".parse::<Mode>().unwrap(), Mode::Rttyr);
        assert_eq!("digu".parse::<Mode>().unwrap(), Mode::PktUsb);
        assert_eq!("lsbd2".parse::<Mode>().unwrap(), Mode::LsbD2);
        assert_eq!("sam".parse::<Mode>().unwrap(), Mode::Sam);
    }
}
