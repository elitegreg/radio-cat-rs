mod dummy;

use std::sync::OnceLock;

use crate::{
    driver::{DriverDescriptor, RadioSession, TransportRequirement},
    error::{RadioError, Result},
    protocol::{
        icom_civ as icom_protocol, kenwood_ascii as kenwood_protocol, runtime,
        smartsdr as smartsdr_protocol,
    },
    transport::TransportConfig,
    DataPttRelationship, Frequency, Mode, RadioRegion, ValueRange,
};

use dummy::{DummyRadioSession, DUMMY_DRIVER};

trait RadioSessionFactory: Sync {
    fn descriptors(&self) -> Vec<DriverDescriptor>;
    fn matches(&self, normalized_id: &str) -> bool;
    fn create(&self, normalized_id: &str, options: DriverOptions) -> Result<Box<dyn RadioSession>>;
}

/// Parsed exactly once by the registry, before a transport is opened.
///
/// `RadioConfig` intentionally keeps its convenient string input, while
/// sessions only ever receive their protocol's typed options.
#[derive(Debug, Clone)]
enum DriverOptions {
    Dummy(String),
    Kenwood(kenwood_protocol::KenwoodAsciiOptions),
    Icom(icom_protocol::IcomCivOptions),
    SmartSdr(smartsdr_protocol::SmartSdrOptions),
}

struct DummyFactory;
struct KenwoodFactory;
struct IcomFactory;
struct SmartSdrFactory;

impl RadioSessionFactory for DummyFactory {
    fn descriptors(&self) -> Vec<DriverDescriptor> {
        vec![DUMMY_DRIVER]
    }

    fn matches(&self, normalized_id: &str) -> bool {
        normalized_id == DUMMY_DRIVER.id
    }

    fn create(
        &self,
        _normalized_id: &str,
        options: DriverOptions,
    ) -> Result<Box<dyn RadioSession>> {
        let DriverOptions::Dummy(options) = options else {
            unreachable!("dummy factory received another driver's options")
        };
        Ok(Box::new(DummyRadioSession::with_options(options)))
    }
}

impl RadioSessionFactory for KenwoodFactory {
    fn descriptors(&self) -> Vec<DriverDescriptor> {
        kenwood_protocol::SUPPORTED_PROFILES
            .iter()
            .map(|profile| profile.descriptor)
            .collect()
    }

    fn matches(&self, normalized_id: &str) -> bool {
        kenwood_protocol::profile_by_id(normalized_id).is_some()
    }

    fn create(&self, normalized_id: &str, options: DriverOptions) -> Result<Box<dyn RadioSession>> {
        let DriverOptions::Kenwood(options) = options else {
            unreachable!("Kenwood factory received another driver's options")
        };
        runtime::kenwood_session(
            kenwood_protocol::profile_by_id(normalized_id).expect("factory matched profile"),
            options,
        )
    }
}

impl RadioSessionFactory for IcomFactory {
    fn descriptors(&self) -> Vec<DriverDescriptor> {
        icom_protocol::SUPPORTED_PROFILES
            .iter()
            .map(|profile| profile.descriptor)
            .collect()
    }

    fn matches(&self, normalized_id: &str) -> bool {
        icom_protocol::profile_by_id(normalized_id).is_some()
    }

    fn create(&self, normalized_id: &str, options: DriverOptions) -> Result<Box<dyn RadioSession>> {
        let DriverOptions::Icom(options) = options else {
            unreachable!("ICOM factory received another driver's options")
        };
        runtime::icom_session(
            icom_protocol::profile_by_id(normalized_id).expect("factory matched profile"),
            options,
        )
    }
}

impl RadioSessionFactory for SmartSdrFactory {
    fn descriptors(&self) -> Vec<DriverDescriptor> {
        smartsdr_protocol::SUPPORTED_PROFILES
            .iter()
            .map(|profile| profile.descriptor)
            .collect()
    }

    fn matches(&self, normalized_id: &str) -> bool {
        smartsdr_protocol::profile_by_id(normalized_id).is_some()
    }

