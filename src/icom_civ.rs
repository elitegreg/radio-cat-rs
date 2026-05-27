use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tokio_serial::SerialPortBuilderExt;
use tracing::{debug, trace};

use crate::{
    options::RadioOptions, ConnectionConfig, ControllableRadio, Frequency, Mode, RadioError, Result,
};

const CIV_PREAMBLE: u8 = 0xFE;
const CIV_END: u8 = 0xFD;
const CIV_ACK: u8 = 0xFB;
const CIV_NAK: u8 = 0xFA;
const CIV_COLLISION: u8 = 0xFC;

const CIV_DEFAULT_CONTROLLER_ADDR: u8 = 0xE0;
const CIV_DEFAULT_RETRY_MAX: u8 = 3;
const CIV_DEFAULT_RETRY_BACKOFF_MS: u64 = 25;

// NOTE: We could not source-verify every model's default CI-V address from authoritative
// manufacturer manuals in this repository. For models that use UNKNOWN_CIV_ADDR, treat it as a
// placeholder and override at runtime via options:
//   civ.rig_addr=0xNN,civ.controller_addr=0xE0
const UNKNOWN_CIV_ADDR: u8 = 0x94;

const MAX_CW_TEXT_BYTES: usize = 60;
const CIV_CW_CHUNK_BYTES: usize = 30;
const MIN_CW_WPM: u16 = 1;
const MAX_CW_WPM: u16 = 255;
const MAX_FREQUENCY_HZ_5_BCD: u64 = 99_999_999_99;
const MAX_FREQUENCY_HZ_4_BCD: u64 = 99_999_999;
const KEYER_SPEED_SUBCOMMAND: u8 = 0x0C;

trait AsyncPort: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T> AsyncPort for T where T: AsyncRead + AsyncWrite + Send + Unpin {}
type BoxedPort = Box<dyn AsyncPort>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum IcomModel {
    Ic707,
    Ic725,
    Ic726,
    Ic728,
    Ic729,
    Ic735,
    Ic736,
    Ic737,
    Ic738,
    Ic751,
    Ic761,
    Ic765,
    Ic775,
    Ic781,
    Ic271,
    Ic275,
    Ic375,
    Ic471,
    Ic475,
    Ic575,
    Ic820h,
    Ic821h,
    Ic970,
    Ic1275,
    Ic706,
    Ic706Mkii,
    Ic706Mkiig,
    Ic78,
    Ic703,
    Ic718,
    Ic746,
    Ic746Pro,
    Ic756,
    Ic756Pro,
    Ic756ProIi,
    Ic756ProIii,
    Ic7000,
    Ic7200,
    Ic7410,
    Ic910,
    Ic9100,
    Ic7100,
    Ic7600,
    Ic7700,
    Ic7800,
    Ic7300,
    Ic7300Mk2,
    Ic705,
    Ic7610,
    Ic7760,
    Ic7850,
    Ic7851,
    Ic905,
    Ic9700,
    IcF8101,
    X108g,
    X6100,
    X6200,
    G90,
    X5105,
}

