use std::str::FromStr;

use tracing::debug;

use crate::{
    icom_civ::{IcomCivRadio, IcomModel},
    kenwood::KenwoodProfile,
    options::RadioOptions,
    ConnectionConfig, ControllableRadio, KenwoodRadio, RadioError, Result,
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
    Icom(IcomModel),
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
        Self::Icom(IcomModel::Ic707),
        Self::Icom(IcomModel::Ic725),
        Self::Icom(IcomModel::Ic726),
        Self::Icom(IcomModel::Ic728),
        Self::Icom(IcomModel::Ic729),
        Self::Icom(IcomModel::Ic735),
        Self::Icom(IcomModel::Ic736),
        Self::Icom(IcomModel::Ic737),
        Self::Icom(IcomModel::Ic738),
        Self::Icom(IcomModel::Ic751),
        Self::Icom(IcomModel::Ic761),
        Self::Icom(IcomModel::Ic765),
        Self::Icom(IcomModel::Ic775),
        Self::Icom(IcomModel::Ic781),
        Self::Icom(IcomModel::Ic271),
        Self::Icom(IcomModel::Ic275),
        Self::Icom(IcomModel::Ic375),
        Self::Icom(IcomModel::Ic471),
        Self::Icom(IcomModel::Ic475),
        Self::Icom(IcomModel::Ic575),
        Self::Icom(IcomModel::Ic820h),
        Self::Icom(IcomModel::Ic821h),
        Self::Icom(IcomModel::Ic970),
        Self::Icom(IcomModel::Ic1275),
        Self::Icom(IcomModel::Ic706),
        Self::Icom(IcomModel::Ic706Mkii),
        Self::Icom(IcomModel::Ic706Mkiig),
        Self::Icom(IcomModel::Ic78),
        Self::Icom(IcomModel::Ic703),
        Self::Icom(IcomModel::Ic718),
        Self::Icom(IcomModel::Ic746),
        Self::Icom(IcomModel::Ic746Pro),
        Self::Icom(IcomModel::Ic756),
        Self::Icom(IcomModel::Ic756Pro),
        Self::Icom(IcomModel::Ic756ProIi),
        Self::Icom(IcomModel::Ic756ProIii),
        Self::Icom(IcomModel::Ic7000),
        Self::Icom(IcomModel::Ic7200),
        Self::Icom(IcomModel::Ic7410),
        Self::Icom(IcomModel::Ic910),
        Self::Icom(IcomModel::Ic9100),
        Self::Icom(IcomModel::Ic7100),
        Self::Icom(IcomModel::Ic7600),
        Self::Icom(IcomModel::Ic7700),
        Self::Icom(IcomModel::Ic7800),
        Self::Icom(IcomModel::Ic7300),
        Self::Icom(IcomModel::Ic7300Mk2),
        Self::Icom(IcomModel::Ic705),
        Self::Icom(IcomModel::Ic7610),
        Self::Icom(IcomModel::Ic7760),
        Self::Icom(IcomModel::Ic7850),
        Self::Icom(IcomModel::Ic7851),
        Self::Icom(IcomModel::Ic905),
        Self::Icom(IcomModel::Ic9700),
        Self::Icom(IcomModel::IcF8101),
        Self::Icom(IcomModel::X108g),
        Self::Icom(IcomModel::X6100),
        Self::Icom(IcomModel::X6200),
        Self::Icom(IcomModel::G90),
        Self::Icom(IcomModel::X5105),
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
            Self::Icom(model) => model.as_str(),
        }
    }

    fn kenwood_profile(self) -> Option<KenwoodProfile> {
        match self {
            Self::KenwoodClassic => Some(KenwoodProfile::KenwoodClassic),
            Self::KenwoodClassicKeyer => Some(KenwoodProfile::KenwoodClassicKeyer),
            Self::KenwoodClassicMorse => Some(KenwoodProfile::KenwoodClassicMorse),
            Self::KenwoodClassicKeyerMorse => Some(KenwoodProfile::KenwoodClassicKeyerMorse),
            Self::KenwoodTs940 => Some(KenwoodProfile::KenwoodTs940),
            Self::KenwoodTs570 => Some(KenwoodProfile::KenwoodTs570),
            Self::KenwoodTs480 => Some(KenwoodProfile::KenwoodTs480),
            Self::KenwoodTs480PlainCw => Some(KenwoodProfile::KenwoodTs480PlainCw),
            Self::KenwoodTs480Minimal => Some(KenwoodProfile::KenwoodTs480Minimal),
            Self::KenwoodTs480SdrUno => Some(KenwoodProfile::KenwoodTs480SdrUno),
            Self::KenwoodTs590 => Some(KenwoodProfile::KenwoodTs590),
            Self::KenwoodTs890 => Some(KenwoodProfile::KenwoodTs890),
            Self::KenwoodTs990 => Some(KenwoodProfile::KenwoodTs990),
            Self::ElecraftK2 => Some(KenwoodProfile::ElecraftK2),
            Self::ElecraftK3 => Some(KenwoodProfile::ElecraftK3),
            Self::ElecraftK4 => Some(KenwoodProfile::ElecraftK4),
            Self::Ic10Derived => Some(KenwoodProfile::Ic10Derived),
            Self::Flex6xxx => Some(KenwoodProfile::Flex6xxx),
            Self::PowerSdrThetis => Some(KenwoodProfile::PowerSdrThetis),
            Self::Icom(_) => None,
        }
    }
}