    fn create(&self, normalized_id: &str, options: DriverOptions) -> Result<Box<dyn RadioSession>> {
        let DriverOptions::SmartSdr(options) = options else {
            unreachable!("SmartSDR factory received another driver's options")
        };
        runtime::smartsdr_session(
            smartsdr_protocol::profile_by_id(normalized_id).expect("factory matched profile"),
            options,
        )
    }
}

static DUMMY_FACTORY: DummyFactory = DummyFactory;
static KENWOOD_FACTORY: KenwoodFactory = KenwoodFactory;
static ICOM_FACTORY: IcomFactory = IcomFactory;
static SMARTSDR_FACTORY: SmartSdrFactory = SmartSdrFactory;

static FACTORIES: [&dyn RadioSessionFactory; 4] = [
    &DUMMY_FACTORY,
    &KENWOOD_FACTORY,
    &ICOM_FACTORY,
    &SMARTSDR_FACTORY,
];

static SUPPORTED_DRIVERS: OnceLock<Vec<DriverDescriptor>> = OnceLock::new();

pub(crate) fn supported_drivers() -> &'static [DriverDescriptor] {
    SUPPORTED_DRIVERS
        .get_or_init(|| {
            for profile in kenwood_protocol::SUPPORTED_PROFILES {
                runtime::validate_kenwood_profile(profile)
                    .expect("registered Kenwood profile query plan must be valid");
            }
            for profile in icom_protocol::SUPPORTED_PROFILES {
                runtime::validate_icom_profile(profile)
                    .expect("registered ICOM profile query plan must be valid");
            }
            let descriptors: Vec<_> = FACTORIES
                .iter()
                .flat_map(|factory| factory.descriptors())
                .collect();
            for descriptor in &descriptors {
                if descriptor.supported_regions().is_empty() {
                    capabilities_for(descriptor.id, None)
                        .expect("registered profile capabilities must be valid");
                } else {
                    for region in descriptor.supported_regions() {
                        capabilities_for(descriptor.id, Some(*region))
                            .expect("registered regional profile capabilities must be valid");
                    }
                }
            }
            descriptors
        })
        .as_slice()
}

pub(crate) fn capabilities_for(
    id: &str,
    region: Option<RadioRegion>,
) -> Result<crate::RadioCapabilities> {
    if id == DUMMY_DRIVER.id {
        return Ok(crate::RadioCapabilities::dummy_all());
    }

    let mut capabilities = kenwood_protocol::profile_by_id(id)
        .map(|profile| profile.capabilities)
        .or_else(|| icom_protocol::profile_by_id(id).map(|profile| profile.capabilities))
        .or_else(|| smartsdr_protocol::profile_by_id(id).map(|profile| profile.capabilities))
        .ok_or_else(|| RadioError::UnsupportedDriver {
            driver: id.to_string(),
        })?;
    if (kenwood_protocol::profile_by_id(id).is_some() || icom_protocol::profile_by_id(id).is_some())
        && region.is_none()
    {
        return Err(RadioError::InvalidValue {
            field: "region",
            message: "an IARU region is required for this physical radio profile".to_string(),
        });
    }
    let rx_ranges = receive_ranges(id);
    capabilities.main_rx.frequency_ranges = rx_ranges;
    if let Some(sub_rx) = &mut capabilities.sub_rx {
        sub_rx.frequency_ranges = rx_ranges;
    }
    capabilities.main_rx.modes = supported_modes(id);
    if let Some(sub_rx) = &mut capabilities.sub_rx {
        sub_rx.modes = supported_modes(id);
    }
    if let Some(tx) = &mut capabilities.tx {
        tx.modes = supported_modes(id);
        if matches!(
            id,
            "kenwood-ts590"
                | "kenwood-ts890"
                | "kenwood-ts990"
                | "kenwood-ts480"
                | "kenwood-ts2000"
        ) {
            tx.data_ptt_relationship = DataPttRelationship::Distinct;
        }
        if let Some(region) = region {
            tx.frequency_ranges = transmit_ranges(id, region);
        }
    }
    if let Some(profile) = icom_protocol::profile_by_id(id) {
        capabilities.main_rx.rf.attenuator_values = profile.attenuator_values_db;
        if let Some(sub_rx) = &mut capabilities.sub_rx
            && sub_rx.rf.attenuator.is_supported()
        {
            sub_rx.rf.attenuator_values = profile.attenuator_values_db;
        }
    }
    if let Some(keyer) = &mut capabilities.keyer {
        keyer.speed_range_wpm = Some(keyer_range(id));
    }
    capabilities.validate()?;
    Ok(capabilities)
}

