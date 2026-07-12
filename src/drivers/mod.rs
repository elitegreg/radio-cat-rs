mod dummy;

use std::sync::OnceLock;

use crate::{
    driver::{DriverDescriptor, RadioSession},
    error::{RadioError, Result},
    protocol::{
        icom_civ as icom_protocol, kenwood_ascii as kenwood_protocol, runtime,
        smartsdr as smartsdr_protocol,
    },
    transport::TransportConfig,
};

use dummy::{DummyRadioSession, DUMMY_DRIVER};

trait RadioSessionFactory: Sync {
    fn descriptors(&self) -> Vec<DriverDescriptor>;
    fn matches(&self, normalized_id: &str) -> bool;
    fn create(&self, normalized_id: &str, options: &str) -> Result<Box<dyn RadioSession>>;

    fn validate_transport(&self, _config: &TransportConfig, _caller_provided: bool) -> Result<()> {
        Ok(())
    }
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

    fn create(&self, _normalized_id: &str, options: &str) -> Result<Box<dyn RadioSession>> {
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

    fn create(&self, normalized_id: &str, options: &str) -> Result<Box<dyn RadioSession>> {
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

    fn create(&self, normalized_id: &str, options: &str) -> Result<Box<dyn RadioSession>> {
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

    fn create(&self, normalized_id: &str, options: &str) -> Result<Box<dyn RadioSession>> {
        runtime::smartsdr_session(
            smartsdr_protocol::profile_by_id(normalized_id).expect("factory matched profile"),
            options,
        )
    }

    fn validate_transport(&self, config: &TransportConfig, caller_provided: bool) -> Result<()> {
        if caller_provided || matches!(config, TransportConfig::Tcp { .. }) {
            return Ok(());
        }

        let message = match config {
            TransportConfig::Serial { .. } => "flexradio-smartsdr supports TCP transport only",
            TransportConfig::None => "flexradio-smartsdr requires a TCP transport configuration",
            TransportConfig::Tcp { .. } => unreachable!(),
        };
        Err(RadioError::InvalidValue {
            field: "transport",
            message: message.to_string(),
        })
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
            FACTORIES
                .iter()
                .flat_map(|factory| factory.descriptors())
                .collect()
        })
        .as_slice()
}

pub(crate) fn create_session(
    requested_id: &str,
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

    factory.validate_transport(transport, caller_provided_transport)?;
    factory.create(&normalized_id, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_ids_case_insensitively() {
        let session = create_session("  ICOM-IC705 ", "", &TransportConfig::None, false).unwrap();
        assert_eq!(session.descriptor().id, "icom-ic705");
    }

    #[test]
    fn registry_rejects_unknown_ids() {
        assert!(matches!(
            create_session("unknown", "", &TransportConfig::None, false),
            Err(RadioError::UnsupportedDriver { .. })
        ));
    }

    #[test]
    fn smartsdr_transport_is_validated_by_its_factory() {
        assert!(create_session(
            "flexradio-smartsdr",
            "",
            &TransportConfig::serial("ignored", 9_600),
            false,
        )
        .is_err());
        assert!(create_session("flexradio-smartsdr", "", &TransportConfig::None, true,).is_ok());
    }
}
