use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frequency {
    hz: u64,
}

impl Frequency {
    pub const fn from_hz(hz: u64) -> Self {
        Self { hz }
    }

    pub const fn from_khz(khz: u64) -> Self {
        Self { hz: khz * 1_000 }
    }

    pub fn from_decimal_khz(khz: f64) -> Self {
        Self {
            hz: (khz * 1_000.0).round() as u64,
        }
    }

    pub fn from_decimal_mhz(mhz: f64) -> Self {
        Self {
            hz: (mhz * 1_000_000.0).round() as u64,
        }
    }

    pub const fn hz(&self) -> u64 {
        self.hz
    }

    pub const fn khz(&self) -> u64 {
        self.hz / 1_000
    }

    pub fn mhz(&self) -> f64 {
        self.hz as f64 / 1_000_000.0
    }
}

impl fmt::Display for Frequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hz.is_multiple_of(1_000_000) {
            write!(f, "{} MHz", self.hz / 1_000_000)
        } else if self.hz >= 1_000_000 {
            write!(f, "{:.3} MHz", self.mhz())
        } else if self.hz.is_multiple_of(1_000) {
            write!(f, "{} kHz", self.hz / 1_000)
        } else {
            write!(f, "{:.3} kHz", self.hz as f64 / 1_000.0)
        }
    }
}

#[macro_export]
macro_rules! khz {
    ($value:literal) => {
        $crate::Frequency::from_decimal_khz($value as f64)
    };
}

#[macro_export]
macro_rules! mhz {
    ($value:literal) => {
        $crate::Frequency::from_decimal_mhz($value as f64)
    };
}
