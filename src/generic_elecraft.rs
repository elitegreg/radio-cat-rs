use std::{fmt, num::ParseIntError, str::FromStr, sync::Arc};

use async_trait::async_trait;

use crate::{
    transport::{CatTransport, CommandIo},
    ConnectionConfig, ControllableRadio, Frequency, Mode, RadioError, Result,
};

const MIN_FREQUENCY_HZ: u64 = 100_000;
const MAX_FREQUENCY_HZ: u64 = 54_000_000;
const MIN_CW_WPM: u16 = 8;
const MAX_CW_WPM: u16 = 100;
const MAX_CW_TEXT_BYTES: usize = 60;

#[derive(Clone)]
pub struct GenericElecraft {
    io: Arc<dyn CommandIo>,
}

impl fmt::Debug for GenericElecraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenericElecraft").finish_non_exhaustive()
    }
}

impl GenericElecraft {
    pub async fn connect(connection: ConnectionConfig) -> Result<Self> {
        let transport = CatTransport::open(&connection).await?;
        let io: Arc<dyn CommandIo> = Arc::new(transport);

        io.send("AI0;").await?;

        Ok(Self { io })
    }

    #[cfg(test)]
    fn from_io(io: Arc<dyn CommandIo>) -> Self {
        Self { io }
    }

    fn format_frequency_set(frequency: Frequency) -> Result<String> {
        let frequency_hz = frequency.hz();

        if !(MIN_FREQUENCY_HZ..=MAX_FREQUENCY_HZ).contains(&frequency_hz) {
            return Err(RadioError::FrequencyOutOfRange(frequency_hz));
        }

        Ok(format!("FA{frequency_hz};"))
    }

    fn format_mode_set(mode: Mode) -> String {
        format!("MD{};", Self::mode_to_elecraft_code(mode))
    }

    fn format_cw_speed_set(wpm: u16) -> Result<String> {
        if !(MIN_CW_WPM..=MAX_CW_WPM).contains(&wpm) {
            return Err(RadioError::CwSpeedOutOfRange(wpm));
        }

        Ok(format!("KS{wpm:03};"))
    }

    fn format_cw_text(text: &str) -> Result<String> {
        if text.len() > MAX_CW_TEXT_BYTES {
            return Err(RadioError::CwTextTooLong(text.len()));
        }

        if !text.is_ascii() || text.contains(';') || text.contains('\r') || text.contains('\n') {
            return Err(RadioError::InvalidCwText);
        }

        Ok(format!("KY {text};"))
    }

    fn parse_numeric_response<T>(response: &str, prefix: &'static str) -> Result<T>
    where
        T: FromStr<Err = ParseIntError>,
    {
        let body = Self::response_body(response, prefix)?;
        body.parse()
            .map_err(|source| RadioError::parse_int(response, source))
    }

    fn parse_frequency_response(response: &str) -> Result<Frequency> {
        let frequency_hz = Self::parse_numeric_response::<u64>(response, "FA")?;
        Ok(Frequency::from_hz(frequency_hz))
    }

    fn parse_mode_response(response: &str) -> Result<Mode> {
        let code = Self::parse_numeric_response::<u8>(response, "MD")?;
        Self::mode_from_elecraft_code(code)
    }

    fn response_body<'a>(response: &'a str, prefix: &'static str) -> Result<&'a str> {
        response
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(';'))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RadioError::InvalidResponse {
                command: prefix,
                response: response.to_string(),
            })
    }

    fn mode_to_elecraft_code(mode: Mode) -> u8 {
        match mode {
            Mode::Lsb => 1,
            Mode::Usb => 2,
            Mode::Cw => 3,
            Mode::Fm => 4,
        }
    }

    fn mode_from_elecraft_code(code: u8) -> Result<Mode> {
        match code {
            1 => Ok(Mode::Lsb),
            2 => Ok(Mode::Usb),
            3 | 7 => Ok(Mode::Cw),
            4 => Ok(Mode::Fm),
            _ => Err(RadioError::UnsupportedMode(code)),
        }
    }
}

