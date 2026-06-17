mod dummy;
mod icom_civ;
mod kenwood_ascii;

pub use dummy::{DummyRadioDriver, DUMMY_DRIVER};
pub use icom_civ::IcomCivDriver;
pub use kenwood_ascii::KenwoodAsciiDriver;

use std::sync::OnceLock;

use crate::{
    protocol::{icom_civ as icom_protocol, kenwood_ascii as kenwood_protocol},
    DriverDescriptor,
};

static SUPPORTED_DRIVERS: OnceLock<Vec<DriverDescriptor>> = OnceLock::new();

pub fn supported_drivers() -> &'static [DriverDescriptor] {
    SUPPORTED_DRIVERS
        .get_or_init(|| {
            let mut descriptors = Vec::with_capacity(
                1 + kenwood_protocol::SUPPORTED_PROFILES.len()
                    + icom_protocol::SUPPORTED_PROFILES.len(),
            );
            descriptors.push(DUMMY_DRIVER);
            descriptors.extend(
                kenwood_protocol::SUPPORTED_PROFILES
                    .iter()
                    .map(|profile| profile.descriptor),
            );
            descriptors.extend(
                icom_protocol::SUPPORTED_PROFILES
                    .iter()
                    .map(|profile| profile.descriptor),
            );
            descriptors
        })
        .as_slice()
}

pub fn driver_descriptor(id: &str) -> Option<DriverDescriptor> {
    supported_drivers()
        .iter()
        .copied()
        .find(|driver| driver.id.eq_ignore_ascii_case(id))
}