impl FromStr for RadioKind {
    type Err = RadioError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = normalize_radio_name(value);

        let kind = match normalized.as_str() {
            "kenwood" | "kenwoodclassic" | "ts140s" | "ts680s" | "ts711" | "ts790" | "ts811"
            | "ts690s" | "ts450s" | "ts850" | "trc80" | "sdrconsole" | "usdx" | "hamgeekusdx" => {
                Some(Self::KenwoodClassic)
            }
            "kenwoodclassickeyer" | "ts50s" | "ts930" => Some(Self::KenwoodClassicKeyer),
            "kenwoodclassicmorse" | "ts950s" | "ts950sdx" | "ts870s" => {
                Some(Self::KenwoodClassicMorse)
            }
            "kenwoodclassickeyermorse" | "ts2000" | "pihpsdr" | "tx500" => {
                Some(Self::KenwoodClassicKeyerMorse)
            }
            "kenwoodts940" | "ts940s" => Some(Self::KenwoodTs940),
            "kenwoodts570" | "ts570s" | "ts570d" => Some(Self::KenwoodTs570),
            "kenwoodts480" | "ts480" | "trusdx" => Some(Self::KenwoodTs480),
            "kenwoodts480plaincw" | "qcx" | "qdx" | "qcxqdx" => Some(Self::KenwoodTs480PlainCw),
            "kenwoodts480minimal" | "qmx" | "pt8000a" | "dsp" | "malachite" => {
                Some(Self::KenwoodTs480Minimal)
            }
            "kenwoodts480sdruno" | "sdruno" => Some(Self::KenwoodTs480SdrUno),
            "kenwoodts590" | "ts590s" | "ts590sg" | "fx4" | "fx4c" | "fx4cr" | "fx4l" => {
                Some(Self::KenwoodTs590)
            }
            "kenwoodts890" | "ts890s" => Some(Self::KenwoodTs890),
            "kenwoodts990" | "ts990s" => Some(Self::KenwoodTs990),
            "elecraftk2" | "k2" => Some(Self::ElecraftK2),
            "elecraftk3" | "k3" | "k3s" | "kx2" | "kx3" => Some(Self::ElecraftK3),
            "elecraftk4" | "k4" | "elecraft" => Some(Self::ElecraftK4),
            "ic10derived" | "ts440s" | "r5000" => Some(Self::Ic10Derived),
            "flex6xxx" | "6xxx" | "flex" => Some(Self::Flex6xxx),
            "powersdrthetis" | "powersdr" | "thetis" => Some(Self::PowerSdrThetis),
            _ => None,
        };

        if let Some(kind) = kind {
            return Ok(kind);
        }

        if let Some(model) = IcomModel::from_alias(value) {
            return Ok(Self::Icom(model));
        }

        Err(RadioError::UnknownRadio(value.to_string()))
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
    create_radio_with_options(kind, connection, "").await
}

pub async fn create_radio_with_options(
    kind: RadioKind,
    connection: ConnectionConfig,
    options: &str,
) -> Result<Box<dyn ControllableRadio>> {
    debug!(
        radio_kind = kind.as_str(),
        ?connection,
        options,
        "creating radio"
    );

    let parsed_options = RadioOptions::parse(options);

    if let Some(profile) = kind.kenwood_profile() {
        return Ok(Box::new(KenwoodRadio::connect(connection, profile).await?));
    }

    match kind {
        RadioKind::Icom(model) => Ok(Box::new(
            IcomCivRadio::connect(connection, model, &parsed_options).await?,
        )),
        _ => unreachable!("all non-Icom kinds are mapped through kenwood_profile"),
    }
}

#[cfg(test)]
mod tests {
    use super::{supported_radio_kinds, RadioKind};
    use crate::IcomModel;

    #[test]
    fn lists_supported_radio_kinds() {
        let kinds = supported_radio_kinds();
        assert!(kinds.contains(&RadioKind::KenwoodTs590));
        assert!(kinds.contains(&RadioKind::Icom(IcomModel::Ic7300)));
        assert!(kinds.contains(&RadioKind::Icom(IcomModel::Ic7610)));
    }

    #[test]
    fn parses_protocol_aliases() {
        for (alias, expected) in [
            ("ts-590sg", RadioKind::KenwoodTs590),
            ("k4", RadioKind::ElecraftK4),
            ("6xxx", RadioKind::Flex6xxx),
            ("ic-7300", RadioKind::Icom(IcomModel::Ic7300)),
            ("IC7610", RadioKind::Icom(IcomModel::Ic7610)),
            ("ic-706mkiig", RadioKind::Icom(IcomModel::Ic706Mkiig)),
            ("x108g", RadioKind::Icom(IcomModel::X108g)),
            ("x6100", RadioKind::Icom(IcomModel::X6100)),
            ("x6200", RadioKind::Icom(IcomModel::X6200)),
            ("g90", RadioKind::Icom(IcomModel::G90)),
            ("x5105", RadioKind::Icom(IcomModel::X5105)),
        ] {
            assert_eq!(alias.parse::<RadioKind>().unwrap(), expected);
        }
    }
}
