use std::str::FromStr;

use tracing::debug;

use crate::{
    flex_native::{FlexNativeModel, FlexNativeRadio},
    icom_civ::{IcomCivRadio, IcomModel},
    kenwood::KenwoodModel,
    options::RadioOptions,
    yaesu_newcat::{YaesuModel, YaesuNewCatRadio},
    ConnectionConfig, ControllableRadio, KenwoodRadio, RadioError, Result,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioKind {
    Kenwood(KenwoodModel),
    Icom(IcomModel),
    Yaesu(YaesuModel),
    FlexNative(FlexNativeModel),
}

impl RadioKind {
    pub const ALL: &'static [Self] = &[
        Self::Kenwood(KenwoodModel::Ts140s),
        Self::Kenwood(KenwoodModel::Ts680s),
        Self::Kenwood(KenwoodModel::Ts711),
        Self::Kenwood(KenwoodModel::Ts790),
        Self::Kenwood(KenwoodModel::Ts811),
        Self::Kenwood(KenwoodModel::Ts690s),
        Self::Kenwood(KenwoodModel::Ts50s),
        Self::Kenwood(KenwoodModel::Ts930),
        Self::Kenwood(KenwoodModel::Ts940s),
        Self::Kenwood(KenwoodModel::Ts950s),
        Self::Kenwood(KenwoodModel::Ts950Sdx),
        Self::Kenwood(KenwoodModel::Ts440s),
        Self::Kenwood(KenwoodModel::R5000),
        Self::Kenwood(KenwoodModel::Ts450s),
        Self::Kenwood(KenwoodModel::Ts850),
        Self::Kenwood(KenwoodModel::Ts870s),
        Self::Kenwood(KenwoodModel::Ts570s),
        Self::Kenwood(KenwoodModel::Ts570d),
        Self::Kenwood(KenwoodModel::Ts2000),
        Self::Kenwood(KenwoodModel::SdrConsole),
        Self::Kenwood(KenwoodModel::Ts480),
        Self::Kenwood(KenwoodModel::TrUsdx),
        Self::Kenwood(KenwoodModel::Qcx),
        Self::Kenwood(KenwoodModel::Qdx),
        Self::Kenwood(KenwoodModel::Qmx),
        Self::Kenwood(KenwoodModel::Pt8000a),
        Self::Kenwood(KenwoodModel::SdrUno),
        Self::Kenwood(KenwoodModel::DspMalachite),
        Self::Kenwood(KenwoodModel::Ts590s),
        Self::Kenwood(KenwoodModel::Ts590sg),
        Self::Kenwood(KenwoodModel::Fx4),
        Self::Kenwood(KenwoodModel::Fx4c),
        Self::Kenwood(KenwoodModel::Fx4cr),
        Self::Kenwood(KenwoodModel::Fx4l),
        Self::Kenwood(KenwoodModel::Ts890s),
        Self::Kenwood(KenwoodModel::Ts990s),
        Self::Kenwood(KenwoodModel::Trc80),
        Self::Kenwood(KenwoodModel::K2),
        Self::Kenwood(KenwoodModel::K3),
        Self::Kenwood(KenwoodModel::K3s),
        Self::Kenwood(KenwoodModel::K4),
        Self::Kenwood(KenwoodModel::Kx3),
        Self::Kenwood(KenwoodModel::Kx2),
        Self::Kenwood(KenwoodModel::Flex6xxx),
        Self::Kenwood(KenwoodModel::PowerSdr),
        Self::Kenwood(KenwoodModel::Thetis),
        Self::Kenwood(KenwoodModel::PiHpsdr),
        Self::Kenwood(KenwoodModel::UsdxHamgeek),
        Self::Kenwood(KenwoodModel::Tx500),
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
        Self::Icom(IcomModel::X108g),
        Self::Icom(IcomModel::X6100),
        Self::Icom(IcomModel::X6200),
        Self::Icom(IcomModel::G90),
        Self::Icom(IcomModel::X5105),
        Self::Yaesu(YaesuModel::Ft450),
        Self::Yaesu(YaesuModel::Ft950),
        Self::Yaesu(YaesuModel::Ft2000),
        Self::Yaesu(YaesuModel::Ftdx1200),
        Self::Yaesu(YaesuModel::Ftdx3000),
        Self::Yaesu(YaesuModel::Ftdx5000),
        Self::Yaesu(YaesuModel::Ftdx9000),
        Self::Yaesu(YaesuModel::Ftdx9000Old),
        Self::Yaesu(YaesuModel::Ft991),
        Self::Yaesu(YaesuModel::Ft891),
        Self::Yaesu(YaesuModel::Ft710),
        Self::Yaesu(YaesuModel::Ftdx10),
        Self::Yaesu(YaesuModel::Ftdx101d),
        Self::Yaesu(YaesuModel::Ftdx101mp),
        Self::FlexNative(FlexNativeModel::SliceA),
        Self::FlexNative(FlexNativeModel::SliceB),
        Self::FlexNative(FlexNativeModel::SliceC),
        Self::FlexNative(FlexNativeModel::SliceD),
        Self::FlexNative(FlexNativeModel::SliceE),
        Self::FlexNative(FlexNativeModel::SliceF),
        Self::FlexNative(FlexNativeModel::SliceG),
        Self::FlexNative(FlexNativeModel::SliceH),
    ];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kenwood(model) => model.as_str(),
            Self::Icom(model) => model.as_str(),
            Self::Yaesu(model) => model.as_str(),
            Self::FlexNative(model) => model.as_str(),
        }
    }

    fn kenwood_model(self) -> Option<KenwoodModel> {
        match self {
            Self::Kenwood(model) => Some(model),
            Self::Icom(_) | Self::Yaesu(_) | Self::FlexNative(_) => None,
        }
    }
}

