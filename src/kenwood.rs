use std::{fmt, num::ParseIntError, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use tracing::debug;

use crate::{
    transport::{CatTransport, CommandIo},
    ConnectionConfig, ControllableRadio, Frequency, Mode, RadioError, Result,
};

const MAX_FREQUENCY_HZ: u64 = 99_999_999_999;
const MIN_CW_WPM: u16 = 1;
const MAX_CW_WPM: u16 = 999;
const MAX_CW_TEXT_BYTES: usize = 60;
const POST_SEND_RESPONSE_WAIT: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KenwoodProfile {
    KenwoodClassic,
    KenwoodClassicKeyer,
    KenwoodClassicMorse,
    KenwoodClassicKeyerMorse,
    KenwoodTs940,
    KenwoodTs570,
    KenwoodTs480,
    KenwoodTs480PlainCw,
    KenwoodTs480Minimal,
    KenwoodTs480SdrUno,
    KenwoodTs590,
    KenwoodTs890,
    KenwoodTs990,
    ElecraftK2,
    ElecraftK3,
    ElecraftK4,
    Ic10Derived,
    Flex6xxx,
    PowerSdrThetis,
}

impl KenwoodProfile {
    fn descriptor(self) -> &'static RadioProfile {
        match self {
            Self::KenwoodClassic => &PROFILE_KENWOOD_CLASSIC,
            Self::KenwoodClassicKeyer => &PROFILE_KENWOOD_CLASSIC_KEYER,
            Self::KenwoodClassicMorse => &PROFILE_KENWOOD_CLASSIC_MORSE,
            Self::KenwoodClassicKeyerMorse => &PROFILE_KENWOOD_CLASSIC_KEYER_MORSE,
            Self::KenwoodTs940 => &PROFILE_KENWOOD_TS940,
            Self::KenwoodTs570 => &PROFILE_KENWOOD_TS570,
            Self::KenwoodTs480 => &PROFILE_KENWOOD_TS480,
            Self::KenwoodTs480PlainCw => &PROFILE_KENWOOD_TS480_PLAIN_CW,
            Self::KenwoodTs480Minimal => &PROFILE_KENWOOD_TS480_MINIMAL,
            Self::KenwoodTs480SdrUno => &PROFILE_KENWOOD_TS480_SDRUNO,
            Self::KenwoodTs590 => &PROFILE_KENWOOD_TS590,
            Self::KenwoodTs890 => &PROFILE_KENWOOD_TS890,
            Self::KenwoodTs990 => &PROFILE_KENWOOD_TS990,
            Self::ElecraftK2 => &PROFILE_ELECRAFT_K2,
            Self::ElecraftK3 => &PROFILE_ELECRAFT_K3,
            Self::ElecraftK4 => &PROFILE_ELECRAFT_K4,
            Self::Ic10Derived => &PROFILE_IC10_DERIVED,
            Self::Flex6xxx => &PROFILE_FLEX_6XXX,
            Self::PowerSdrThetis => &PROFILE_POWER_SDR_THETIS,
        }
    }

    const fn vendor_name(self) -> &'static str {
        match self {
            Self::ElecraftK2 | Self::ElecraftK3 | Self::ElecraftK4 => "Elecraft",
            Self::Flex6xxx => "FlexRadio",
            _ => "Kenwood",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum KenwoodModel {
    Ts140s,
    Ts680s,
    Ts711,
    Ts790,
    Ts811,
    Ts690s,
    Ts50s,
    Ts930,
    Ts940s,
    Ts950s,
    Ts950Sdx,
    Ts440s,
    Ts450s,
    Ts850,
    Ts870s,
    Ts570s,
    Ts570d,
    Ts2000,
    SdrConsole,
    Ts480,
    TrUsdx,
    Qcx,
    Qdx,
    Qmx,
    Pt8000a,
    SdrUno,
    Ts590s,
    Ts590sg,
    Fx4,
    Fx4c,
    Fx4cr,
    Fx4l,
    Ts890s,
    Ts990s,
    Trc80,
    K2,
    K3,
    K3s,
    K4,
    Kx3,
    Kx2,
    Flex6xxx,
    PowerSdr,
    Thetis,
    PiHpsdr,
    UsdxHamgeek,
    Tx500,
}

impl KenwoodModel {
    pub const ALL: &'static [Self] = &[
        Self::Ts140s,
        Self::Ts680s,
        Self::Ts711,
        Self::Ts790,
        Self::Ts811,
        Self::Ts690s,
        Self::Ts50s,
        Self::Ts930,
        Self::Ts940s,
        Self::Ts950s,
        Self::Ts950Sdx,
        Self::Ts440s,
        Self::Ts450s,
        Self::Ts850,
        Self::Ts870s,
        Self::Ts570s,
        Self::Ts570d,
        Self::Ts2000,
        Self::SdrConsole,
        Self::Ts480,
        Self::TrUsdx,
        Self::Qcx,
        Self::Qdx,
        Self::Qmx,
        Self::Pt8000a,
        Self::SdrUno,
        Self::Ts590s,
        Self::Ts590sg,
        Self::Fx4,
        Self::Fx4c,
        Self::Fx4cr,
        Self::Fx4l,
        Self::Ts890s,
        Self::Ts990s,
        Self::Trc80,
        Self::K2,
        Self::K3,
        Self::K3s,
        Self::K4,
        Self::Kx3,
        Self::Kx2,
        Self::Flex6xxx,
        Self::PowerSdr,
        Self::Thetis,
        Self::PiHpsdr,
        Self::UsdxHamgeek,
        Self::Tx500,
    ];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ts140s => "ts-140s",
            Self::Ts680s => "ts-680s",
            Self::Ts711 => "ts-711",
            Self::Ts790 => "ts-790",
            Self::Ts811 => "ts-811",
            Self::Ts690s => "ts-690s",
            Self::Ts50s => "ts-50s",
            Self::Ts930 => "ts-930",
            Self::Ts940s => "ts-940s",
            Self::Ts950s => "ts-950s",
            Self::Ts950Sdx => "ts-950sdx",
            Self::Ts440s => "ts-440s",
            Self::Ts450s => "ts-450s",
            Self::Ts850 => "ts-850",
            Self::Ts870s => "ts-870s",
            Self::Ts570s => "ts-570s",
            Self::Ts570d => "ts-570d",
            Self::Ts2000 => "ts-2000",
            Self::SdrConsole => "sdrconsole",
            Self::Ts480 => "ts-480",
            Self::TrUsdx => "trusdx",
            Self::Qcx => "qcx",
            Self::Qdx => "qdx",
            Self::Qmx => "qmx",
            Self::Pt8000a => "pt-8000a",
            Self::SdrUno => "sdruno",
            Self::Ts590s => "ts-590s",
            Self::Ts590sg => "ts-590sg",
            Self::Fx4 => "fx4",
            Self::Fx4c => "fx4c",
            Self::Fx4cr => "fx4cr",
            Self::Fx4l => "fx4l",
            Self::Ts890s => "ts-890s",
            Self::Ts990s => "ts-990s",
            Self::Trc80 => "trc-80",
            Self::K2 => "k2",
            Self::K3 => "k3",
            Self::K3s => "k3s",
            Self::K4 => "k4",
            Self::Kx3 => "kx3",
            Self::Kx2 => "kx2",
            Self::Flex6xxx => "flex-6xxx (kenwood compat.)",
            Self::PowerSdr => "powersdr",
            Self::Thetis => "thetis",
            Self::PiHpsdr => "pihpsdr",
            Self::UsdxHamgeek => "usdx (hamgeek)",
            Self::Tx500 => "tx-500",
        }
    }

    pub fn display_name(self) -> String {
        match self {
            Self::SdrConsole => return "SDRConsole".to_string(),
            Self::TrUsdx => return "TruSDX".to_string(),
            Self::Qcx => return "QRPLabs QCX".to_string(),
            Self::Qdx => return "QRPLabs QDX".to_string(),
            Self::Qmx => return "QRPLabs QMX".to_string(),
            Self::Pt8000a => return "Hilberling PT-8000A".to_string(),
            Self::SdrUno => return "SDRPlay SDRUno".to_string(),
            Self::Fx4 => return "BG2FX FX-4".to_string(),
            Self::Fx4c => return "BG2FX FX-4C".to_string(),
            Self::Fx4cr => return "BG2FX FX-4CR".to_string(),
            Self::Fx4l => return "BG2FX FX-4L".to_string(),
            Self::PowerSdr => return "PowerSDR".to_string(),
            Self::Thetis => return "Thetis".to_string(),
            Self::PiHpsdr => return "PiHPSDR".to_string(),
            Self::UsdxHamgeek => return "HamGeek USDX".to_string(),
            Self::Tx500 => return "Lab599 TX-500".to_string(),
            _ => (),
        }

        format!(
            "{} {}",
            self.profile().vendor_name(),
            self.as_str().to_ascii_uppercase()
        )
    }

    pub(crate) fn from_alias(value: &str) -> Option<Self> {
        let normalized = normalize_kenwood_name(value);

        Self::all().iter().copied().find(|model| {
            let info = model.info();
            normalize_kenwood_name(info.name) == normalized
                || info
                    .aliases
                    .iter()
                    .any(|alias| normalize_kenwood_name(alias) == normalized)
        })
    }

    pub(crate) fn profile(self) -> KenwoodProfile {
        self.info().profile
    }

    fn info(self) -> KenwoodModelInfo {
        match self {
            Self::Ts140s => KenwoodModelInfo::new("ts-140s", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts680s => KenwoodModelInfo::new("ts-680s", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts711 => KenwoodModelInfo::new("ts-711", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts790 => KenwoodModelInfo::new("ts-790", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts811 => KenwoodModelInfo::new("ts-811", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts690s => KenwoodModelInfo::new(
                "ts-690s",
                KenwoodProfile::KenwoodClassic,
                &["kenwood", "kenwood-classic"],
            ),
            Self::Ts50s => KenwoodModelInfo::new(
                "ts-50s",
                KenwoodProfile::KenwoodClassicKeyer,
                &["kenwood-classic-keyer"],
            ),
            Self::Ts930 => {
                KenwoodModelInfo::new("ts-930", KenwoodProfile::KenwoodClassicKeyer, &[])
            }
            Self::Ts940s => {
                KenwoodModelInfo::new("ts-940s", KenwoodProfile::KenwoodTs940, &["kenwood-ts940"])
            }
            Self::Ts950s => KenwoodModelInfo::new(
                "ts-950s",
                KenwoodProfile::KenwoodClassicMorse,
                &["kenwood-classic-morse"],
            ),
            Self::Ts950Sdx => {
                KenwoodModelInfo::new("ts-950sdx", KenwoodProfile::KenwoodClassicMorse, &[])
            }
            Self::Ts440s => {
                KenwoodModelInfo::new("ts-440s", KenwoodProfile::Ic10Derived, &["ic10-derived"])
            }
            Self::Ts450s => KenwoodModelInfo::new("ts-450s", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts850 => KenwoodModelInfo::new("ts-850", KenwoodProfile::KenwoodClassic, &[]),
            Self::Ts870s => {
                KenwoodModelInfo::new("ts-870s", KenwoodProfile::KenwoodClassicMorse, &[])
            }
            Self::Ts570s => {
                KenwoodModelInfo::new("ts-570s", KenwoodProfile::KenwoodTs570, &["kenwood-ts570"])
            }
            Self::Ts570d => KenwoodModelInfo::new("ts-570d", KenwoodProfile::KenwoodTs570, &[]),
            Self::Ts2000 => KenwoodModelInfo::new(
                "ts-2000",
                KenwoodProfile::KenwoodClassicKeyerMorse,
                &["kenwood-classic-keyer-morse"],
            ),
            Self::SdrConsole => {
                KenwoodModelInfo::new("sdrconsole", KenwoodProfile::KenwoodClassic, &[])
            }
            Self::Ts480 => {
                KenwoodModelInfo::new("ts-480", KenwoodProfile::KenwoodTs480, &["kenwood-ts480"])
            }
            Self::TrUsdx => KenwoodModelInfo::new("trusdx", KenwoodProfile::KenwoodTs480, &[]),
            Self::Qcx => KenwoodModelInfo::new(
                "qcx",
                KenwoodProfile::KenwoodTs480PlainCw,
                &["kenwood-ts480-plain-cw"],
            ),
            Self::Qdx => KenwoodModelInfo::new("qdx", KenwoodProfile::KenwoodTs480PlainCw, &[]),
            Self::Qmx => KenwoodModelInfo::new(
                "qmx",
                KenwoodProfile::KenwoodTs480Minimal,
                &["kenwood-ts480-minimal"],
            ),
            Self::Pt8000a => {
                KenwoodModelInfo::new("pt-8000a", KenwoodProfile::KenwoodTs480Minimal, &[])
            }
            Self::SdrUno => KenwoodModelInfo::new(
                "sdruno",
                KenwoodProfile::KenwoodTs480SdrUno,
                &["kenwood-ts480-sdruno"],
            ),
            Self::Ts590s => {
                KenwoodModelInfo::new("ts-590s", KenwoodProfile::KenwoodTs590, &["kenwood-ts590"])
            }
            Self::Ts590sg => KenwoodModelInfo::new("ts-590sg", KenwoodProfile::KenwoodTs590, &[]),
            Self::Fx4 => KenwoodModelInfo::new("fx4", KenwoodProfile::KenwoodTs590, &[]),
            Self::Fx4c => KenwoodModelInfo::new("fx4c", KenwoodProfile::KenwoodTs590, &[]),
            Self::Fx4cr => KenwoodModelInfo::new("fx4cr", KenwoodProfile::KenwoodTs590, &[]),
            Self::Fx4l => KenwoodModelInfo::new("fx4l", KenwoodProfile::KenwoodTs590, &[]),
            Self::Ts890s => {
                KenwoodModelInfo::new("ts-890s", KenwoodProfile::KenwoodTs890, &["kenwood-ts890"])
            }
            Self::Ts990s => {
                KenwoodModelInfo::new("ts-990s", KenwoodProfile::KenwoodTs990, &["kenwood-ts990"])
            }
            Self::Trc80 => KenwoodModelInfo::new("trc-80", KenwoodProfile::KenwoodClassic, &[]),
            Self::K2 => KenwoodModelInfo::new("k2", KenwoodProfile::ElecraftK2, &["elecraft-k2"]),
            Self::K3 => KenwoodModelInfo::new("k3", KenwoodProfile::ElecraftK3, &["elecraft-k3"]),
            Self::K3s => KenwoodModelInfo::new("k3s", KenwoodProfile::ElecraftK3, &[]),
            Self::K4 => KenwoodModelInfo::new(
                "k4",
                KenwoodProfile::ElecraftK4,
                &["elecraft-k4", "elecraft"],
            ),
            Self::Kx3 => KenwoodModelInfo::new("kx3", KenwoodProfile::ElecraftK3, &[]),
            Self::Kx2 => KenwoodModelInfo::new("kx2", KenwoodProfile::ElecraftK3, &[]),
            Self::Flex6xxx => KenwoodModelInfo::new(
                "flex-6xxx (kenwood compat.)",
                KenwoodProfile::Flex6xxx,
                &["flex-6xxx", "flex", "6xxx", "flex6xxxkenwoodcompat"],
            ),
            Self::PowerSdr => KenwoodModelInfo::new(
                "powersdr",
                KenwoodProfile::PowerSdrThetis,
                &["powersdr-thetis"],
            ),
            Self::Thetis => KenwoodModelInfo::new("thetis", KenwoodProfile::PowerSdrThetis, &[]),
            Self::PiHpsdr => {
                KenwoodModelInfo::new("pihpsdr", KenwoodProfile::KenwoodClassicKeyerMorse, &[])
            }
            Self::UsdxHamgeek => KenwoodModelInfo::new(
                "usdx (hamgeek)",
                KenwoodProfile::KenwoodClassic,
                &["usdx", "hamgeek-usdx", "hamgeekusdx"],
            ),
            Self::Tx500 => {
                KenwoodModelInfo::new("tx-500", KenwoodProfile::KenwoodClassicKeyerMorse, &[])
            }
        }
    }
}

#[derive(Clone, Copy)]
struct KenwoodModelInfo {
    name: &'static str,
    profile: KenwoodProfile,
    aliases: &'static [&'static str],
}

impl KenwoodModelInfo {
    const fn new(
        name: &'static str,
        profile: KenwoodProfile,
        aliases: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            profile,
            aliases,
        }
    }
}

fn normalize_kenwood_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| {
            !character.is_ascii_whitespace()
                && *character != '-'
                && *character != '_'
                && *character != '/'
                && *character != '('
                && *character != ')'
                && *character != '.'
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

#[derive(Clone, Copy)]
struct ModeCode {
    code: &'static str,
    mode: Mode,
}

#[derive(Clone, Copy)]
enum CwSendStyle {
    Plain,
    Ky2,
    Padded24,
    RightAligned24,
}

#[derive(Clone, Copy)]
enum CwStopStyle {
    Ky0,
    K3,
    K4,
}

impl CwStopStyle {
    const fn command(self) -> &'static str {
        match self {
            Self::Ky0 => "KY0;",
            Self::K3 => "KY \u{04};",
            Self::K4 => "KY @;",
        }
    }
}

#[derive(Clone, Copy)]
struct RadioProfile {
    name: &'static str,
    frequency_get_command: &'static str,
    frequency_set_prefix: &'static str,
    frequency_response_prefix: &'static str,
    mode_get_command: &'static str,
    mode_set_prefix: &'static str,
    mode_response_prefix: &'static str,
    mode_map: &'static [ModeCode],
    keyer_supported: bool,
    cw_send_style: Option<CwSendStyle>,
    cw_stop_style: Option<CwStopStyle>,
}

const MODE_MAP_DEFAULT: [ModeCode; 19] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "4",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "5",
        mode: Mode::Am,
    },
    ModeCode {
        code: "6",
        mode: Mode::Rtty,
    },
    ModeCode {
        code: "7",
        mode: Mode::Cwr,
    },
    ModeCode {
        code: "8",
        mode: Mode::Tune,
    },
    ModeCode {
        code: "9",
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: "A",
        mode: Mode::Psk,
    },
    ModeCode {
        code: "B",
        mode: Mode::Pskr,
    },
    ModeCode {
        code: "C",
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: "D",
        mode: Mode::PktUsb,
    },
    ModeCode {
        code: "E",
        mode: Mode::PktFm,
    },
    ModeCode {
        code: "F",
        mode: Mode::PktAm,
    },
    ModeCode {
        code: "G",
        mode: Mode::LsbD2,
    },
    ModeCode {
        code: "H",
        mode: Mode::UsbD2,
    },
    ModeCode {
        code: "K",
        mode: Mode::LsbD3,
    },
    ModeCode {
        code: "L",
        mode: Mode::UsbD3,
    },
];

const MODE_MAP_TS940: [ModeCode; 5] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "4",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "5",
        mode: Mode::Am,
    },
];

