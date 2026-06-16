use std::time::Duration;

use super::AsciiFrame;

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
    pub fn frame(self) -> AsciiFrame {
        match self {
            Self::AutoInfo(frame) | Self::Query(frame) => {
                AsciiFrame::new(frame).expect("startup metadata must contain valid ASCII frames")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PollPlan {
    pub interval: Duration,
    pub queries: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KenwoodAsciiProfile {
    pub id: &'static str,
    pub display_name: &'static str,
    pub brand: Brand,
    pub frequency_format: FrequencyFormat,
    pub startup: &'static [StartupStep],
    pub poll: Option<PollPlan>,
}

const TS590_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD;"),
];

const TS890_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("SF0;"),
    StartupStep::Query("SF1;"),
];

const TS990_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("OM0;"),
    StartupStep::Query("OM1;"),
];

const TS2000_FAMILY_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD;"),
];

const IF232_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI1;"),
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
];

const ELECRAFT_K4_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::AutoInfo("AID250;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD;"),
];

const ELECRAFT_K3_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD;"),
];

const ELECRAFT_K2_STARTUP: &[StartupStep] = &[
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD;"),
];

const YAESU_DUAL_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD0;"),
];

const YAESU_FTDX101_STARTUP: &[StartupStep] = &[
    StartupStep::AutoInfo("AI2;"),
    StartupStep::Query("IF;"),
    StartupStep::Query("FA;"),
    StartupStep::Query("FB;"),
    StartupStep::Query("MD0;"),
    StartupStep::Query("MD1;"),
];

const SLOW_POLL: PollPlan = PollPlan {
    interval: Duration::from_secs(15),
    queries: &["PC;", "KS;"],
};

pub const SUPPORTED_PROFILES: &[KenwoodAsciiProfile] = &[
    KenwoodAsciiProfile {
        id: "kenwood-ts590",
        display_name: "Kenwood TS-590",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS590_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-ts890",
        display_name: "Kenwood TS-890",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS890_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-ts990",
        display_name: "Kenwood TS-990",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS990_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-ts2000",
        display_name: "Kenwood TS-2000",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS2000_FAMILY_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-ts480",
        display_name: "Kenwood TS-480",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS2000_FAMILY_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-ts570",
        display_name: "Kenwood TS-570",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS2000_FAMILY_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-ts870",
        display_name: "Kenwood TS-870",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: TS2000_FAMILY_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "kenwood-if232",
        display_name: "Kenwood IF-232 Protocol",
        brand: Brand::Kenwood,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: IF232_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "elecraft-k4",
        display_name: "Elecraft K4",
        brand: Brand::Elecraft,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: ELECRAFT_K4_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "elecraft-k3",
        display_name: "Elecraft K3 Family",
        brand: Brand::Elecraft,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: ELECRAFT_K3_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "elecraft-k2",
        display_name: "Elecraft K2",
        brand: Brand::Elecraft,
        frequency_format: FrequencyFormat::Hertz11Digit,
        startup: ELECRAFT_K2_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "yaesu-ftdx101",
        display_name: "Yaesu FTDX-101",
        brand: Brand::Yaesu,
        frequency_format: FrequencyFormat::Hertz9Digit,
        startup: YAESU_FTDX101_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "yaesu-ftdx10",
        display_name: "Yaesu FTDX-10",
        brand: Brand::Yaesu,
        frequency_format: FrequencyFormat::Hertz9Digit,
        startup: YAESU_DUAL_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "yaesu-ft710",
        display_name: "Yaesu FT-710",
        brand: Brand::Yaesu,
        frequency_format: FrequencyFormat::Hertz9Digit,
        startup: YAESU_DUAL_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "yaesu-ft891",
        display_name: "Yaesu FT-891",
        brand: Brand::Yaesu,
        frequency_format: FrequencyFormat::Hertz9Digit,
        startup: YAESU_DUAL_STARTUP,
        poll: Some(SLOW_POLL),
    },
    KenwoodAsciiProfile {
        id: "yaesu-ft991",
        display_name: "Yaesu FT-991",
        brand: Brand::Yaesu,
        frequency_format: FrequencyFormat::Hertz9Digit,
        startup: YAESU_DUAL_STARTUP,
        poll: Some(SLOW_POLL),
    },
];

pub fn profile_by_id(id: &str) -> Option<&'static KenwoodAsciiProfile> {
    SUPPORTED_PROFILES
        .iter()
        .find(|profile| profile.id.eq_ignore_ascii_case(id))
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
    fn startup_steps_expand_to_valid_frames() {
        let profile = profile_by_id("elecraft-k4").unwrap();
        let frames: Vec<_> = profile.startup.iter().map(|step| step.frame()).collect();

        assert_eq!(frames[0].as_str(), "AI2;");
        assert_eq!(frames[1].as_str(), "AID250;");
        assert!(frames.iter().all(|frame| frame.as_str().ends_with(';')));
    }
}