fn keyer_range(id: &str) -> ValueRange<u8> {
    let (min, max) = match id {
        "kenwood-ts2000" | "kenwood-ts480" | "kenwood-ts570" | "kenwood-ts870" => (10, 60),
        "elecraft-k4" => (8, 100),
        "elecraft-k3" => (8, 50),
        "elecraft-k2" => (9, 50),
        _ => (4, 60),
    };
    ValueRange::new(min, max, 1)
}

const HF_RX: &[ValueRange<Frequency>] = &[ValueRange::new(
    Frequency::from_hz(30_000),
    Frequency::from_hz(60_000_000),
    Frequency::from_hz(1),
)];
const HF_VU_RX: &[ValueRange<Frequency>] = &[
    ValueRange::new(
        Frequency::from_hz(30_000),
        Frequency::from_hz(200_000_000),
        Frequency::from_hz(1),
    ),
    ValueRange::new(
        Frequency::from_hz(400_000_000),
        Frequency::from_hz(470_000_000),
        Frequency::from_hz(1),
    ),
];
const TS2000_RX: &[ValueRange<Frequency>] = &[
    ValueRange::new(
        Frequency::from_hz(30_000),
        Frequency::from_hz(60_000_000),
        Frequency::from_hz(1),
    ),
    ValueRange::new(
        Frequency::from_hz(118_000_000),
        Frequency::from_hz(174_000_000),
        Frequency::from_hz(1),
    ),
    ValueRange::new(
        Frequency::from_hz(220_000_000),
        Frequency::from_hz(512_000_000),
        Frequency::from_hz(1),
    ),
    ValueRange::new(
        Frequency::from_hz(1_240_000_000),
        Frequency::from_hz(1_300_000_000),
        Frequency::from_hz(1),
    ),
];

const HF_R1: &[ValueRange<Frequency>] = &[
    mhz_range(1_800_000, 2_000_000),
    mhz_range(3_500_000, 3_800_000),
    mhz_range(5_351_500, 5_366_500),
    mhz_range(7_000_000, 7_200_000),
    mhz_range(10_100_000, 10_150_000),
    mhz_range(14_000_000, 14_350_000),
    mhz_range(18_068_000, 18_168_000),
    mhz_range(21_000_000, 21_450_000),
    mhz_range(24_890_000, 24_990_000),
    mhz_range(28_000_000, 29_700_000),
    mhz_range(50_000_000, 52_000_000),
];
const HF_R2: &[ValueRange<Frequency>] = &[
    mhz_range(1_800_000, 2_000_000),
    mhz_range(3_500_000, 4_000_000),
    mhz_range(5_330_500, 5_406_500),
    mhz_range(7_000_000, 7_300_000),
    mhz_range(10_100_000, 10_150_000),
    mhz_range(14_000_000, 14_350_000),
    mhz_range(18_068_000, 18_168_000),
    mhz_range(21_000_000, 21_450_000),
    mhz_range(24_890_000, 24_990_000),
    mhz_range(28_000_000, 29_700_000),
    mhz_range(50_000_000, 54_000_000),
];
const HF_R3: &[ValueRange<Frequency>] = &[
    mhz_range(1_800_000, 2_000_000),
    mhz_range(3_500_000, 3_900_000),
    mhz_range(5_351_500, 5_366_500),
    mhz_range(7_000_000, 7_300_000),
    mhz_range(10_100_000, 10_150_000),
    mhz_range(14_000_000, 14_350_000),
    mhz_range(18_068_000, 18_168_000),
    mhz_range(21_000_000, 21_450_000),
    mhz_range(24_890_000, 24_990_000),
    mhz_range(28_000_000, 29_700_000),
    mhz_range(50_000_000, 54_000_000),
];
const HF_VU_R1: &[ValueRange<Frequency>] = &[
    HF_R1[0],
    HF_R1[1],
    HF_R1[2],
    HF_R1[3],
    HF_R1[4],
    HF_R1[5],
    HF_R1[6],
    HF_R1[7],
    HF_R1[8],
    HF_R1[9],
    HF_R1[10],
    mhz_range(144_000_000, 146_000_000),
    mhz_range(430_000_000, 440_000_000),
];
const HF_VU_R2: &[ValueRange<Frequency>] = &[
    HF_R2[0],
    HF_R2[1],
    HF_R2[2],
    HF_R2[3],
    HF_R2[4],
    HF_R2[5],
    HF_R2[6],
    HF_R2[7],
    HF_R2[8],
    HF_R2[9],
    HF_R2[10],
    mhz_range(144_000_000, 148_000_000),
    mhz_range(420_000_000, 450_000_000),
];
const HF_VU_R3: &[ValueRange<Frequency>] = &[
    HF_R3[0],
    HF_R3[1],
    HF_R3[2],
    HF_R3[3],
    HF_R3[4],
    HF_R3[5],
    HF_R3[6],
    HF_R3[7],
    HF_R3[8],
    HF_R3[9],
    HF_R3[10],
    mhz_range(144_000_000, 148_000_000),
    mhz_range(430_000_000, 450_000_000),
];