const MODE_MAP_SDRUNO: [ModeCode; 19] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "4",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "5",
        mode: Mode::Am,
    },
    ModeCode {
        code: "6",
        mode: Mode::Rtty,
    },
    ModeCode {
        code: "7",
        mode: Mode::Cwr,
    },
    ModeCode {
        code: "8",
        mode: Mode::PktUsb,
    },
    ModeCode {
        code: "9",
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: "A",
        mode: Mode::Psk,
    },
    ModeCode {
        code: "B",
        mode: Mode::Pskr,
    },
    ModeCode {
        code: "C",
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: "D",
        mode: Mode::PktUsb,
    },
    ModeCode {
        code: "E",
        mode: Mode::PktFm,
    },
    ModeCode {
        code: "F",
        mode: Mode::PktAm,
    },
    ModeCode {
        code: "G",
        mode: Mode::LsbD2,
    },
    ModeCode {
        code: "H",
        mode: Mode::UsbD2,
    },
    ModeCode {
        code: "K",
        mode: Mode::LsbD3,
    },
    ModeCode {
        code: "L",
        mode: Mode::UsbD3,
    },
];

const MODE_MAP_K2: [ModeCode; 6] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "6",
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: "7",
        mode: Mode::Cwr,
    },
    ModeCode {
        code: "9",
        mode: Mode::PktUsb,
    },
];

