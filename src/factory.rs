use std::str::FromStr;

use tracing::debug;

use crate::{
    kenwood::KenwoodProfile, ConnectionConfig, ControllableRadio, KenwoodRadio, RadioError, Result,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioKind {
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

impl RadioKind {
    pub const ALL: &'static [Self] = &[
        Self::KenwoodClassic,
        Self::KenwoodClassicKeyer,
        Self::KenwoodClassicMorse,
        Self::KenwoodClassicKeyerMorse,
        Self::KenwoodTs940,
        Self::KenwoodTs570,
        Self::KenwoodTs480,
        Self::KenwoodTs480PlainCw,
        Self::KenwoodTs480Minimal,
        Self::KenwoodTs480SdrUno,
        Self::KenwoodTs590,
        Self::KenwoodTs890,
        Self::KenwoodTs990,
        Self::ElecraftK2,
        Self::ElecraftK3,
        Self::ElecraftK4,
        Self::Ic10Derived,
        Self::Flex6xxx,
        Self::PowerSdrThetis,
    ];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::KenwoodClassic => "kenwood-classic",
            Self::KenwoodClassicKeyer => "kenwood-classic-keyer",
            Self::KenwoodClassicMorse => "kenwood-classic-morse",
            Self::KenwoodClassicKeyerMorse => "kenwood-classic-keyer-morse",
            Self::KenwoodTs940 => "kenwood-ts940",
            Self::KenwoodTs570 => "kenwood-ts570",
            Self::KenwoodTs480 => "kenwood-ts480",
            Self::KenwoodTs480PlainCw => "kenwood-ts480-plain-cw",
            Self::KenwoodTs480Minimal => "kenwood-ts480-minimal",
            Self::KenwoodTs480SdrUno => "kenwood-ts480-sdruno",
            Self::KenwoodTs590 => "kenwood-ts590",
            Self::KenwoodTs890 => "kenwood-ts890",
            Self::KenwoodTs990 => "kenwood-ts990",
            Self::ElecraftK2 => "elecraft-k2",
            Self::ElecraftK3 => "elecraft-k3",
            Self::ElecraftK4 => "elecraft-k4",
            Self::Ic10Derived => "ic10-derived",
            Self::Flex6xxx => "flex-6xxx",
            Self::PowerSdrThetis => "powersdr-thetis",
        }
    }

    fn profile(self) -> KenwoodProfile {
        match self {
            Self::KenwoodClassic => KenwoodProfile::KenwoodClassic,
            Self::KenwoodClassicKeyer => KenwoodProfile::KenwoodClassicKeyer,
            Self::KenwoodClassicMorse => KenwoodProfile::KenwoodClassicMorse,
            Self::KenwoodClassicKeyerMorse => KenwoodProfile::KenwoodClassicKeyerMorse,
            Self::KenwoodTs940 => KenwoodProfile::KenwoodTs940,
            Self::KenwoodTs570 => KenwoodProfile::KenwoodTs570,
            Self::KenwoodTs480 => KenwoodProfile::KenwoodTs480,
            Self::KenwoodTs480PlainCw => KenwoodProfile::KenwoodTs480PlainCw,
            Self::KenwoodTs480Minimal => KenwoodProfile::KenwoodTs480Minimal,
            Self::KenwoodTs480SdrUno => KenwoodProfile::KenwoodTs480SdrUno,
            Self::KenwoodTs590 => KenwoodProfile::KenwoodTs590,
            Self::KenwoodTs890 => KenwoodProfile::KenwoodTs890,
            Self::KenwoodTs990 => KenwoodProfile::KenwoodTs990,
            Self::ElecraftK2 => KenwoodProfile::ElecraftK2,
            Self::ElecraftK3 => KenwoodProfile::ElecraftK3,
            Self::ElecraftK4 => KenwoodProfile::ElecraftK4,
            Self::Ic10Derived => KenwoodProfile::Ic10Derived,
            Self::Flex6xxx => KenwoodProfile::Flex6xxx,
            Self::PowerSdrThetis => KenwoodProfile::PowerSdrThetis,
        }
    }
}