const fn mhz_range(min: u64, max: u64) -> ValueRange<Frequency> {
    ValueRange::new(
        Frequency::from_hz(min),
        Frequency::from_hz(max),
        Frequency::from_hz(1),
    )
}

const COMMON_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
    Mode::DataAm,
];
const BASIC_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
];
const K2_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Rtty,
    Mode::RttyReverse,
];
const EXTENDED_KENWOOD_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::Psk,
    Mode::PskReverse,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
    Mode::DataAm,
];
const ELECRAFT_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::Psk,
    Mode::PskReverse,
    Mode::DataLsb,
    Mode::DataUsb,
];
const YAESU_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::Psk,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
];
const YAESU_FT891_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::Psk,
    Mode::DataLsb,
    Mode::DataUsb,
];
const YAESU_FT991_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::Psk,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
    Mode::DigitalVoice,
];
const ICOM_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
    Mode::DataAm,
];
const ICOM_VOICE_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Wfm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
    Mode::DataAm,
    Mode::DigitalVoice,
];
const ICOM_PSK_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::CwReverse,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::RttyReverse,
    Mode::Psk,
    Mode::PskReverse,
    Mode::DataLsb,
    Mode::DataUsb,
    Mode::DataFm,
    Mode::DataAm,
];
const SMARTSDR_MODES: &[Mode] = &[
    Mode::Lsb,
    Mode::Usb,
    Mode::Cw,
    Mode::Am,
    Mode::Fm,
    Mode::Rtty,
    Mode::DataLsb,
    Mode::DataUsb,
];

fn receive_ranges(id: &str) -> &'static [ValueRange<Frequency>] {
    match id {
        "kenwood-ts2000" => TS2000_RX,
        "yaesu-ft991" | "icom-ic705" | "icom-ic7100" => HF_VU_RX,
        _ => HF_RX,
    }
}

fn transmit_ranges(id: &str, region: RadioRegion) -> &'static [ValueRange<Frequency>] {
    let vhf_uhf = matches!(
        id,
        "kenwood-ts2000" | "yaesu-ft991" | "icom-ic705" | "icom-ic7100"
    );
    match (region, vhf_uhf) {
        (RadioRegion::IaruRegion1, false) => HF_R1,
        (RadioRegion::IaruRegion2, false) => HF_R2,
        (RadioRegion::IaruRegion3, false) => HF_R3,
        (RadioRegion::IaruRegion1, true) => HF_VU_R1,
        (RadioRegion::IaruRegion2, true) => HF_VU_R2,
        (RadioRegion::IaruRegion3, true) => HF_VU_R3,
    }
}

