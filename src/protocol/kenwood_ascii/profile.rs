use std::time::Duration;

use crate::{
    capabilities::{
        Capability, KeyerCapabilities, RadioCapabilities, ReceiverCapabilities, ReceiverKind,
        ReceiverRfCapabilities, RitXitCapabilities, RitXitOffsetType, StateUpdateCapability,
        TransmitterCapabilities,
    },
    driver::DriverDescriptor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Brand {
    Kenwood,
    Elecraft,
    Yaesu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrequencyFormat {
    Hertz9Digit,
    Hertz11Digit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupStep {
    AutoInfo(&'static str),
    Query(&'static str),
}

impl StartupStep {
    pub const fn label(self) -> &'static str {
        match self {
            Self::AutoInfo(label) | Self::Query(label) => label,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PollPlan {
    pub interval: Duration,
    pub queries: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KenwoodAsciiProfile {
    pub descriptor: DriverDescriptor,
    pub brand: Brand,
    pub receiver_kind: ReceiverKind,
    pub frequency_format: FrequencyFormat,
    pub capabilities: RadioCapabilities,
    pub update_strategy: StateUpdateCapability,
    pub startup: &'static [StartupStep],
    pub poll: Option<PollPlan>,
}

impl KenwoodAsciiProfile {
    pub const fn id(self) -> &'static str {
        self.descriptor.id
    }
}

const RW: Capability = Capability::ReadWrite;
const RO: Capability = Capability::ReadOnly;
const WO: Capability = Capability::WriteOnly;
const EMULATED: Capability = Capability::Emulated;
const UNSUPPORTED: Capability = Capability::Unsupported;

const FULL_RF: ReceiverRfCapabilities = ReceiverRfCapabilities::new(RW, RW, RW, RW, RW);
const NO_AUTO_NOTCH_RF: ReceiverRfCapabilities =
    ReceiverRfCapabilities::new(RW, RW, RW, RW, UNSUPPORTED);
const NB_ONLY_RF: ReceiverRfCapabilities =
    ReceiverRfCapabilities::new(RW, RW, RW, UNSUPPORTED, UNSUPPORTED);
const NO_RF: ReceiverRfCapabilities = ReceiverRfCapabilities::new(
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
    UNSUPPORTED,
);

const FULL_RX: ReceiverCapabilities = ReceiverCapabilities::new(RW, RW, RW, RW, FULL_RF);
const K3_RX: ReceiverCapabilities = ReceiverCapabilities::new(RW, RW, RW, RW, NB_ONLY_RF);
const NO_AUTO_NOTCH_RX: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, RW, RW, NO_AUTO_NOTCH_RF);
const NO_FILTER_NO_AUTO_NOTCH_RX: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, UNSUPPORTED, UNSUPPORTED, NO_AUTO_NOTCH_RF);
const BW_ONLY_RX: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, RW, UNSUPPORTED, NB_ONLY_RF);
const NO_FILTER_NO_RF_RX: ReceiverCapabilities =
    ReceiverCapabilities::new(RW, RW, UNSUPPORTED, UNSUPPORTED, NO_RF);

const FULL_TX: TransmitterCapabilities = TransmitterCapabilities::new(RW, RW, RW, RW, RW);
const IF232_TX: TransmitterCapabilities = TransmitterCapabilities::new(RW, RW, UNSUPPORTED, RW, RW);

const MAIN_RIT_XIT: RitXitCapabilities = RitXitCapabilities::new(
    RW,
    UNSUPPORTED,
    RW,
    RW,
    UNSUPPORTED,
    RitXitOffsetType::Shared,
);
const K4_RIT_XIT: RitXitCapabilities =
    RitXitCapabilities::new(RW, RW, RW, RW, RW, RitXitOffsetType::Shared);
const K2_RIT_XIT: RitXitCapabilities = RitXitCapabilities::new(
    RW,
    UNSUPPORTED,
    RW,
    RO,
    UNSUPPORTED,
    RitXitOffsetType::Shared,
);

const FULL_KEYER: KeyerCapabilities = KeyerCapabilities::new(RW, EMULATED, WO, WO);
const YAESU_KEYER: KeyerCapabilities =
    KeyerCapabilities::new(RW, UNSUPPORTED, UNSUPPORTED, UNSUPPORTED);

const HYBRID: StateUpdateCapability = StateUpdateCapability::Hybrid;

const IF232_POLL: Duration = Duration::from_secs(2);

const TS590_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FR"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("DA"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("filter-state"),
    StartupStep::Query("NT"),
    StartupStep::Query("NB"),
    StartupStep::Query("NR"),
    StartupStep::Query("PA"),
    StartupStep::Query("RA"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const TS890_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FR"),
    StartupStep::Query("FT"),
    StartupStep::Query("SF0"),
    StartupStep::Query("SF1"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("RF"),
    StartupStep::Query("filter-hi-lo"),
    StartupStep::Query("NT"),
    StartupStep::Query("NB1"),
    StartupStep::Query("NB2"),
    StartupStep::Query("NR"),
    StartupStep::Query("PA"),
    StartupStep::Query("RA"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const TS990_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("SP"),
    StartupStep::Query("OM0"),
    StartupStep::Query("OM1"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("RF"),
    StartupStep::Query("filter-hi-lo-main"),
    StartupStep::Query("filter-hi-lo-sub"),
    StartupStep::Query("NT0"),
    StartupStep::Query("NT1"),
    StartupStep::Query("NB10"),
    StartupStep::Query("NB11"),
    StartupStep::Query("NB20"),
    StartupStep::Query("NB21"),
    StartupStep::Query("NR0"),
    StartupStep::Query("NR1"),
    StartupStep::Query("PA0"),
    StartupStep::Query("PA1"),
    StartupStep::Query("RA0"),
    StartupStep::Query("RA1"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const TS2000_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FR"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("filter-state"),
    StartupStep::Query("NT"),
    StartupStep::Query("NB"),
    StartupStep::Query("NR"),
    StartupStep::Query("PA"),
    StartupStep::Query("RA"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const TS480_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FR"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("filter-state"),
    StartupStep::Query("NB"),
    StartupStep::Query("NR"),
    StartupStep::Query("PA"),
    StartupStep::Query("RA"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const TS570_TS870_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FR"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("NB"),
    StartupStep::Query("NR"),
    StartupStep::Query("PA"),
    StartupStep::Query("RA"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const IF232_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI1;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("SP"),
    StartupStep::Query("MD"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
];

const ELECRAFT_K4_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::AutoInfo("AID250;"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("DT"),
    StartupStep::Query("MD$"),
    StartupStep::Query("DT$"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("RO"),
    StartupStep::Query("RT$"),
    StartupStep::Query("RO$"),
    StartupStep::Query("BW"),
    StartupStep::Query("BW$"),
    StartupStep::Query("IS"),
    StartupStep::Query("IS$"),
    StartupStep::Query("NA"),
    StartupStep::Query("NA$"),
    StartupStep::Query("NB"),
    StartupStep::Query("NB$"),
    StartupStep::Query("NR"),
    StartupStep::Query("NR$"),
    StartupStep::Query("PA"),
    StartupStep::Query("PA$"),
    StartupStep::Query("RA"),
    StartupStep::Query("RA$"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const ELECRAFT_K3_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("DT"),
    StartupStep::Query("MD$"),
    StartupStep::Query("DT$"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("RO"),
    StartupStep::Query("BW"),
    StartupStep::Query("BW$"),
    StartupStep::Query("IS"),
    StartupStep::Query("IS$"),
    StartupStep::Query("NB"),
    StartupStep::Query("NB$"),
    StartupStep::Query("PA"),
    StartupStep::Query("PA$"),
    StartupStep::Query("RA"),
    StartupStep::Query("RA$"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const ELECRAFT_K2_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FR"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("FW"),
    StartupStep::Query("NB"),
    StartupStep::Query("PA"),
    StartupStep::Query("RA"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const YAESU_FTDX101_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI1;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD0"),
    StartupStep::Query("MD1"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("SH0"),
    StartupStep::Query("SH1"),
    StartupStep::Query("IS0"),
    StartupStep::Query("IS1"),
    StartupStep::Query("BC0"),
    StartupStep::Query("BC1"),
    StartupStep::Query("NB0"),
    StartupStep::Query("NB1"),
    StartupStep::Query("NR0"),
    StartupStep::Query("NR1"),
    StartupStep::Query("PA0"),
    StartupStep::Query("PA1"),
    StartupStep::Query("RA0"),
    StartupStep::Query("RA1"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const YAESU_FTDX10_FT710_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI1;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD0"),
    StartupStep::Query("MD1"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("SH0"),
    StartupStep::Query("IS0"),
    StartupStep::Query("BC0"),
    StartupStep::Query("NB0"),
    StartupStep::Query("NR0"),
    StartupStep::Query("PA0"),
    StartupStep::Query("RA0"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const YAESU_FT891_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI1;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("ST"),
    StartupStep::Query("MD0"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("NA0"),
    StartupStep::Query("SH0"),
    StartupStep::Query("IS0"),
    StartupStep::Query("BC0"),
    StartupStep::Query("NB0"),
    StartupStep::Query("NR0"),
    StartupStep::Query("PA0"),
    StartupStep::Query("RA0"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const YAESU_FT991_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI1;"),
    StartupStep::Query("IF"),
    StartupStep::Query("FA"),
    StartupStep::Query("FB"),
    StartupStep::Query("FT"),
    StartupStep::Query("MD0"),
    StartupStep::Query("RT"),
    StartupStep::Query("XT"),
    StartupStep::Query("NA0"),
    StartupStep::Query("SH0"),
    StartupStep::Query("IS0"),
    StartupStep::Query("BC0"),
    StartupStep::Query("NB0"),
    StartupStep::Query("NR0"),
    StartupStep::Query("PA0"),
    StartupStep::Query("RA0"),
    StartupStep::Query("PC"),
    StartupStep::Query("KS"),
];

const IF232_POLL_QUERIES: &[&str] = &["IF", "FA", "FB", "SP", "MD", "RT", "XT"];

const fn descriptor(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> DriverDescriptor {
    DriverDescriptor {
        id,
        display_name,
        description,
    }
}

const fn dual_capabilities(
    receiver_kind: ReceiverKind,
    rx: ReceiverCapabilities,
    tx: TransmitterCapabilities,
    rit_xit: RitXitCapabilities,
    keyer: Option<KeyerCapabilities>,
) -> RadioCapabilities {
    RadioCapabilities::new(
        receiver_kind,
        rx,
        Some(rx),
        Some(tx),
        rit_xit,
        keyer,
        HYBRID,
    )
}

pub const SUPPORTED_PROFILES: &[KenwoodAsciiProfile] = &[
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts590",
            "Kenwood TS-590",
            "Kenwood ASCII profile metadata for TS-590 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS590_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts890",
            "Kenwood TS-890",
            "Kenwood ASCII profile metadata for TS-890 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS890_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts990",
            "Kenwood TS-990",
            "Kenwood ASCII profile metadata for TS-990 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualRx,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualRx,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS990_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts2000",
            "Kenwood TS-2000",
            "Kenwood ASCII profile metadata for TS-2000 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS2000_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts480",
            "Kenwood TS-480",
            "Kenwood ASCII profile metadata for TS-480 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            NO_AUTO_NOTCH_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS480_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts570",
            "Kenwood TS-570",
            "Kenwood ASCII profile metadata for TS-570 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            NO_FILTER_NO_AUTO_NOTCH_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS570_TS870_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-ts870",
            "Kenwood TS-870",
            "Kenwood ASCII profile metadata for TS-870 radios.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            NO_FILTER_NO_AUTO_NOTCH_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: TS570_TS870_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "kenwood-if232",
            "Kenwood IF-232 Protocol",
            "Kenwood IF-232 protocol profile metadata.",
        ),
        brand: Brand::Kenwood,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            NO_FILTER_NO_RF_RX,
            IF232_TX,
            MAIN_RIT_XIT,
            None,
        ),
        update_strategy: HYBRID,
        startup: IF232_STARTUP,
        poll: Some(PollPlan {
            interval: IF232_POLL,
            queries: IF232_POLL_QUERIES,
        }),
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "elecraft-k4",
            "Elecraft K4",
            "Elecraft K4 shared Kenwood-ASCII profile metadata.",
        ),
        brand: Brand::Elecraft,
        receiver_kind: ReceiverKind::DualRx,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualRx,
            FULL_RX,
            FULL_TX,
            K4_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: ELECRAFT_K4_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "elecraft-k3",
            "Elecraft K3 Family",
            "Elecraft K3/KX profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Elecraft,
        receiver_kind: ReceiverKind::DualRx,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualRx,
            K3_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: ELECRAFT_K3_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "elecraft-k2",
            "Elecraft K2",
            "Elecraft K2 profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Elecraft,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz11Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            BW_ONLY_RX,
            FULL_TX,
            K2_RIT_XIT,
            Some(FULL_KEYER),
        ),
        update_strategy: HYBRID,
        startup: ELECRAFT_K2_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "yaesu-ftdx101",
            "Yaesu FTDX-101",
            "Yaesu FTDX-101 profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Yaesu,
        receiver_kind: ReceiverKind::DualRx,
        frequency_format: FrequencyFormat::Hertz9Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualRx,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(YAESU_KEYER),
        ),
        update_strategy: HYBRID,
        startup: YAESU_FTDX101_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "yaesu-ftdx10",
            "Yaesu FTDX-10",
            "Yaesu FTDX-10 profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Yaesu,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz9Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(YAESU_KEYER),
        ),
        update_strategy: HYBRID,
        startup: YAESU_FTDX10_FT710_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "yaesu-ft710",
            "Yaesu FT-710",
            "Yaesu FT-710 profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Yaesu,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz9Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(YAESU_KEYER),
        ),
        update_strategy: HYBRID,
        startup: YAESU_FTDX10_FT710_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "yaesu-ft891",
            "Yaesu FT-891",
            "Yaesu FT-891 profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Yaesu,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz9Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(YAESU_KEYER),
        ),
        update_strategy: HYBRID,
        startup: YAESU_FT891_STARTUP,
        poll: None,
    },
    KenwoodAsciiProfile {
        descriptor: descriptor(
            "yaesu-ft991",
            "Yaesu FT-991",
            "Yaesu FT-991 profile metadata on the shared Kenwood-ASCII engine.",
        ),
        brand: Brand::Yaesu,
        receiver_kind: ReceiverKind::DualVfo,
        frequency_format: FrequencyFormat::Hertz9Digit,
        capabilities: dual_capabilities(
            ReceiverKind::DualVfo,
            FULL_RX,
            FULL_TX,
            MAIN_RIT_XIT,
            Some(YAESU_KEYER),
        ),
        update_strategy: HYBRID,
        startup: YAESU_FT991_STARTUP,
        poll: None,
    },
];

pub fn profile_by_id(id: &str) -> Option<&'static KenwoodAsciiProfile> {
    SUPPORTED_PROFILES
        .iter()
        .find(|profile| profile.descriptor.id.eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_cover_the_initial_matrix() {
        assert_eq!(SUPPORTED_PROFILES.len(), 16);
        assert!(profile_by_id("kenwood-ts590").is_some());
        assert!(profile_by_id("ELECRAFT-K4").is_some());
        assert!(profile_by_id("yaesu-ft991").is_some());
    }

    #[test]
    fn metadata_contains_startup_and_poll_plans() {
        let profile = profile_by_id("elecraft-k4").unwrap();
        assert_eq!(profile.startup[0].label(), "AI2;");
        assert_eq!(profile.startup[1].label(), "AID250;");
        assert!(profile.startup.iter().any(|step| step.label() == "BW$"));
        assert!(profile.poll.is_none());
    }

    #[test]
    fn capabilities_follow_profile_caveats() {
        let if232 = profile_by_id("kenwood-if232").unwrap();
        assert_eq!(
            if232.capabilities.main_rx.filter_bandwidth,
            Capability::Unsupported
        );
        assert_eq!(
            if232.capabilities.tx.unwrap().power,
            Capability::Unsupported
        );
        assert!(if232.capabilities.keyer.is_none());

        let k2 = profile_by_id("elecraft-k2").unwrap();
        assert_eq!(
            k2.capabilities.main_rx.filter_shift,
            Capability::Unsupported
        );
        assert_eq!(k2.capabilities.rit_xit.offset, Capability::ReadOnly);
        assert_eq!(
            k2.capabilities.rit_xit.sub_rit_enabled,
            Capability::Unsupported
        );

        let k4 = profile_by_id("elecraft-k4").unwrap();
        assert_eq!(
            k4.capabilities.rit_xit.main_rit_enabled,
            Capability::ReadWrite
        );
        assert_eq!(
            k4.capabilities.rit_xit.sub_rit_enabled,
            Capability::ReadWrite
        );
        assert_eq!(
            k4.capabilities.rit_xit.offset_type,
            RitXitOffsetType::Shared
        );
        assert_eq!(k4.capabilities.keyer.unwrap().sending, Capability::Emulated);

        let yaesu = profile_by_id("yaesu-ftdx10").unwrap();
        let keyer = yaesu.capabilities.keyer.unwrap();
        assert_eq!(keyer.send_cw, Capability::Unsupported);
        assert_eq!(keyer.stop_cw, Capability::Unsupported);
        assert_eq!(keyer.speed_wpm, Capability::ReadWrite);
    }

    #[test]
    fn receiver_kind_and_update_strategy_match_capabilities_shape() {
        for profile in SUPPORTED_PROFILES {
            assert_eq!(profile.capabilities.state_updates, profile.update_strategy);
            match profile.receiver_kind {
                ReceiverKind::SingleVfo => assert!(profile.capabilities.sub_rx.is_none()),
                ReceiverKind::DualVfo | ReceiverKind::DualRx => {
                    assert!(profile.capabilities.sub_rx.is_some())
                }
            }
        }

        let ts990 = profile_by_id("kenwood-ts990").unwrap();
        assert_eq!(ts990.receiver_kind, ReceiverKind::DualRx);

        let if232 = profile_by_id("kenwood-if232").unwrap();
        assert_eq!(if232.poll.unwrap().interval, IF232_POLL);
    }

    #[test]
    fn yaesu_dual_vfo_startup_queries_md1_for_sub_vfo_mode() {
        for id in ["yaesu-ftdx101", "yaesu-ftdx10", "yaesu-ft710"] {
            let profile = profile_by_id(id).unwrap();
            assert!(
                profile
                    .startup
                    .iter()
                    .any(|step| matches!(step, StartupStep::Query("MD1"))),
                "{id} startup missing MD1 query"
            );
        }
    }
}