const MODE_MAP_TS990: [ModeCode; 17] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "4",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "5",
        mode: Mode::Am,
    },
    ModeCode {
        code: "6",
        mode: Mode::Rtty,
    },
    ModeCode {
        code: "7",
        mode: Mode::Cwr,
    },
    ModeCode {
        code: "9",
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: "C",
        mode: Mode::LsbD1,
    },
    ModeCode {
        code: "D",
        mode: Mode::UsbD1,
    },
    ModeCode {
        code: "E",
        mode: Mode::PktFm,
    },
    ModeCode {
        code: "G",
        mode: Mode::LsbD2,
    },
    ModeCode {
        code: "H",
        mode: Mode::UsbD2,
    },
    ModeCode {
        code: "I",
        mode: Mode::PktFm,
    },
    ModeCode {
        code: "K",
        mode: Mode::LsbD3,
    },
    ModeCode {
        code: "L",
        mode: Mode::UsbD3,
    },
    ModeCode {
        code: "M",
        mode: Mode::PktFm,
    },
];

const MODE_MAP_FLEX_6XXX: [ModeCode; 7] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "4",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "5",
        mode: Mode::Am,
    },
    ModeCode {
        code: "6",
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: "9",
        mode: Mode::PktUsb,
    },
];

const MODE_MAP_POWER_SDR_THETIS: [ModeCode; 10] = [
    ModeCode {
        code: "0",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "1",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Dsb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cwr,
    },
    ModeCode {
        code: "4",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "5",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "6",
        mode: Mode::Am,
    },
    ModeCode {
        code: "7",
        mode: Mode::PktUsb,
    },
    ModeCode {
        code: "9",
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: "10",
        mode: Mode::Sam,
    },
];