fn supported_modes(id: &str) -> &'static [Mode] {
    match id {
        "kenwood-ts590" => COMMON_MODES,
        "kenwood-ts990" => EXTENDED_KENWOOD_MODES,
        "kenwood-ts890" | "kenwood-ts2000" | "kenwood-ts480" | "kenwood-ts570"
        | "kenwood-ts870" | "kenwood-if232" => BASIC_MODES,
        "elecraft-k4" | "elecraft-k3" => ELECRAFT_MODES,
        "elecraft-k2" => K2_MODES,
        "yaesu-ft891" => YAESU_FT891_MODES,
        "yaesu-ft991" => YAESU_FT991_MODES,
        "yaesu-ftdx101" | "yaesu-ftdx10" | "yaesu-ft710" => YAESU_MODES,
        "flexradio-smartsdr" => SMARTSDR_MODES,
        "icom-ic705" | "icom-ic7100" => ICOM_VOICE_MODES,
        "icom-ic7610" | "icom-ic7760" => ICOM_PSK_MODES,
        "icom-ic7300" => ICOM_MODES,
        _ => COMMON_MODES,
    }
}

pub(crate) fn create_session(
    requested_id: &str,
    region: Option<RadioRegion>,
    options: &str,
    transport: &TransportConfig,
    caller_provided_transport: bool,
) -> Result<Box<dyn RadioSession>> {
    let normalized_id = requested_id.trim().to_ascii_lowercase();
    let Some(factory) = FACTORIES
        .iter()
        .find(|factory| factory.matches(&normalized_id))
    else {
        return Err(RadioError::UnsupportedDriver {
            driver: requested_id.to_string(),
        });
    };

    let descriptor = supported_drivers()
        .iter()
        .find(|descriptor| descriptor.id == normalized_id)
        .expect("factory match always has a supported descriptor");
    validate_transport(
        descriptor.transport_requirement,
        transport,
        caller_provided_transport,
    )?;
    capabilities_for(descriptor.id, region)?;
    let options = parse_options(&normalized_id, options)?;
    factory.create(&normalized_id, options)
}

fn parse_options(id: &str, options: &str) -> Result<DriverOptions> {
    if id == DUMMY_DRIVER.id {
        return Ok(DriverOptions::Dummy(options.to_owned()));
    }
    if kenwood_protocol::profile_by_id(id).is_some() {
        return Ok(DriverOptions::Kenwood(
            kenwood_protocol::KenwoodAsciiOptions::parse(options)?,
        ));
    }
    if let Some(profile) = icom_protocol::profile_by_id(id) {
        return Ok(DriverOptions::Icom(icom_protocol::IcomCivOptions::parse(
            profile, options,
        )?));
    }
    if let Some(profile) = smartsdr_protocol::profile_by_id(id) {
        return Ok(DriverOptions::SmartSdr(
            smartsdr_protocol::SmartSdrOptions::parse(profile, options)?,
        ));
    }
    Err(RadioError::UnsupportedDriver {
        driver: id.to_owned(),
    })
}