#[async_trait]
impl ControllableRadio for GenericElecraft {
    async fn get_frequency(&self) -> Result<Frequency> {
        let response = self.io.query("FA;").await?;
        Self::parse_frequency_response(&response)
    }

    async fn set_frequency(&self, frequency: Frequency) -> Result<()> {
        let command = Self::format_frequency_set(frequency)?;
        self.io.send(&command).await
    }

    async fn get_mode(&self) -> Result<Mode> {
        let response = self.io.query("MD;").await?;
        Self::parse_mode_response(&response)
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        let command = Self::format_mode_set(mode);
        self.io.send(&command).await
    }

    async fn send_cw(&self, text: &str) -> Result<()> {
        let command = Self::format_cw_text(text)?;
        self.io.send(&command).await
    }

    async fn stop_cw(&self) -> Result<()> {
        // Abort any queued CW text and force the radio back to receive immediately.
        self.io.send("KY @;RX;").await
    }

    async fn get_cw_wpm(&self) -> Result<u16> {
        let response = self.io.query("KS;").await?;
        Self::parse_numeric_response(&response, "KS")
    }

    async fn set_cw_wpm(&self, wpm: u16) -> Result<()> {
        let command = Self::format_cw_speed_set(wpm)?;
        self.io.send(&command).await
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use tokio::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MockIo {
        sent: Mutex<Vec<String>>,
        responses: Mutex<VecDeque<(String, String)>>,
    }

    impl MockIo {
        async fn push_query(&self, command: &str, response: &str) {
            self.responses
                .lock()
                .await
                .push_back((command.to_string(), response.to_string()));
        }

        async fn sent_commands(&self) -> Vec<String> {
            self.sent.lock().await.clone()
        }
    }

    #[async_trait]
    impl CommandIo for MockIo {
        async fn send(&self, command: &str) -> Result<()> {
            self.sent.lock().await.push(command.to_string());
            Ok(())
        }

        async fn query(&self, command: &str) -> Result<String> {
            self.sent.lock().await.push(command.to_string());

            let (expected_command, response) = self
                .responses
                .lock()
                .await
                .pop_front()
                .expect("expected queued response");

            assert_eq!(expected_command, command);

            Ok(response)
        }
    }

    #[test]
    fn parses_frequency_response() {
        let frequency = GenericElecraft::parse_frequency_response("FA00014074000;")
            .expect("frequency should parse");

        assert_eq!(frequency, Frequency::from_hz(14_074_000));
    }

    #[test]
    fn maps_cw_reverse_to_cw() {
        let mode = GenericElecraft::parse_mode_response("MD7;").expect("mode should parse");
        assert_eq!(mode, Mode::Cw);
    }

    #[test]
    fn rejects_invalid_cw_text() {
        let error = GenericElecraft::format_cw_text("CQ;TEST").expect_err("text should fail");
        assert!(matches!(error, RadioError::InvalidCwText));
    }

    #[tokio::test]
    async fn uses_expected_commands() {
        let io = Arc::new(MockIo::default());
        io.push_query("FA;", "FA00007050000;").await;
        io.push_query("MD;", "MD3;").await;
        io.push_query("KS;", "KS018;").await;

        let radio = GenericElecraft::from_io(io.clone());

        assert_eq!(
            radio.get_frequency().await.unwrap(),
            Frequency::from_hz(7_050_000)
        );
        assert_eq!(radio.get_mode().await.unwrap(), Mode::Cw);
        assert_eq!(radio.get_cw_wpm().await.unwrap(), 18);

        radio
            .set_frequency(Frequency::from_hz(14_074_000))
            .await
            .unwrap();
        radio.set_mode(Mode::Usb).await.unwrap();
        radio.set_cw_wpm(20).await.unwrap();
        radio.send_cw("CQ TEST").await.unwrap();
        radio.stop_cw().await.unwrap();

        assert_eq!(
            io.sent_commands().await,
            vec![
                "FA;",
                "MD;",
                "KS;",
                "FA14074000;",
                "MD2;",
                "KS020;",
                "KY CQ TEST;",
                "KY @;RX;",
            ]
        );
    }
}