impl FromStr for RadioKind {
    type Err = RadioError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        if let Some(model) = KenwoodModel::from_alias(value) {
            return Ok(Self::Kenwood(model));
        }

        if let Some(model) = IcomModel::from_alias(value) {
            return Ok(Self::Icom(model));
        }

        if let Some(model) = YaesuModel::from_alias(value) {
            return Ok(Self::Yaesu(model));
        }

        if let Some(model) = FlexNativeModel::from_alias(value) {
            return Ok(Self::FlexNative(model));
        }

        Err(RadioError::UnknownRadio(value.to_string()))
    }
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

    if let Some(model) = kind.kenwood_model() {
        return Ok(Box::new(
            KenwoodRadio::connect(connection, model.profile()).await?,
        ));
    }

    match kind {
        RadioKind::Icom(model) => Ok(Box::new(
            IcomCivRadio::connect(connection, model, &parsed_options).await?,
        )),
        RadioKind::Yaesu(model) => Ok(Box::new(
            YaesuNewCatRadio::connect(connection, model, &parsed_options).await?,
        )),
        RadioKind::FlexNative(model) => Ok(Box::new(
            FlexNativeRadio::connect(connection, model, &parsed_options).await?,
        )),
        _ => {
            unreachable!("all non-Icom/Yaesu/Flex-native kinds are mapped through kenwood_model")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{supported_radio_kinds, RadioKind};
    use crate::{FlexNativeModel, IcomModel, KenwoodModel, YaesuModel};

    #[test]
    fn lists_supported_radio_kinds() {
        let kinds = supported_radio_kinds();
        assert!(kinds.contains(&RadioKind::Kenwood(KenwoodModel::Ts590s)));
        assert!(kinds.contains(&RadioKind::Icom(IcomModel::Ic7300)));
        assert!(kinds.contains(&RadioKind::Icom(IcomModel::Ic7610)));
        assert!(kinds.contains(&RadioKind::Yaesu(YaesuModel::Ft991)));
        assert!(kinds.contains(&RadioKind::Yaesu(YaesuModel::Ftdx101mp)));
        assert!(kinds.contains(&RadioKind::FlexNative(FlexNativeModel::SliceA)));
        assert!(kinds.contains(&RadioKind::FlexNative(FlexNativeModel::SliceH)));
    }

    #[test]
    fn parses_protocol_aliases() {
        for (alias, expected) in [
            ("ts-590sg", RadioKind::Kenwood(KenwoodModel::Ts590sg)),
            ("k4", RadioKind::Kenwood(KenwoodModel::K4)),
            ("6xxx", RadioKind::Kenwood(KenwoodModel::Flex6xxx)),
            ("ic-7300", RadioKind::Icom(IcomModel::Ic7300)),
            ("IC7610", RadioKind::Icom(IcomModel::Ic7610)),
            ("ic-706mkiig", RadioKind::Icom(IcomModel::Ic706Mkiig)),
            ("x108g", RadioKind::Icom(IcomModel::X108g)),
            ("x6100", RadioKind::Icom(IcomModel::X6100)),
            ("x6200", RadioKind::Icom(IcomModel::X6200)),
            ("g90", RadioKind::Icom(IcomModel::G90)),
            ("x5105", RadioKind::Icom(IcomModel::X5105)),
            ("ft-991", RadioKind::Yaesu(YaesuModel::Ft991)),
            ("ftdx101mp", RadioKind::Yaesu(YaesuModel::Ftdx101mp)),
            ("ftdx-9000-old", RadioKind::Yaesu(YaesuModel::Ftdx9000Old)),
            (
                "smartsdr-slice-a",
                RadioKind::FlexNative(FlexNativeModel::SliceA),
            ),
            (
                "smartsdr-slice-h (native)",
                RadioKind::FlexNative(FlexNativeModel::SliceH),
            ),
            (
                "flex-6xxx (kenwood compat.)",
                RadioKind::Kenwood(KenwoodModel::Flex6xxx),
            ),
        ] {
            assert_eq!(alias.parse::<RadioKind>().unwrap(), expected);
        }
    }
}