impl IcomModel {
    pub const ALL: &'static [Self] = &[
        Self::Ic707,
        Self::Ic725,
        Self::Ic726,
        Self::Ic728,
        Self::Ic729,
        Self::Ic735,
        Self::Ic736,
        Self::Ic737,
        Self::Ic738,
        Self::Ic751,
        Self::Ic761,
        Self::Ic765,
        Self::Ic775,
        Self::Ic781,
        Self::Ic271,
        Self::Ic275,
        Self::Ic375,
        Self::Ic471,
        Self::Ic475,
        Self::Ic575,
        Self::Ic820h,
        Self::Ic821h,
        Self::Ic970,
        Self::Ic1275,
        Self::Ic706,
        Self::Ic706Mkii,
        Self::Ic706Mkiig,
        Self::Ic78,
        Self::Ic703,
        Self::Ic718,
        Self::Ic746,
        Self::Ic746Pro,
        Self::Ic756,
        Self::Ic756Pro,
        Self::Ic756ProIi,
        Self::Ic756ProIii,
        Self::Ic7000,
        Self::Ic7200,
        Self::Ic7410,
        Self::Ic910,
        Self::Ic9100,
        Self::Ic7100,
        Self::Ic7600,
        Self::Ic7700,
        Self::Ic7800,
        Self::Ic7300,
        Self::Ic7300Mk2,
        Self::Ic705,
        Self::Ic7610,
        Self::Ic7760,
        Self::Ic7850,
        Self::Ic7851,
        Self::Ic905,
        Self::Ic9700,
        Self::IcF8101,
        Self::X108g,
        Self::X6100,
        Self::X6200,
        Self::G90,
        Self::X5105,
    ];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ic707 => "ic-707",
            Self::Ic725 => "ic-725",
            Self::Ic726 => "ic-726",
            Self::Ic728 => "ic-728",
            Self::Ic729 => "ic-729",
            Self::Ic735 => "ic-735",
            Self::Ic736 => "ic-736",
            Self::Ic737 => "ic-737",
            Self::Ic738 => "ic-738",
            Self::Ic751 => "ic-751",
            Self::Ic761 => "ic-761",
            Self::Ic765 => "ic-765",
            Self::Ic775 => "ic-775",
            Self::Ic781 => "ic-781",
            Self::Ic271 => "ic-271",
            Self::Ic275 => "ic-275",
            Self::Ic375 => "ic-375",
            Self::Ic471 => "ic-471",
            Self::Ic475 => "ic-475",
            Self::Ic575 => "ic-575",
            Self::Ic820h => "ic-820h",
            Self::Ic821h => "ic-821h",
            Self::Ic970 => "ic-970",
            Self::Ic1275 => "ic-1275",
            Self::Ic706 => "ic-706",
            Self::Ic706Mkii => "ic-706mkii",
            Self::Ic706Mkiig => "ic-706mkiig",
            Self::Ic78 => "ic-78",
            Self::Ic703 => "ic-703",
            Self::Ic718 => "ic-718",
            Self::Ic746 => "ic-746",
            Self::Ic746Pro => "ic-746pro",
            Self::Ic756 => "ic-756",
            Self::Ic756Pro => "ic-756pro",
            Self::Ic756ProIi => "ic-756proii",
            Self::Ic756ProIii => "ic-756proiii",
            Self::Ic7000 => "ic-7000",
            Self::Ic7200 => "ic-7200",
            Self::Ic7410 => "ic-7410",
            Self::Ic910 => "ic-910",
            Self::Ic9100 => "ic-9100",
            Self::Ic7100 => "ic-7100",
            Self::Ic7600 => "ic-7600",
            Self::Ic7700 => "ic-7700",
            Self::Ic7800 => "ic-7800",
            Self::Ic7300 => "ic-7300",
            Self::Ic7300Mk2 => "ic-7300mk2",
            Self::Ic705 => "ic-705",
            Self::Ic7610 => "ic-7610",
            Self::Ic7760 => "ic-7760",
            Self::Ic7850 => "ic-7850",
            Self::Ic7851 => "ic-7851",
            Self::Ic905 => "ic-905",
            Self::Ic9700 => "ic-9700",
            Self::IcF8101 => "ic-f8101",
            Self::X108g => "x108g",
            Self::X6100 => "x6100",
            Self::X6200 => "x6200",
            Self::G90 => "g90",
            Self::X5105 => "x5105",
        }
    }

    pub(crate) fn from_alias(value: &str) -> Option<Self> {
        let normalized = normalize_model_name(value);

        Self::all().iter().copied().find(|model| {
            let info = model.info();
            normalize_model_name(info.name) == normalized
                || info
                    .aliases
                    .iter()
                    .any(|alias| normalize_model_name(alias) == normalized)
        })
    }

    fn info(self) -> IcomModelInfo {
        match self {
            Self::Ic707 => {
                IcomModelInfo::new("ic-707", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic725 => {
                IcomModelInfo::new("ic-725", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic726 => {
                IcomModelInfo::new("ic-726", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic728 => {
                IcomModelInfo::new("ic-728", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic729 => {
                IcomModelInfo::new("ic-729", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic735 => {
                IcomModelInfo::new("ic-735", IcomProfile::EarlyHf731, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic736 => {
                IcomModelInfo::new("ic-736", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic737 => {
                IcomModelInfo::new("ic-737", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic738 => {
                IcomModelInfo::new("ic-738", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic751 => {
                IcomModelInfo::new("ic-751", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic761 => {
                IcomModelInfo::new("ic-761", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic765 => {
                IcomModelInfo::new("ic-765", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic775 => {
                IcomModelInfo::new("ic-775", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic781 => {
                IcomModelInfo::new("ic-781", IcomProfile::EarlyHf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic271 => {
                IcomModelInfo::new("ic-271", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic275 => {
                IcomModelInfo::new("ic-275", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic375 => {
                IcomModelInfo::new("ic-375", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic471 => {
                IcomModelInfo::new("ic-471", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic475 => {
                IcomModelInfo::new("ic-475", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic575 => {
                IcomModelInfo::new("ic-575", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic820h => IcomModelInfo::new(
                "ic-820h",
                IcomProfile::EarlyVhfUhf731,
                UNKNOWN_CIV_ADDR,
                &[],
            ),
            Self::Ic821h => IcomModelInfo::new(
                "ic-821h",
                IcomProfile::EarlyVhfUhf731,
                UNKNOWN_CIV_ADDR,
                &[],
            ),
            Self::Ic970 => {
                IcomModelInfo::new("ic-970", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic1275 => {
                IcomModelInfo::new("ic-1275", IcomProfile::EarlyVhfUhf, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic706 => {
                IcomModelInfo::new("ic-706", IcomProfile::Icom706Family, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic706Mkii => IcomModelInfo::new(
                "ic-706mkii",
                IcomProfile::Icom706Family,
                UNKNOWN_CIV_ADDR,
                &["ic706mkii"],
            ),
            Self::Ic706Mkiig => IcomModelInfo::new(
                "ic-706mkiig",
                IcomProfile::Icom706Family,
                UNKNOWN_CIV_ADDR,
                &["ic706mkiig"],
            ),
            Self::Ic78 => IcomModelInfo::new("ic-78", IcomProfile::Icom78, UNKNOWN_CIV_ADDR, &[]),
            Self::Ic703 => {
                IcomModelInfo::new("ic-703", IcomProfile::Icom703, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic718 => {
                IcomModelInfo::new("ic-718", IcomProfile::Icom718, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic746 => {
                IcomModelInfo::new("ic-746", IcomProfile::Icom746Family, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic746Pro => IcomModelInfo::new(
                "ic-746pro",
                IcomProfile::Icom746Family,
                UNKNOWN_CIV_ADDR,
                &["ic-746pro"],
            ),
            Self::Ic756 => {
                IcomModelInfo::new("ic-756", IcomProfile::Icom756Family, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic756Pro => IcomModelInfo::new(
                "ic-756pro",
                IcomProfile::Icom756Family,
                UNKNOWN_CIV_ADDR,
                &["ic-756pro"],
            ),
            Self::Ic756ProIi => IcomModelInfo::new(
                "ic-756proii",
                IcomProfile::Icom756Family,
                UNKNOWN_CIV_ADDR,
                &["ic-756proii", "ic756proii"],
            ),
            Self::Ic756ProIii => IcomModelInfo::new(
                "ic-756proiii",
                IcomProfile::Icom756Family,
                UNKNOWN_CIV_ADDR,
                &["ic-756proiii", "ic756proiii"],
            ),
            Self::Ic7000 => {
                IcomModelInfo::new("ic-7000", IcomProfile::Icom7000, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7200 => {
                IcomModelInfo::new("ic-7200", IcomProfile::Icom7200, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7410 => {
                IcomModelInfo::new("ic-7410", IcomProfile::Icom7410, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic910 => {
                IcomModelInfo::new("ic-910", IcomProfile::Icom910, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic9100 => {
                IcomModelInfo::new("ic-9100", IcomProfile::Icom9100, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7100 => IcomModelInfo::new("ic-7100", IcomProfile::Icom7100, 0x88, &[]),
            Self::Ic7600 => {
                IcomModelInfo::new("ic-7600", IcomProfile::Icom7600, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7700 => {
                IcomModelInfo::new("ic-7700", IcomProfile::Icom7700, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7800 => {
                IcomModelInfo::new("ic-7800", IcomProfile::Icom7800, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7300 => IcomModelInfo::new("ic-7300", IcomProfile::ModernDirect, 0x94, &[]),
            Self::Ic7300Mk2 => IcomModelInfo::new(
                "ic-7300mk2",
                IcomProfile::ModernDirect,
                UNKNOWN_CIV_ADDR,
                &["ic-7300mk2"],
            ),
            Self::Ic705 => IcomModelInfo::new("ic-705", IcomProfile::ModernDirect, 0xA4, &[]),
            Self::Ic7610 => IcomModelInfo::new("ic-7610", IcomProfile::ModernDirect, 0x98, &[]),
            Self::Ic7760 => {
                IcomModelInfo::new("ic-7760", IcomProfile::ModernDirect, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7850 => {
                IcomModelInfo::new("ic-7850", IcomProfile::ModernDirect, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic7851 => {
                IcomModelInfo::new("ic-7851", IcomProfile::ModernDirect, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic905 => {
                IcomModelInfo::new("ic-905", IcomProfile::ModernDirect, UNKNOWN_CIV_ADDR, &[])
            }
            Self::Ic9700 => IcomModelInfo::new("ic-9700", IcomProfile::ModernDirect, 0xA2, &[]),
            Self::IcF8101 => {
                IcomModelInfo::new("ic-f8101", IcomProfile::IcomF8101, UNKNOWN_CIV_ADDR, &[])
            }
            Self::X108g => IcomModelInfo::new(
                "x108g",
                IcomProfile::XieguX108g,
                UNKNOWN_CIV_ADDR,
                &["x-108g"],
            ),
            Self::X6100 => {
                IcomModelInfo::new("x6100", IcomProfile::XieguNewer, UNKNOWN_CIV_ADDR, &[])
            }
            Self::X6200 => {
                IcomModelInfo::new("x6200", IcomProfile::XieguNewer, UNKNOWN_CIV_ADDR, &[])
            }
            Self::G90 => IcomModelInfo::new("g90", IcomProfile::XieguNewer, UNKNOWN_CIV_ADDR, &[]),
            Self::X5105 => {
                IcomModelInfo::new("x5105", IcomProfile::XieguNewer, UNKNOWN_CIV_ADDR, &[])
            }
        }
    }
}

#[derive(Clone, Copy)]
struct IcomModelInfo {
    name: &'static str,
    profile: IcomProfile,
    default_rig_address: u8,
    aliases: &'static [&'static str],
}

impl IcomModelInfo {
    const fn new(
        name: &'static str,
        profile: IcomProfile,
        default_rig_address: u8,
        aliases: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            profile,
            default_rig_address,
            aliases,
        }
    }
}

fn normalize_model_name(value: &str) -> String {
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
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IcomProfile {
    EarlyHf,
    EarlyHf731,
    EarlyVhfUhf,
    EarlyVhfUhf731,
    Icom706Family,
    Icom78,
    Icom703,
    Icom718,
    Icom746Family,
    Icom756Family,
    Icom7000,
    Icom7200,
    Icom7410,
    Icom910,
    Icom9100,
    Icom7100,
    Icom7600,
    Icom7700,
    Icom7800,
    ModernDirect,
    IcomF8101,
    XieguX108g,
    XieguNewer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrequencyFamily {
    Bcd5,
    Old731Bcd4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModeFamily {
    Generic,
    Icom7800,
    F8101,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataOverlayFamily {
    None,
    Civ1a06,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MorseFamily {
    None,
    SendOnly,
    SendStop,
}

#[derive(Clone, Copy, Debug)]
struct ProfileDescriptor {
    name: &'static str,
    frequency_family: FrequencyFamily,
    mode_family: ModeFamily,
    data_overlay_family: DataOverlayFamily,
    keyer_supported: bool,
    morse_family: MorseFamily,
}

impl IcomProfile {
    fn descriptor(self) -> ProfileDescriptor {
        match self {
            Self::EarlyHf => ProfileDescriptor {
                name: "early-hf",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::EarlyHf731 => ProfileDescriptor {
                name: "early-hf-731",
                frequency_family: FrequencyFamily::Old731Bcd4,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::EarlyVhfUhf => ProfileDescriptor {
                name: "early-vhf-uhf",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::EarlyVhfUhf731 => ProfileDescriptor {
                name: "early-vhf-uhf-731",
                frequency_family: FrequencyFamily::Old731Bcd4,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::Icom706Family => ProfileDescriptor {
                name: "icom-706-family",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::Icom78 => ProfileDescriptor {
                name: "icom-78",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::Icom703 => ProfileDescriptor {
                name: "icom-703",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::Icom718 => ProfileDescriptor {
                name: "icom-718",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom746Family => ProfileDescriptor {
                name: "icom-746-family",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom756Family => ProfileDescriptor {
                name: "icom-756-family",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom7000 => ProfileDescriptor {
                name: "icom-7000",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom7200 => ProfileDescriptor {
                name: "icom-7200",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom7410 => ProfileDescriptor {
                name: "icom-7410",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::SendStop,
            },
            Self::Icom910 => ProfileDescriptor {
                name: "icom-910",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom9100 => ProfileDescriptor {
                name: "icom-9100",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::None,
            },
            Self::Icom7100 => ProfileDescriptor {
                name: "icom-7100",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::SendStop,
            },
            Self::Icom7600 => ProfileDescriptor {
                name: "icom-7600",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::SendOnly,
            },
            Self::Icom7700 => ProfileDescriptor {
                name: "icom-7700",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::SendStop,
            },
            Self::Icom7800 => ProfileDescriptor {
                name: "icom-7800",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Icom7800,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: true,
                morse_family: MorseFamily::SendStop,
            },
            Self::ModernDirect => ProfileDescriptor {
                name: "modern-direct",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::Civ1a06,
                keyer_supported: true,
                morse_family: MorseFamily::SendStop,
            },
            Self::IcomF8101 => ProfileDescriptor {
                name: "icom-f8101",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::F8101,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::XieguX108g => ProfileDescriptor {
                name: "xiegu-x108g",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
            Self::XieguNewer => ProfileDescriptor {
                name: "xiegu-newer-ci-v",
                frequency_family: FrequencyFamily::Bcd5,
                mode_family: ModeFamily::Generic,
                data_overlay_family: DataOverlayFamily::None,
                keyer_supported: false,
                morse_family: MorseFamily::None,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct ModeCode {
    code: u8,
    mode: Mode,
}

const MODE_MAP_GENERIC: [ModeCode; 19] = [
    ModeCode {
        code: 0x00,
        mode: Mode::Lsb,
    },
    ModeCode {
        code: 0x01,
        mode: Mode::Usb,
    },
    ModeCode {
        code: 0x02,
        mode: Mode::Am,
    },
    ModeCode {
        code: 0x03,
        mode: Mode::Cw,
    },
    ModeCode {
        code: 0x04,
        mode: Mode::Rtty,
    },
    ModeCode {
        code: 0x05,
        mode: Mode::Fm,
    },
    ModeCode {
        code: 0x06,
        mode: Mode::Wfm,
    },
    ModeCode {
        code: 0x07,
        mode: Mode::Cwr,
    },
    ModeCode {
        code: 0x08,
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: 0x11,
        mode: Mode::Ams,
    },
    ModeCode {
        code: 0x12,
        mode: Mode::Psk,
    },
    ModeCode {
        code: 0x13,
        mode: Mode::Pskr,
    },
    ModeCode {
        code: 0x16,
        mode: Mode::P25,
    },
    ModeCode {
        code: 0x17,
        mode: Mode::DStar,
    },
    ModeCode {
        code: 0x18,
        mode: Mode::Dpmr,
    },
    ModeCode {
        code: 0x19,
        mode: Mode::NxdnVn,
    },
    ModeCode {
        code: 0x20,
        mode: Mode::NxdnN,
    },
    ModeCode {
        code: 0x21,
        mode: Mode::Dcr,
    },
    ModeCode {
        code: 0x22,
        mode: Mode::Dd,
    },
];

const MODE_MAP_F8101: [ModeCode; 14] = [
    ModeCode {
        code: 0x00,
        mode: Mode::Lsb,
    },
    ModeCode {
        code: 0x01,
        mode: Mode::Usb,
    },
    ModeCode {
        code: 0x02,
        mode: Mode::Am,
    },
    ModeCode {
        code: 0x03,
        mode: Mode::Cw,
    },
    ModeCode {
        code: 0x04,
        mode: Mode::Rtty,
    },
    ModeCode {
        code: 0x05,
        mode: Mode::Fm,
    },
    ModeCode {
        code: 0x07,
        mode: Mode::Cwr,
    },
    ModeCode {
        code: 0x08,
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: 0x18,
        mode: Mode::LsbD1,
    },
    ModeCode {
        code: 0x19,
        mode: Mode::UsbD1,
    },
    ModeCode {
        code: 0x20,
        mode: Mode::LsbD2,
    },
    ModeCode {
        code: 0x21,
        mode: Mode::UsbD2,
    },
    ModeCode {
        code: 0x22,
        mode: Mode::LsbD3,
    },
    ModeCode {
        code: 0x23,
        mode: Mode::UsbD3,
    },
];

#[derive(Clone, Copy, Debug)]
struct CivAddressing {
    rig_addr: u8,
    controller_addr: u8,
}

#[derive(Clone, Copy, Debug)]
struct RetryPolicy {
    max_retries: u8,
    backoff: Duration,
}

#[derive(Clone, Debug)]
enum ExpectedResponse {
    Prefix(Vec<u8>),
    AckOrPrefix(Vec<u8>),
}

#[derive(Debug)]
struct CivFrame {
    destination: u8,
    source: u8,
    data: Vec<u8>,
}

#[async_trait]
trait CivIo: Send + Sync {
    async fn transact(
        &self,
        addressing: CivAddressing,
        command_data: &[u8],
        expected: ExpectedResponse,
        retry: RetryPolicy,
    ) -> Result<Vec<u8>>;
}

struct CivTransport {
    io: Mutex<BoxedPort>,
    timeout: Duration,
}

impl CivTransport {
    async fn open(connection: &ConnectionConfig) -> Result<Self> {
        let timeout_duration = match connection {
            ConnectionConfig::Serial { timeout, .. } | ConnectionConfig::Tcp { timeout, .. } => {
                *timeout
            }
        };

        let io: BoxedPort = match connection {
            ConnectionConfig::Serial {
                path, baud_rate, ..
            } => {
                debug!(
                    path = %path.display(),
                    baud_rate = *baud_rate,
                    timeout = ?timeout_duration,
                    "opening CI-V serial transport"
                );
                let stream = tokio_serial::new(path.to_string_lossy().into_owned(), *baud_rate)
                    .open_native_async()?;
                Box::new(stream)
            }
            ConnectionConfig::Tcp {
                host,
                port,
                timeout: connect_timeout,
                ..
            } => {
                debug!(host, port = *port, timeout = ?connect_timeout, "opening CI-V tcp transport");
                let stream = timeout(*connect_timeout, TcpStream::connect((host.as_str(), *port)))
                    .await
                    .map_err(|_| RadioError::Timeout {
                        operation: "TCP connect",
                    })??;
                Box::new(stream)
            }
        };

        Ok(Self {
            io: Mutex::new(io),
            timeout: timeout_duration,
        })
    }

    fn build_frame(addressing: CivAddressing, command_data: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(command_data.len() + 6);
        frame.push(CIV_PREAMBLE);
        frame.push(CIV_PREAMBLE);
        frame.push(addressing.rig_addr);
        frame.push(addressing.controller_addr);
        frame.extend_from_slice(command_data);
        frame.push(CIV_END);
        frame
    }

    async fn write_frame_locked<T>(
        io: &mut T,
        frame: &[u8],
        timeout_duration: Duration,
    ) -> Result<()>
    where
        T: AsyncWrite + Unpin + ?Sized,
    {
        trace!(frame = ?frame, "sending CI-V frame");
        timeout(timeout_duration, async {
            io.write_all(frame).await?;
            io.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| RadioError::Timeout {
            operation: "write frame",
        })??;

        Ok(())
    }

    async fn read_frame_locked<T>(io: &mut T, timeout_duration: Duration) -> Result<CivFrame>
    where
        T: AsyncRead + Unpin + ?Sized,
    {
        timeout(timeout_duration, async {
            let mut frame = Vec::new();
            let mut preamble_bytes = 0_u8;

            loop {
                let mut byte = [0_u8; 1];
                let read = io.read(&mut byte).await?;

                if read == 0 {
                    return Err(RadioError::ConnectionClosed);
                }

                let value = byte[0];

                if preamble_bytes < 2 {
                    if value == CIV_PREAMBLE {
                        preamble_bytes += 1;
                        frame.push(value);
                    } else {
                        preamble_bytes = 0;
                        frame.clear();
                    }
                    continue;
                }

                frame.push(value);

                if value == CIV_END {
                    break;
                }
            }

            parse_frame(&frame)
        })
        .await
        .map_err(|_| RadioError::Timeout {
            operation: "read frame",
        })?
    }
}

#[async_trait]
impl CivIo for CivTransport {
    async fn transact(
        &self,
        addressing: CivAddressing,
        command_data: &[u8],
        expected: ExpectedResponse,
        retry: RetryPolicy,
    ) -> Result<Vec<u8>> {
        let frame = Self::build_frame(addressing, command_data);

        for attempt in 0..=retry.max_retries {
            let mut io = self.io.lock().await;
            Self::write_frame_locked(&mut *io, &frame, self.timeout).await?;

            let collided = loop {
                let response = Self::read_frame_locked(&mut *io, self.timeout).await?;

                if response.destination == addressing.rig_addr
                    && response.source == addressing.controller_addr
                    && response.data == command_data
                {
                    // command echo
                    continue;
                }

                if response.data.as_slice() == [CIV_COLLISION] {
                    break true;
                }

                if response.data.as_slice() == [CIV_NAK] {
                    return Err(RadioError::CivNak);
                }

                if matches_expected(&response.data, &expected) {
                    return Ok(response.data);
                }
            };

            drop(io);

            if collided {
                if attempt == retry.max_retries {
                    return Err(RadioError::CivCollision);
                }

                sleep(retry.backoff).await;
                continue;
            }
        }

        Err(RadioError::CivCollision)
    }
}

fn matches_expected(data: &[u8], expected: &ExpectedResponse) -> bool {
    match expected {
        ExpectedResponse::Prefix(prefix) => data.starts_with(prefix),
        ExpectedResponse::AckOrPrefix(prefix) => data == [CIV_ACK] || data.starts_with(prefix),
    }
}

fn parse_frame(frame: &[u8]) -> Result<CivFrame> {
    if frame.len() < 6
        || frame[0] != CIV_PREAMBLE
        || frame[1] != CIV_PREAMBLE
        || *frame.last().unwrap_or(&0) != CIV_END
    {
        return Err(RadioError::CivProtocol("invalid frame shape".to_string()));
    }

    let destination = frame[2];
    let source = frame[3];
    let data = frame[4..frame.len() - 1].to_vec();

    if data.is_empty() {
        return Err(RadioError::CivProtocol("empty frame payload".to_string()));
    }

    Ok(CivFrame {
        destination,
        source,
        data,
    })
}

#[derive(Clone)]
pub struct IcomCivRadio {
    io: Arc<dyn CivIo>,
    model: IcomModel,
    profile: IcomProfile,
    addressing: CivAddressing,
    retry: RetryPolicy,
}

impl fmt::Debug for IcomCivRadio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IcomCivRadio")
            .field("model", &self.model.as_str())
            .field("profile", &self.profile.descriptor().name)
            .field(
                "rig_addr",
                &format_args!("0x{:02X}", self.addressing.rig_addr),
            )
            .field(
                "controller_addr",
                &format_args!("0x{:02X}", self.addressing.controller_addr),
            )
            .finish_non_exhaustive()
    }
}

impl IcomCivRadio {
    pub(crate) async fn connect(
        connection: ConnectionConfig,
        model: IcomModel,
        options: &RadioOptions,
    ) -> Result<Self> {
        let info = model.info();
        let profile = info.profile;

        let rig_addr =
            parse_u8_option(options, "civ.rig_addr")?.unwrap_or(info.default_rig_address);
        let controller_addr =
            parse_u8_option(options, "civ.controller_addr")?.unwrap_or(CIV_DEFAULT_CONTROLLER_ADDR);
        let retry_max = parse_u8_option(options, "civ.retry_max")?.unwrap_or(CIV_DEFAULT_RETRY_MAX);
        let retry_backoff_ms = parse_u64_option(options, "civ.retry_backoff_ms")?
            .unwrap_or(CIV_DEFAULT_RETRY_BACKOFF_MS);

        let transport = Arc::new(CivTransport::open(&connection).await?) as Arc<dyn CivIo>;

        debug!(
            model = info.name,
            profile = profile.descriptor().name,
            rig_addr = format_args!("0x{rig_addr:02X}"),
            controller_addr = format_args!("0x{controller_addr:02X}"),
            retry_max,
            retry_backoff_ms,
            "connected Icom CI-V radio"
        );

        Ok(Self {
            io: transport,
            model,
            profile,
            addressing: CivAddressing {
                rig_addr,
                controller_addr,
            },
            retry: RetryPolicy {
                max_retries: retry_max,
                backoff: Duration::from_millis(retry_backoff_ms),
            },
        })
    }

    #[cfg(test)]
    fn from_io(
        io: Arc<dyn CivIo>,
        model: IcomModel,
        profile: IcomProfile,
        addressing: CivAddressing,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            io,
            model,
            profile,
            addressing,
            retry,
        }
    }

    fn descriptor(&self) -> ProfileDescriptor {
        self.profile.descriptor()
    }

    fn mode_map(&self) -> &'static [ModeCode] {
        match self.descriptor().mode_family {
            ModeFamily::Generic | ModeFamily::Icom7800 => &MODE_MAP_GENERIC,
            ModeFamily::F8101 => &MODE_MAP_F8101,
        }
    }

    fn decode_mode_code(&self, code: u8) -> Result<Mode> {
        if self.descriptor().mode_family == ModeFamily::Icom7800 {
            return match code {
                0x12 => Ok(Mode::PktUsb),
                0x13 => Ok(Mode::PktLsb),
                _ => self
                    .mode_map()
                    .iter()
                    .find(|mapping| mapping.code == code)
                    .map(|mapping| mapping.mode)
                    .ok_or_else(|| RadioError::UnsupportedModeCode(format!("0x{code:02X}"))),
            };
        }

        self.mode_map()
            .iter()
            .find(|mapping| mapping.code == code)
            .map(|mapping| mapping.mode)
            .ok_or_else(|| RadioError::UnsupportedModeCode(format!("0x{code:02X}")))
    }

    fn encode_mode_code(&self, mode: Mode) -> Result<u8> {
        if self.descriptor().mode_family == ModeFamily::Icom7800 {
            return match mode {
                Mode::PktUsb => Ok(0x12),
                Mode::PktLsb => Ok(0x13),
                _ => self
                    .mode_map()
                    .iter()
                    .find(|mapping| mapping.mode == mode)
                    .map(|mapping| mapping.code)
                    .ok_or_else(|| RadioError::UnsupportedModeForRadio {
                        mode: mode.to_string(),
                        radio: self.model.as_str(),
                    }),
            };
        }

        self.mode_map()
            .iter()
            .find(|mapping| mapping.mode == mode)
            .map(|mapping| mapping.code)
            .ok_or_else(|| RadioError::UnsupportedModeForRadio {
                mode: mode.to_string(),
                radio: self.model.as_str(),
            })
    }

    async fn query_command(&self, command: &[u8], expected_prefix: &[u8]) -> Result<Vec<u8>> {
        self.io
            .transact(
                self.addressing,
                command,
                ExpectedResponse::Prefix(expected_prefix.to_vec()),
                self.retry,
            )
            .await
    }

    async fn send_command(&self, command: &[u8], expected_prefix: &[u8]) -> Result<()> {
        self.io
            .transact(
                self.addressing,
                command,
                ExpectedResponse::AckOrPrefix(expected_prefix.to_vec()),
                self.retry,
            )
            .await?;
        Ok(())
    }

    fn frequency_bcd_bytes(&self) -> usize {
        match self.descriptor().frequency_family {
            FrequencyFamily::Bcd5 => 5,
            FrequencyFamily::Old731Bcd4 => 4,
        }
    }

    fn max_frequency_hz(&self) -> u64 {
        match self.descriptor().frequency_family {
            FrequencyFamily::Bcd5 => MAX_FREQUENCY_HZ_5_BCD,
            FrequencyFamily::Old731Bcd4 => MAX_FREQUENCY_HZ_4_BCD,
        }
    }

    fn parse_frequency_response(&self, response: &[u8]) -> Result<Frequency> {
        let required = 1 + self.frequency_bcd_bytes();

        if response.len() < required || response[0] != 0x03 {
            return Err(RadioError::CivProtocol(format!(
                "invalid frequency response for {}: {:?}",
                self.model.as_str(),
                response
            )));
        }

        let frequency_hz = decode_bcd_frequency(&response[1..required])?;
        Ok(Frequency::from_hz(frequency_hz))
    }

    fn format_frequency_set(&self, frequency: Frequency) -> Result<Vec<u8>> {
        let frequency_hz = frequency.hz();
        if !(1..=self.max_frequency_hz()).contains(&frequency_hz) {
            return Err(RadioError::FrequencyOutOfRange(frequency_hz));
        }

        let mut command = vec![0x05];
        command.extend_from_slice(&encode_bcd_frequency(
            frequency_hz,
            self.frequency_bcd_bytes(),
        )?);
        Ok(command)
    }

    async fn query_data_mode_enabled(&self) -> Result<bool> {
        match self.descriptor().data_overlay_family {
            DataOverlayFamily::None => Ok(false),
            DataOverlayFamily::Civ1a06 => {
                let response = self.query_command(&[0x1A, 0x06], &[0x1A, 0x06]).await?;
                if response.len() < 3 {
                    return Err(RadioError::CivProtocol(format!(
                        "invalid data-mode response for {}: {:?}",
                        self.model.as_str(),
                        response
                    )));
                }
                Ok(response[2] != 0)
            }
        }
    }

    async fn set_data_mode_enabled(&self, enabled: bool) -> Result<()> {
        match self.descriptor().data_overlay_family {
            DataOverlayFamily::None => Ok(()),
            DataOverlayFamily::Civ1a06 => {
                let command = [0x1A, 0x06, if enabled { 0x01 } else { 0x00 }];
                self.send_command(&command, &[0x1A, 0x06]).await
            }
        }
    }

    fn map_packet_mode_to_base(mode: Mode) -> Option<Mode> {
        match mode {
            Mode::PktUsb => Some(Mode::Usb),
            Mode::PktLsb => Some(Mode::Lsb),
            Mode::PktFm => Some(Mode::Fm),
            Mode::PktAm => Some(Mode::Am),
            _ => None,
        }
    }

    fn map_base_to_packet_mode(mode: Mode) -> Option<Mode> {
        match mode {
            Mode::Usb => Some(Mode::PktUsb),
            Mode::Lsb => Some(Mode::PktLsb),
            Mode::Fm => Some(Mode::PktFm),
            Mode::Am => Some(Mode::PktAm),
            _ => None,
        }
    }

    fn parse_mode_response(&self, response: &[u8]) -> Result<Mode> {
        if response.len() < 2 || response[0] != 0x04 {
            return Err(RadioError::CivProtocol(format!(
                "invalid mode response for {}: {:?}",
                self.model.as_str(),
                response
            )));
        }

        self.decode_mode_code(response[1])
    }

    fn format_mode_set(&self, mode: Mode) -> Result<Vec<u8>> {
        let code = self.encode_mode_code(mode)?;
        // Use filter preset 1 by default. Width handling is intentionally out of scope for the
        // ControllableRadio-only surface in this crate.
        Ok(vec![0x06, code, 0x01])
    }

    fn check_keyer_support(&self, operation: &'static str) -> Result<()> {
        if self.descriptor().keyer_supported {
            Ok(())
        } else {
            Err(RadioError::UnsupportedOperation {
                operation,
                radio: self.model.as_str(),
            })
        }
    }

    fn check_morse_send_support(&self) -> Result<()> {
        if matches!(
            self.descriptor().morse_family,
            MorseFamily::SendOnly | MorseFamily::SendStop
        ) {
            Ok(())
        } else {
            Err(RadioError::UnsupportedOperation {
                operation: "send-cw",
                radio: self.model.as_str(),
            })
        }
    }

    fn validate_cw_text(text: &str) -> Result<()> {
        if text.is_empty()
            || !text.is_ascii()
            || text.contains('\r')
            || text.contains('\n')
            || text.len() > MAX_CW_TEXT_BYTES
        {
            if text.len() > MAX_CW_TEXT_BYTES {
                return Err(RadioError::CwTextTooLong(text.len()));
            }
            return Err(RadioError::InvalidCwText);
        }

        Ok(())
    }

    fn parse_keyer_value(response: &[u8]) -> Result<u16> {
        if response.len() < 4 || response[0] != 0x14 || response[1] != KEYER_SPEED_SUBCOMMAND {
            return Err(RadioError::CivProtocol(format!(
                "invalid keyer response: {:?}",
                response
            )));
        }

        decode_bcd_u16(&response[2..4])
    }

    fn format_keyer_set(wpm: u16) -> Result<[u8; 4]> {
        if !(MIN_CW_WPM..=MAX_CW_WPM).contains(&wpm) {
            return Err(RadioError::CwSpeedOutOfRange(wpm));
        }

        let [byte0, byte1] = encode_bcd_u16(wpm);
        Ok([0x14, KEYER_SPEED_SUBCOMMAND, byte0, byte1])
    }
}

#[async_trait]
impl ControllableRadio for IcomCivRadio {
    async fn get_frequency(&self) -> Result<Frequency> {
        let response = self.query_command(&[0x03], &[0x03]).await?;
        self.parse_frequency_response(&response)
    }

    async fn set_frequency(&self, frequency: Frequency) -> Result<()> {
        let command = self.format_frequency_set(frequency)?;
        self.send_command(&command, &[0x05]).await
    }

    async fn get_mode(&self) -> Result<Mode> {
        let response = self.query_command(&[0x04], &[0x04]).await?;
        let mode = self.parse_mode_response(&response)?;

        if self.descriptor().data_overlay_family == DataOverlayFamily::None {
            return Ok(mode);
        }

        if let Some(packet_mode) = Self::map_base_to_packet_mode(mode) {
            if self.query_data_mode_enabled().await? {
                return Ok(packet_mode);
            }
        }

        Ok(mode)
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        if self.descriptor().data_overlay_family != DataOverlayFamily::None {
            if let Some(base_mode) = Self::map_packet_mode_to_base(mode) {
                let command = self.format_mode_set(base_mode)?;
                self.send_command(&command, &[0x06]).await?;
                return self.set_data_mode_enabled(true).await;
            }

            if matches!(mode, Mode::Usb | Mode::Lsb | Mode::Fm | Mode::Am) {
                let command = self.format_mode_set(mode)?;
                self.send_command(&command, &[0x06]).await?;
                return self.set_data_mode_enabled(false).await;
            }
        }

        let command = self.format_mode_set(mode)?;
        self.send_command(&command, &[0x06]).await
    }

    async fn send_cw(&self, text: &str) -> Result<()> {
        self.check_morse_send_support()?;
        Self::validate_cw_text(text)?;

        for chunk in text.as_bytes().chunks(CIV_CW_CHUNK_BYTES) {
            let mut command = Vec::with_capacity(chunk.len() + 1);
            command.push(0x17);
            command.extend_from_slice(chunk);
            self.send_command(&command, &[0x17]).await?;
        }

        Ok(())
    }

    async fn stop_cw(&self) -> Result<()> {
        match self.descriptor().morse_family {
            MorseFamily::SendStop => self.send_command(&[0x17, 0xFF], &[0x17]).await,
            MorseFamily::SendOnly | MorseFamily::None => Err(RadioError::UnsupportedOperation {
                operation: "stop-cw",
                radio: self.model.as_str(),
            }),
        }
    }

    async fn get_cw_wpm(&self) -> Result<u16> {
        self.check_keyer_support("get-cw-wpm")?;
        let response = self
            .query_command(
                &[0x14, KEYER_SPEED_SUBCOMMAND],
                &[0x14, KEYER_SPEED_SUBCOMMAND],
            )
            .await?;
        Self::parse_keyer_value(&response)
    }

    async fn set_cw_wpm(&self, wpm: u16) -> Result<()> {
        self.check_keyer_support("set-cw-wpm")?;
        let command = Self::format_keyer_set(wpm)?;
        self.send_command(&command, &[0x14, KEYER_SPEED_SUBCOMMAND])
            .await
    }
}

fn decode_bcd_frequency(bytes: &[u8]) -> Result<u64> {
    let mut multiplier = 1_u64;
    let mut frequency_hz = 0_u64;

    for byte in bytes {
        let low = byte & 0x0F;
        let high = (byte >> 4) & 0x0F;

        if low > 9 || high > 9 {
            return Err(RadioError::CivProtocol(format!(
                "invalid BCD frequency byte: 0x{byte:02X}"
            )));
        }

        frequency_hz += u64::from(low) * multiplier;
        multiplier *= 10;
        frequency_hz += u64::from(high) * multiplier;
        multiplier *= 10;
    }

    Ok(frequency_hz)
}

fn encode_bcd_frequency(value: u64, byte_count: usize) -> Result<Vec<u8>> {
    let mut remaining = value;
    let mut bytes = Vec::with_capacity(byte_count);

    for _ in 0..byte_count {
        let low = (remaining % 10) as u8;
        remaining /= 10;
        let high = (remaining % 10) as u8;
        remaining /= 10;
        bytes.push((high << 4) | low);
    }

    if remaining > 0 {
        return Err(RadioError::FrequencyOutOfRange(value));
    }

    Ok(bytes)
}

fn decode_bcd_u16(bytes: &[u8]) -> Result<u16> {
    if bytes.len() != 2 {
        return Err(RadioError::CivProtocol(format!(
            "invalid 2-byte BCD width: {}",
            bytes.len()
        )));
    }

    let mut value = 0_u16;
    let mut multiplier = 1_u16;

    for byte in bytes {
        let low = byte & 0x0F;
        let high = (byte >> 4) & 0x0F;

        if low > 9 || high > 9 {
            return Err(RadioError::CivProtocol(format!(
                "invalid BCD byte in keyer value: 0x{byte:02X}"
            )));
        }

        value += u16::from(low) * multiplier;
        multiplier *= 10;
        value += u16::from(high) * multiplier;
        multiplier *= 10;
    }

    Ok(value)
}

fn encode_bcd_u16(value: u16) -> [u8; 2] {
    let mut remaining = value;

    let low0 = (remaining % 10) as u8;
    remaining /= 10;
    let high0 = (remaining % 10) as u8;
    remaining /= 10;

    let low1 = (remaining % 10) as u8;
    remaining /= 10;
    let high1 = (remaining % 10) as u8;

    [(high0 << 4) | low0, (high1 << 4) | low1]
}

fn parse_u8_option(options: &RadioOptions, key: &str) -> Result<Option<u8>> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };

    let value = value.trim();
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u8::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(|_| RadioError::InvalidOption {
        key: key.to_string(),
        value: value.to_string(),
    })?;

    Ok(Some(parsed))
}

fn parse_u64_option(options: &RadioOptions, key: &str) -> Result<Option<u64>> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };

    let parsed = value
        .trim()
        .parse()
        .map_err(|_| RadioError::InvalidOption {
            key: key.to_string(),
            value: value.to_string(),
        })?;

    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use tokio::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MockCivIo {
        sent: Mutex<Vec<Vec<u8>>>,
        responses: Mutex<VecDeque<Vec<u8>>>,
    }

    impl MockCivIo {
        async fn push_response(&self, response: &[u8]) {
            self.responses.lock().await.push_back(response.to_vec());
        }

        async fn sent_commands(&self) -> Vec<Vec<u8>> {
            self.sent.lock().await.clone()
        }
    }

    #[async_trait]
    impl CivIo for MockCivIo {
        async fn transact(
            &self,
            _addressing: CivAddressing,
            command_data: &[u8],
            _expected: ExpectedResponse,
            _retry: RetryPolicy,
        ) -> Result<Vec<u8>> {
            self.sent.lock().await.push(command_data.to_vec());
            self.responses
                .lock()
                .await
                .pop_front()
                .ok_or_else(|| RadioError::CivProtocol("missing queued mock response".to_string()))
        }
    }

    fn default_retry() -> RetryPolicy {
        RetryPolicy {
            max_retries: 0,
            backoff: Duration::from_millis(0),
        }
    }

    fn default_addressing() -> CivAddressing {
        CivAddressing {
            rig_addr: 0x94,
            controller_addr: 0xE0,
        }
    }

    #[test]
    fn bcd_frequency_round_trip() {
        let encoded = encode_bcd_frequency(14_074_000, 5).unwrap();
        assert_eq!(decode_bcd_frequency(&encoded).unwrap(), 14_074_000);
    }

    #[test]
    fn parses_icom_model_alias() {
        assert_eq!(IcomModel::from_alias("ic-7300"), Some(IcomModel::Ic7300));
        assert_eq!(IcomModel::from_alias("IC7610"), Some(IcomModel::Ic7610));
        assert_eq!(
            IcomModel::from_alias("ic-706mkiig"),
            Some(IcomModel::Ic706Mkiig)
        );
        assert_eq!(IcomModel::from_alias("x-108g"), Some(IcomModel::X108g));
        assert_eq!(IcomModel::from_alias("X6100"), Some(IcomModel::X6100));
        assert_eq!(IcomModel::from_alias("x6200"), Some(IcomModel::X6200));
        assert_eq!(IcomModel::from_alias("g90"), Some(IcomModel::G90));
        assert_eq!(IcomModel::from_alias("x5105"), Some(IcomModel::X5105));
    }

    #[tokio::test]
    async fn overlays_packet_modes_for_modern_profile() {
        let io = Arc::new(MockCivIo::default());
        io.push_response(&[0x04, 0x01]).await;
        io.push_response(&[0x1A, 0x06, 0x01]).await;

        let radio = IcomCivRadio::from_io(
            io,
            IcomModel::Ic7300,
            IcomProfile::ModernDirect,
            default_addressing(),
            default_retry(),
        );

        assert_eq!(radio.get_mode().await.unwrap(), Mode::PktUsb);
    }

    #[tokio::test]
    async fn sends_expected_commands_for_packet_mode_set() {
        let io = Arc::new(MockCivIo::default());
        io.push_response(&[CIV_ACK]).await;
        io.push_response(&[CIV_ACK]).await;

        let radio = IcomCivRadio::from_io(
            io.clone(),
            IcomModel::Ic7300,
            IcomProfile::ModernDirect,
            default_addressing(),
            default_retry(),
        );

        radio.set_mode(Mode::PktUsb).await.unwrap();

        assert_eq!(
            io.sent_commands().await,
            vec![vec![0x06, 0x01, 0x01], vec![0x1A, 0x06, 0x01]]
        );
    }

    #[tokio::test]
    async fn stop_is_unsupported_for_send_only_profiles() {
        let io = Arc::new(MockCivIo::default());
        let radio = IcomCivRadio::from_io(
            io,
            IcomModel::Ic7600,
            IcomProfile::Icom7600,
            default_addressing(),
            default_retry(),
        );

        let error = radio.stop_cw().await.unwrap_err();
        assert!(matches!(
            error,
            RadioError::UnsupportedOperation {
                operation: "stop-cw",
                ..
            }
        ));
    }
}
