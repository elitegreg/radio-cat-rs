mod dummy;

pub use dummy::{DummyRadioDriver, DUMMY_DRIVER};

use crate::DriverDescriptor;

pub fn supported_drivers() -> &'static [DriverDescriptor] {
    &[DUMMY_DRIVER]
}

pub fn driver_descriptor(id: &str) -> Option<DriverDescriptor> {
    supported_drivers()
        .iter()
        .copied()
        .find(|driver| driver.id.eq_ignore_ascii_case(id))
}
