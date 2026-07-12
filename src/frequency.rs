use std::fmt;

use crate::error::RadioError;

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

    pub fn from_decimal_khz(khz: f64) -> crate::Result<Self> {
        Self::from_decimal(khz, 1_000.0, "kilohertz")
    }

    pub fn from_decimal_mhz(mhz: f64) -> crate::Result<Self> {
        Self::from_decimal(mhz, 1_000_000.0, "megahertz")
    }

    fn from_decimal(value: f64, multiplier: f64, unit: &'static str) -> crate::Result<Self> {
        let hz = (value * multiplier).round();
        if !value.is_finite() || value < 0.0 || !hz.is_finite() || hz < 0.0 || hz >= u64::MAX as f64
        {
            return Err(RadioError::InvalidValue {
                field: "frequency",
                message: format!("expected a finite non-negative {unit} value in range"),
            });
        }

        Ok(Self { hz: hz as u64 })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_constructors_reject_invalid_values_without_panicking() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, f64::MAX] {
            let result = std::panic::catch_unwind(|| {
                (
                    Frequency::from_decimal_khz(value),
                    Frequency::from_decimal_mhz(value),
                )
            });
            assert!(result.is_ok());
            let (khz, mhz) = result.unwrap();
            assert!(khz.is_err());
            assert!(mhz.is_err());
        }
    }

    #[test]
    fn decimal_constructors_round_valid_values() {
        assert_eq!(
            Frequency::from_decimal_khz(14_074.001).unwrap().hz(),
            14_074_001
        );
        assert_eq!(
            Frequency::from_decimal_mhz(14.074001).unwrap().hz(),
            14_074_001
        );
    }
}