const MODE_MAP_IC10_DERIVED: [ModeCode; 6] = [
    ModeCode {
        code: "1",
        mode: Mode::Lsb,
    },
    ModeCode {
        code: "2",
        mode: Mode::Usb,
    },
    ModeCode {
        code: "3",
        mode: Mode::Cw,
    },
    ModeCode {
        code: "4",
        mode: Mode::Fm,
    },
    ModeCode {
        code: "5",
        mode: Mode::Am,
    },
    ModeCode {
        code: "6",
        mode: Mode::Rtty,
    },
];

const COMMON_FREQUENCY_GET_COMMAND: &str = "FA;";
const COMMON_FREQUENCY_SET_PREFIX: &str = "FA";
const COMMON_FREQUENCY_RESPONSE_PREFIX: &str = "FA";
const COMMON_MODE_GET_COMMAND: &str = "MD;";
const COMMON_MODE_SET_PREFIX: &str = "MD";
const COMMON_MODE_RESPONSE_PREFIX: &str = "MD";

const PROFILE_KENWOOD_CLASSIC: RadioProfile = RadioProfile {
    name: "kenwood-classic",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: false,
    cw_send_style: None,
    cw_stop_style: None,
};

const PROFILE_KENWOOD_CLASSIC_KEYER: RadioProfile = RadioProfile {
    name: "kenwood-classic-keyer",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: None,
    cw_stop_style: None,
};