fn validate_transport(
    requirement: TransportRequirement,
    config: &TransportConfig,
    caller_provided: bool,
) -> Result<()> {
    if caller_provided || matches!(requirement, TransportRequirement::None) {
        return Ok(());
    }

    let valid = match requirement {
        TransportRequirement::None => true,
        TransportRequirement::SerialOrTcp => {
            matches!(
                config,
                TransportConfig::Serial { .. } | TransportConfig::Tcp { .. }
            )
        }
        TransportRequirement::Tcp => matches!(config, TransportConfig::Tcp { .. }),
    };
    if valid {
        return Ok(());
    }

    let message = match requirement {
        TransportRequirement::None => unreachable!(),
        TransportRequirement::SerialOrTcp => {
            "this physical radio driver requires a serial or TCP transport configuration"
        }
        TransportRequirement::Tcp => "this radio driver requires a TCP transport configuration",
    };
    Err(RadioError::InvalidValue {
        field: "transport",
        message: message.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_ids_case_insensitively() {
        let session = create_session(
            "  ICOM-IC705 ",
            Some(RadioRegion::IaruRegion2),
            "",
            &TransportConfig::serial("ignored", 9_600),
            false,
        )
        .unwrap();
        assert_eq!(session.descriptor().id, "icom-ic705");
    }

    #[test]
    fn registry_validates_all_profiles_for_all_advertised_regions() {
        for descriptor in supported_drivers() {
            if descriptor.supported_regions().is_empty() {
                descriptor.capabilities(None).unwrap().validate().unwrap();
            } else {
                for region in descriptor.supported_regions() {
                    let capabilities = descriptor.capabilities(Some(*region)).unwrap();
                    capabilities.validate().unwrap();
                    assert!(!capabilities.main_rx.modes.is_empty());
                    assert!(!capabilities.main_rx.frequency_ranges.is_empty());
                }
            }
        }
    }

    #[test]
    fn physical_profiles_require_an_iaru_region() {
        let descriptor = supported_drivers()
            .iter()
            .find(|descriptor| descriptor.id == "kenwood-ts590")
            .unwrap();
        assert!(descriptor.capabilities(None).is_err());
        assert_eq!(descriptor.supported_regions(), RadioRegion::ALL);
    }

    #[test]
    fn regional_hardware_ranges_are_selected_explicitly() {
        let descriptor = supported_drivers()
            .iter()
            .find(|descriptor| descriptor.id == "kenwood-ts590")
            .unwrap();
        let region1 = descriptor
            .capabilities(Some(RadioRegion::IaruRegion1))
            .unwrap();
        let region2 = descriptor
            .capabilities(Some(RadioRegion::IaruRegion2))
            .unwrap();
        assert_eq!(
            region1.tx.unwrap().frequency_ranges.last().unwrap().max,
            Frequency::from_hz(52_000_000)
        );
        assert_eq!(
            region2.tx.unwrap().frequency_ranges.last().unwrap().max,
            Frequency::from_hz(54_000_000)
        );
    }

    #[test]
    fn advertised_modes_are_accepted_by_each_profile_codec() {
        let state = crate::RadioState::default();
        for descriptor in supported_drivers() {
            if descriptor.id == "dummy" {
                continue;
            }
            let region = descriptor.supported_regions().first().copied();
            let capabilities = descriptor.capabilities(region).unwrap();
            for mode in capabilities.main_rx.modes {
                let command = crate::RadioCommand::SetReceiverMode {
                    receiver: crate::ReceiverPath::Main,
                    mode: *mode,
                };
                let result = if let Some(profile) = kenwood_protocol::profile_by_id(descriptor.id) {
                    kenwood_protocol::mode::encode(profile, &command, &state)
                        .map(|value| value.is_some())
                } else if let Some(profile) = icom_protocol::profile_by_id(descriptor.id) {
                    let options = icom_protocol::IcomCivOptions::defaults(profile);
                    icom_protocol::encode(profile, options, &command, &state)
                        .map(|value| value.is_some())
                } else if let Some(profile) = smartsdr_protocol::profile_by_id(descriptor.id) {
                    smartsdr_protocol::encode(profile, &command, &state)
                        .map(|value| value.is_some())
                } else {
                    unreachable!()
                };
                assert!(
                    result.is_ok() && result.unwrap(),
                    "{} advertises unsupported {mode}",
                    descriptor.id
                );
            }
        }
    }

    #[test]
    fn registry_rejects_unknown_ids() {
        assert!(matches!(
            create_session("unknown", None, "", &TransportConfig::None, false),
            Err(RadioError::UnsupportedDriver { .. })
        ));
    }

    #[test]
    fn physical_drivers_require_configured_transports() {
        assert!(create_session(
            "kenwood-ts590",
            Some(RadioRegion::IaruRegion2),
            "",
            &TransportConfig::None,
            false
        )
        .is_err());
        assert!(create_session(
            "icom-ic705",
            Some(RadioRegion::IaruRegion2),
            "",
            &TransportConfig::None,
            false
        )
        .is_err());
        assert!(create_session(
            "flexradio-smartsdr",
            None,
            "",
            &TransportConfig::serial("ignored", 9_600),
            false,
        )
        .is_err());
        assert!(create_session(
            "  flexradio-smartsdr  ",
            None,
            "",
            &TransportConfig::None,
            false,
        )
        .is_err());
        assert!(
            create_session("flexradio-smartsdr", None, "", &TransportConfig::None, true,).is_ok()
        );
    }
}
