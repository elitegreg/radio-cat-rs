mod dummy;
mod kenwood_ascii;

pub use dummy::{DummyRadioDriver, DUMMY_DRIVER};
pub use kenwood_ascii::KenwoodAsciiDriver;

use std::sync::OnceLock;

use crate::{protocol::kenwood_ascii::SUPPORTED_PROFILES, DriverDescriptor};

static SUPPORTED_DRIVERS: OnceLock<Vec<DriverDescriptor>> = OnceLock::new();

pub fn supported_drivers() -> &'static [DriverDescriptor] {
    SUPPORTED_DRIVERS
        .get_or_init(|| {
            let mut descriptors = Vec::with_capacity(1 + SUPPORTED_PROFILES.len());
            descriptors.push(DUMMY_DRIVER);
            descriptors.extend(SUPPORTED_PROFILES.iter().map(|profile| profile.descriptor));
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