const PROFILE_KENWOOD_CLASSIC_MORSE: RadioProfile = RadioProfile {
    name: "kenwood-classic-morse",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: false,
    cw_send_style: Some(CwSendStyle::Padded24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_CLASSIC_KEYER_MORSE: RadioProfile = RadioProfile {
    name: "kenwood-classic-keyer-morse",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Padded24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS940: RadioProfile = RadioProfile {
    name: "kenwood-ts940",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_TS940,
    keyer_supported: false,
    cw_send_style: None,
    cw_stop_style: None,
};

const PROFILE_KENWOOD_TS570: RadioProfile = RadioProfile {
    name: "kenwood-ts570",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Padded24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS480: RadioProfile = RadioProfile {
    name: "kenwood-ts480",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Padded24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS480_PLAIN_CW: RadioProfile = RadioProfile {
    name: "kenwood-ts480-plain-cw",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Plain),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS480_MINIMAL: RadioProfile = RadioProfile {
    name: "kenwood-ts480-minimal",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: false,
    cw_send_style: None,
    cw_stop_style: None,
};

const PROFILE_KENWOOD_TS480_SDRUNO: RadioProfile = RadioProfile {
    name: "kenwood-ts480-sdruno",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_SDRUNO,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Padded24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS590: RadioProfile = RadioProfile {
    name: "kenwood-ts590",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::RightAligned24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS890: RadioProfile = RadioProfile {
    name: "kenwood-ts890",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Ky2),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_KENWOOD_TS990: RadioProfile = RadioProfile {
    name: "kenwood-ts990",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_TS990,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Ky2),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_ELECRAFT_K2: RadioProfile = RadioProfile {
    name: "elecraft-k2",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_K2,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Padded24),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_ELECRAFT_K3: RadioProfile = RadioProfile {
    name: "elecraft-k3",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Plain),
    cw_stop_style: Some(CwStopStyle::K3),
};

const PROFILE_ELECRAFT_K4: RadioProfile = RadioProfile {
    name: "elecraft-k4",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_DEFAULT,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Plain),
    cw_stop_style: Some(CwStopStyle::K4),
};

const PROFILE_IC10_DERIVED: RadioProfile = RadioProfile {
    name: "ic10-derived",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_IC10_DERIVED,
    keyer_supported: false,
    cw_send_style: None,
    cw_stop_style: None,
};

const PROFILE_FLEX_6XXX: RadioProfile = RadioProfile {
    name: "flex-6xxx",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_FLEX_6XXX,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Plain),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

const PROFILE_POWER_SDR_THETIS: RadioProfile = RadioProfile {
    name: "powersdr-thetis",
    frequency_get_command: COMMON_FREQUENCY_GET_COMMAND,
    frequency_set_prefix: COMMON_FREQUENCY_SET_PREFIX,
    frequency_response_prefix: COMMON_FREQUENCY_RESPONSE_PREFIX,
    mode_get_command: COMMON_MODE_GET_COMMAND,
    mode_set_prefix: COMMON_MODE_SET_PREFIX,
    mode_response_prefix: COMMON_MODE_RESPONSE_PREFIX,
    mode_map: &MODE_MAP_POWER_SDR_THETIS,
    keyer_supported: true,
    cw_send_style: Some(CwSendStyle::Plain),
    cw_stop_style: Some(CwStopStyle::Ky0),
};

#[derive(Clone)]
pub struct KenwoodRadio {
    io: Arc<dyn CommandIo>,
    profile: KenwoodProfile,
}

impl fmt::Debug for KenwoodRadio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KenwoodRadio")
            .field("profile", &self.profile.descriptor().name)
            .finish_non_exhaustive()
    }
}

impl KenwoodRadio {
    pub(crate) async fn connect(
        connection: ConnectionConfig,
        profile: KenwoodProfile,
    ) -> Result<Self> {
        debug!(
            ?connection,
            profile = profile.descriptor().name,
            "connecting radio"
        );
        let transport = CatTransport::open(&connection).await?;
        let io: Arc<dyn CommandIo> = Arc::new(transport);

        Ok(Self { io, profile })
    }

    #[cfg(test)]
    fn from_io(io: Arc<dyn CommandIo>, profile: KenwoodProfile) -> Self {
        Self { io, profile }
    }

    fn descriptor(&self) -> &'static RadioProfile {
        self.profile.descriptor()
    }

    fn parse_numeric_response<T>(response: &str, prefix: &'static str) -> Result<T>
    where
        T: FromStr<Err = ParseIntError>,
    {
        let body = Self::response_body(response, prefix)?;
        body.parse()
            .map_err(|source| RadioError::parse_int(response, source))
    }

    fn parse_frequency_response(&self, response: &str) -> Result<Frequency> {
        let frequency_hz = Self::parse_numeric_response::<u64>(
            response,
            self.descriptor().frequency_response_prefix,
        )?;
        Ok(Frequency::from_hz(frequency_hz))
    }

    fn parse_mode_response(&self, response: &str) -> Result<Mode> {
        let code = Self::response_body(response, self.descriptor().mode_response_prefix)?;
        self.mode_from_code(code)
    }

    fn response_body<'a>(response: &'a str, prefix: &'static str) -> Result<&'a str> {
        response
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(';'))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RadioError::InvalidResponse {
                command: prefix,
                response: response.to_string(),
            })
    }

    fn format_frequency_set(&self, frequency: Frequency) -> Result<String> {
        let frequency_hz = frequency.hz();

        if !(1..=MAX_FREQUENCY_HZ).contains(&frequency_hz) {
            return Err(RadioError::FrequencyOutOfRange(frequency_hz));
        }

        Ok(format!(
            "{}{frequency_hz:011};",
            self.descriptor().frequency_set_prefix
        ))
    }

    fn format_mode_set(&self, mode: Mode) -> Result<String> {
        let Some(code) = self.code_from_mode(mode) else {
            return Err(RadioError::UnsupportedModeForRadio {
                mode: mode.to_string(),
                radio: self.descriptor().name,
            });
        };

        Ok(format!("{}{code};", self.descriptor().mode_set_prefix))
    }

    fn format_cw_speed_set(&self, wpm: u16) -> Result<String> {
        if !(MIN_CW_WPM..=MAX_CW_WPM).contains(&wpm) {
            return Err(RadioError::CwSpeedOutOfRange(wpm));
        }

        Ok(format!("KS{wpm:03};"))
    }

    fn format_cw_text(&self, text: &str) -> Result<Vec<String>> {
        if text.is_empty()
            || !text.is_ascii()
            || text.contains(';')
            || text.contains('\r')
            || text.contains('\n')
        {
            return Err(RadioError::InvalidCwText);
        }

        if text.len() > MAX_CW_TEXT_BYTES {
            return Err(RadioError::CwTextTooLong(text.len()));
        }

        let Some(style) = self.descriptor().cw_send_style else {
            return Err(RadioError::UnsupportedOperation {
                operation: "send-cw",
                radio: self.descriptor().name,
            });
        };

        Ok(match style {
            CwSendStyle::Plain => vec![format!("KY {text};")],
            CwSendStyle::Ky2 => vec![format!("KY2{text};")],
            CwSendStyle::Padded24 => text
                .as_bytes()
                .chunks(24)
                .map(|chunk| {
                    let chunk = std::str::from_utf8(chunk).expect("ASCII text must be valid UTF-8");
                    format!("KY {chunk:<24};")
                })
                .collect(),
            CwSendStyle::RightAligned24 => text
                .as_bytes()
                .chunks(24)
                .map(|chunk| {
                    let chunk = std::str::from_utf8(chunk).expect("ASCII text must be valid UTF-8");
                    format!("KY {chunk:>24};")
                })
                .collect(),
        })
    }

    fn mode_from_code(&self, code: &str) -> Result<Mode> {
        let code = normalize_mode_code(code);

        self.descriptor()
            .mode_map
            .iter()
            .find(|mapping| normalize_mode_code(mapping.code) == code)
            .map(|mapping| mapping.mode)
            .ok_or_else(|| RadioError::UnsupportedModeCode(code))
    }

    fn code_from_mode(&self, mode: Mode) -> Option<&'static str> {
        self.descriptor()
            .mode_map
            .iter()
            .find(|mapping| mapping.mode == mode)
            .map(|mapping| mapping.code)
    }

    fn check_keyer_support(&self, operation: &'static str) -> Result<()> {
        if self.descriptor().keyer_supported {
            Ok(())
        } else {
            Err(RadioError::UnsupportedOperation {
                operation,
                radio: self.descriptor().name,
            })
        }
    }

    async fn send_command(&self, command: &str) -> Result<()> {
        let response = self
            .io
            .send_with_optional_response(command, POST_SEND_RESPONSE_WAIT)
            .await?;

        if let Some(response) = response {
            if response.contains('?') {
                return Err(RadioError::CommandRejected {
                    command: command.to_string(),
                    response,
                });
            }
        }

        Ok(())
    }
}