impl FromStr for RadioKind {
    type Err = RadioError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match normalize_radio_name(value).as_str() {
            "kenwood" | "kenwoodclassic" | "ts140s" | "ts680s" | "ts711" | "ts790" | "ts811"
            | "ts690s" | "ts450s" | "ts850" | "trc80" | "sdrconsole" | "usdx" | "hamgeekusdx" => {
                Ok(Self::KenwoodClassic)
            }
            "kenwoodclassickeyer" | "ts50s" | "ts930" => Ok(Self::KenwoodClassicKeyer),
            "kenwoodclassicmorse" | "ts950s" | "ts950sdx" | "ts870s" => {
                Ok(Self::KenwoodClassicMorse)
            }
            "kenwoodclassickeyermorse" | "ts2000" | "pihpsdr" | "tx500" => {
                Ok(Self::KenwoodClassicKeyerMorse)
            }
            "kenwoodts940" | "ts940s" => Ok(Self::KenwoodTs940),
            "kenwoodts570" | "ts570s" | "ts570d" => Ok(Self::KenwoodTs570),
            "kenwoodts480" | "ts480" | "trusdx" => Ok(Self::KenwoodTs480),
            "kenwoodts480plaincw" | "qcx" | "qdx" | "qcxqdx" => Ok(Self::KenwoodTs480PlainCw),
            "kenwoodts480minimal" | "qmx" | "pt8000a" | "dsp" | "malachite" => {
                Ok(Self::KenwoodTs480Minimal)
            }
            "kenwoodts480sdruno" | "sdruno" => Ok(Self::KenwoodTs480SdrUno),
            "kenwoodts590" | "ts590s" | "ts590sg" | "fx4" | "fx4c" | "fx4cr" | "fx4l" => {
                Ok(Self::KenwoodTs590)
            }
            "kenwoodts890" | "ts890s" => Ok(Self::KenwoodTs890),
            "kenwoodts990" | "ts990s" => Ok(Self::KenwoodTs990),
            "elecraftk2" | "k2" => Ok(Self::ElecraftK2),
            "elecraftk3" | "k3" | "k3s" | "kx2" | "kx3" => Ok(Self::ElecraftK3),
            "elecraftk4" | "k4" | "elecraft" => Ok(Self::ElecraftK4),
            "ic10derived" | "ts440s" | "r5000" => Ok(Self::Ic10Derived),
            "flex6xxx" | "6xxx" | "flex" => Ok(Self::Flex6xxx),
            "powersdrthetis" | "powersdr" | "thetis" => Ok(Self::PowerSdrThetis),
            _ => Err(RadioError::UnknownRadio(value.to_string())),
        }
    }
}

fn normalize_radio_name(value: &str) -> String {
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

pub const fn supported_radio_kinds() -> &'static [RadioKind] {
    RadioKind::all()
}

pub async fn create_radio(
    kind: RadioKind,
    connection: ConnectionConfig,
) -> Result<Box<dyn ControllableRadio>> {
    debug!(radio_kind = kind.as_str(), ?connection, "creating radio");
    Ok(Box::new(
        KenwoodRadio::connect(connection, kind.profile()).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{supported_radio_kinds, RadioKind};

    #[test]
    fn lists_supported_radio_kinds() {
        assert_eq!(
            supported_radio_kinds(),
            &[
                RadioKind::KenwoodClassic,
                RadioKind::KenwoodClassicKeyer,
                RadioKind::KenwoodClassicMorse,
                RadioKind::KenwoodClassicKeyerMorse,
                RadioKind::KenwoodTs940,
                RadioKind::KenwoodTs570,
                RadioKind::KenwoodTs480,
                RadioKind::KenwoodTs480PlainCw,
                RadioKind::KenwoodTs480Minimal,
                RadioKind::KenwoodTs480SdrUno,
                RadioKind::KenwoodTs590,
                RadioKind::KenwoodTs890,
                RadioKind::KenwoodTs990,
                RadioKind::ElecraftK2,
                RadioKind::ElecraftK3,
                RadioKind::ElecraftK4,
                RadioKind::Ic10Derived,
                RadioKind::Flex6xxx,
                RadioKind::PowerSdrThetis,
            ]
        );
    }

    #[test]
    fn parses_protocol_aliases() {
        for (alias, expected) in [
            ("ts-590sg", RadioKind::KenwoodTs590),
            ("k4", RadioKind::ElecraftK4),
            ("k3s", RadioKind::ElecraftK3),
            ("6xxx", RadioKind::Flex6xxx),
            ("powersdr", RadioKind::PowerSdrThetis),
            ("ts-440s", RadioKind::Ic10Derived),
            ("qcx", RadioKind::KenwoodTs480PlainCw),
            ("sdruno", RadioKind::KenwoodTs480SdrUno),
        ] {
            assert_eq!(alias.parse::<RadioKind>().unwrap(), expected);
        }
    }
}
