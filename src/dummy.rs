use std::{fmt, sync::Arc};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{ControllableRadio, Frequency, Mode, RadioError, Result};

const DEFAULT_FREQUENCY_HZ: u64 = 14_000_000;
const DEFAULT_MODE: Mode = Mode::Cw;
const DEFAULT_CW_WPM: u16 = 20;
const DEFAULT_RIT_HZ: i32 = 0;
const MAX_RIT_OFFSET_HZ: i32 = 9_999;

#[derive(Clone, Copy, Debug)]
struct DummyState {
    frequency: Frequency,
    mode: Mode,
    cw_wpm: u16,
    rit_hz: i32,
}

#[derive(Clone)]
pub struct DummyRadio {
    state: Arc<Mutex<DummyState>>,
}

impl fmt::Debug for DummyRadio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DummyRadio")
            .field("kind", &Self::as_str())
            .field("display_name", &Self::display_name())
            .finish_non_exhaustive()
    }
}

impl DummyRadio {
    pub const fn as_str() -> &'static str {
        "dummy"
    }

    pub const fn display_name() -> &'static str {
        "Dummy (test)"
    }

    pub(crate) fn from_alias(value: &str) -> bool {
        matches!(normalize_name(value).as_str(), "dummy" | "dummytest")
    }

    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(DummyState {
                frequency: Frequency::from_hz(DEFAULT_FREQUENCY_HZ),
                mode: DEFAULT_MODE,
                cw_wpm: DEFAULT_CW_WPM,
                rit_hz: DEFAULT_RIT_HZ,
            })),
        }
    }
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| {
            !character.is_ascii_whitespace()
                && *character != '-'
                && *character != '_'
                && *character != '/'
                && *character != '('
                && *character != ')'
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

#[async_trait]
impl ControllableRadio for DummyRadio {
    async fn get_frequency(&self) -> Result<Frequency> {
        Ok(self.state.lock().await.frequency)
    }

    async fn set_frequency(&self, frequency: Frequency) -> Result<()> {
        self.state.lock().await.frequency = frequency;
        Ok(())
    }

    async fn get_mode(&self) -> Result<Mode> {
        Ok(self.state.lock().await.mode)
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        self.state.lock().await.mode = mode;
        Ok(())
    }

    async fn send_cw(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    async fn stop_cw(&self) -> Result<()> {
        Ok(())
    }

    async fn get_cw_wpm(&self) -> Result<u16> {
        Ok(self.state.lock().await.cw_wpm)
    }

    async fn set_cw_wpm(&self, wpm: u16) -> Result<()> {
        self.state.lock().await.cw_wpm = wpm;
        Ok(())
    }

    async fn get_rit(&self) -> Result<i32> {
        Ok(self.state.lock().await.rit_hz)
    }

    async fn set_rit(&self, offset_hz: i32) -> Result<()> {
        if !(-MAX_RIT_OFFSET_HZ..=MAX_RIT_OFFSET_HZ).contains(&offset_hz) {
            return Err(RadioError::RitOffsetOutOfRange(offset_hz));
        }

        self.state.lock().await.rit_hz = offset_hz;
        Ok(())
    }

    async fn clear_rit(&self) -> Result<()> {
        self.state.lock().await.rit_hz = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dummy_aliases() {
        assert!(DummyRadio::from_alias("dummy"));
        assert!(DummyRadio::from_alias("Dummy (test)"));
        assert!(!DummyRadio::from_alias("not-a-radio"));
    }

    #[tokio::test]
    async fn stores_mutable_state() {
        let radio = DummyRadio::new();

        assert_eq!(
            radio.get_frequency().await.unwrap(),
            Frequency::from_hz(14_000_000)
        );
        assert_eq!(radio.get_mode().await.unwrap(), Mode::Cw);

        radio
            .set_frequency(Frequency::from_hz(7_030_000))
            .await
            .unwrap();
        radio.set_mode(Mode::Usb).await.unwrap();
        radio.set_cw_wpm(32).await.unwrap();

        assert_eq!(
            radio.get_frequency().await.unwrap(),
            Frequency::from_hz(7_030_000)
        );
        assert_eq!(radio.get_mode().await.unwrap(), Mode::Usb);
        assert_eq!(radio.get_cw_wpm().await.unwrap(), 32);
    }
}