fn normalize_mode_code(code: &str) -> String {
    let code = code.trim().to_ascii_uppercase();

    if code.chars().all(|character| character.is_ascii_digit()) {
        let stripped = code.trim_start_matches('0');
        if stripped.is_empty() {
            "0".to_string()
        } else {
            stripped.to_string()
        }
    } else {
        code
    }
}

#[async_trait]
impl ControllableRadio for KenwoodRadio {
    async fn get_frequency(&self) -> Result<Frequency> {
        let response = self
            .io
            .query(self.descriptor().frequency_get_command)
            .await?;
        self.parse_frequency_response(&response)
    }

    async fn set_frequency(&self, frequency: Frequency) -> Result<()> {
        let command = self.format_frequency_set(frequency)?;
        self.send_command(&command).await
    }

    async fn get_mode(&self) -> Result<Mode> {
        let response = self.io.query(self.descriptor().mode_get_command).await?;
        self.parse_mode_response(&response)
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        let command = self.format_mode_set(mode)?;
        self.send_command(&command).await
    }

    async fn send_cw(&self, text: &str) -> Result<()> {
        let commands = self.format_cw_text(text)?;
        for command in commands {
            self.send_command(&command).await?;
        }

        Ok(())
    }

    async fn stop_cw(&self) -> Result<()> {
        let Some(stop_style) = self.descriptor().cw_stop_style else {
            return Err(RadioError::UnsupportedOperation {
                operation: "stop-cw",
                radio: self.descriptor().name,
            });
        };

        self.send_command(stop_style.command()).await
    }

    async fn get_cw_wpm(&self) -> Result<u16> {
        self.check_keyer_support("get-cw-wpm")?;
        let response = self.io.query("KS;").await?;
        Self::parse_numeric_response(&response, "KS")
    }

    async fn set_cw_wpm(&self, wpm: u16) -> Result<()> {
        self.check_keyer_support("set-cw-wpm")?;
        let command = self.format_cw_speed_set(wpm)?;
        self.send_command(&command).await
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use tokio::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MockIo {
        sent: Mutex<Vec<String>>,
        responses: Mutex<VecDeque<(String, String)>>,
        send_responses: Mutex<VecDeque<String>>,
    }

    impl MockIo {
        async fn push_query(&self, command: &str, response: &str) {
            self.responses
                .lock()
                .await
                .push_back((command.to_string(), response.to_string()));
        }

        async fn push_send_response(&self, response: &str) {
            self.send_responses
                .lock()
                .await
                .push_back(response.to_string());
        }

        async fn sent_commands(&self) -> Vec<String> {
            self.sent.lock().await.clone()
        }
    }

    #[async_trait]
    impl CommandIo for MockIo {
        async fn send(&self, command: &str) -> Result<()> {
            self.sent.lock().await.push(command.to_string());
            Ok(())
        }

        async fn send_with_optional_response(
            &self,
            command: &str,
            _response_wait: Duration,
        ) -> Result<Option<String>> {
            self.sent.lock().await.push(command.to_string());
            Ok(self.send_responses.lock().await.pop_front())
        }

        async fn query(&self, command: &str) -> Result<String> {
            self.sent.lock().await.push(command.to_string());

            let (expected_command, response) = self
                .responses
                .lock()
                .await
                .pop_front()
                .expect("expected queued response");

            assert_eq!(expected_command, command);

            Ok(response)
        }
    }

    #[test]
    fn parses_ts990_extended_modes() {
        let io = Arc::new(MockIo::default());
        let radio = KenwoodRadio::from_io(io, KenwoodProfile::KenwoodTs990);

        let mode = radio.parse_mode_response("MDD;").unwrap();
        assert_eq!(mode, Mode::UsbD1);
    }

    #[test]
    fn rejects_mode_not_in_profile_map() {
        let io = Arc::new(MockIo::default());
        let radio = KenwoodRadio::from_io(io, KenwoodProfile::KenwoodTs940);

        let error = radio.format_mode_set(Mode::Rtty).unwrap_err();
        assert!(matches!(error, RadioError::UnsupportedModeForRadio { .. }));
    }

    #[test]
    fn formats_k4_stop_command() {
        assert_eq!(CwStopStyle::K4.command(), "KY @;");
    }

    #[tokio::test]
    async fn uses_expected_commands_for_ts590_profile() {
        let io = Arc::new(MockIo::default());
        io.push_query("FA;", "FA00014074000;").await;
        io.push_query("MD;", "MD2;").await;
        io.push_query("KS;", "KS018;").await;

        let radio = KenwoodRadio::from_io(io.clone(), KenwoodProfile::KenwoodTs590);

        assert_eq!(
            radio.get_frequency().await.unwrap(),
            Frequency::from_hz(14_074_000)
        );
        assert_eq!(radio.get_mode().await.unwrap(), Mode::Usb);
        assert_eq!(radio.get_cw_wpm().await.unwrap(), 18);

        radio
            .set_frequency(Frequency::from_hz(7_050_000))
            .await
            .unwrap();
        radio.set_mode(Mode::Rtty).await.unwrap();
        radio.set_cw_wpm(20).await.unwrap();
        radio.send_cw("CQ TEST").await.unwrap();
        radio.stop_cw().await.unwrap();

        assert_eq!(
            io.sent_commands().await,
            vec![
                "FA;",
                "MD;",
                "KS;",
                "FA00007050000;",
                "MD6;",
                "KS020;",
                "KY                  CQ TEST;",
                "KY0;",
            ]
        );
    }

    #[tokio::test]
    async fn reports_unsupported_keyer_on_classic_profile() {
        let io = Arc::new(MockIo::default());
        let radio = KenwoodRadio::from_io(io, KenwoodProfile::KenwoodClassic);

        let error = radio.get_cw_wpm().await.unwrap_err();
        assert!(matches!(
            error,
            RadioError::UnsupportedOperation {
                operation: "get-cw-wpm",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn fails_send_when_optional_response_contains_question_mark() {
        let io = Arc::new(MockIo::default());
        io.push_send_response("?;").await;
        let radio = KenwoodRadio::from_io(io, KenwoodProfile::KenwoodTs590);

        let error = radio.set_mode(Mode::Usb).await.unwrap_err();

        assert!(matches!(
            error,
            RadioError::CommandRejected {
                command,
                response,
            } if command == "MD2;" && response == "?;"
        ));
    }
}
