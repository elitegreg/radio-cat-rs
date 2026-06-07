use std::{fmt, num::ParseIntError, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::time::sleep;
use tracing::debug;

use crate::{
    options::RadioOptions,
    transport::{BoxedPort, CatTransport, CommandIo},
    ConnectionConfig, ControllableRadio, Frequency, Mode, RadioError, Result,
};

const MAX_FREQUENCY_HZ: u64 = 99_999_999_999;
const MIN_CW_WPM: u16 = 1;
const MAX_CW_WPM: u16 = 999;
const MAX_CW_TEXT_BYTES: usize = 60;
const MAX_RIT_OFFSET_HZ: i32 = 9_999;

const DEFAULT_RETRY_MAX: u8 = 3;
const DEFAULT_RETRY_BACKOFF_MS: u64 = 25;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum YaesuModel {
    Ft450,
    Ft950,
    Ft2000,
    Ftdx1200,
    Ftdx3000,
    Ftdx5000,
    Ftdx9000,
    Ft991,
    Ft891,
    Ft710,
    Ftdx10,
    Ftdx101d,
    Ftdx101mp,
}

impl YaesuModel {
    pub const ALL: &'static [Self] = &[
        Self::Ft450,
        Self::Ft950,
        Self::Ft2000,
        Self::Ftdx1200,
        Self::Ftdx3000,
        Self::Ftdx5000,
        Self::Ftdx9000,
        Self::Ft991,
        Self::Ft891,
        Self::Ft710,
        Self::Ftdx10,
        Self::Ftdx101d,
        Self::Ftdx101mp,
    ];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ft450 => "ft-450",
            Self::Ft950 => "ft-950",
            Self::Ft2000 => "ft-2000",
            Self::Ftdx1200 => "ftdx-1200",
            Self::Ftdx3000 => "ftdx-3000",
            Self::Ftdx5000 => "ftdx-5000",
            Self::Ftdx9000 => "ftdx-9000",
            Self::Ft991 => "ft-991",
            Self::Ft891 => "ft-891",
            Self::Ft710 => "ft-710",
            Self::Ftdx10 => "ftdx-10",
            Self::Ftdx101d => "ftdx-101d",
            Self::Ftdx101mp => "ftdx-101mp",
        }
    }

    pub fn display_name(self) -> String {
        format!("Yaesu {}", self.as_str().to_ascii_uppercase())
    }

    pub(crate) fn from_alias(value: &str) -> Option<Self> {
        let normalized = normalize_model_name(value);

        Self::all().iter().copied().find(|model| {
            let info = model.info();
            normalize_model_name(info.name) == normalized
                || info
                    .aliases
                    .iter()
                    .any(|alias| normalize_model_name(alias) == normalized)
        })
    }

    fn info(self) -> YaesuModelInfo {
        match self {
            Self::Ft450 => YaesuModelInfo::new("ft-450", YaesuProfile::Early, &["ft450"]),
            Self::Ft950 => YaesuModelInfo::new("ft-950", YaesuProfile::Early, &["ft950"]),
            Self::Ft2000 => YaesuModelInfo::new("ft-2000", YaesuProfile::Early, &["ft2000"]),
            Self::Ftdx1200 => YaesuModelInfo::new("ftdx-1200", YaesuProfile::Mid, &["ftdx1200"]),
            Self::Ftdx3000 => YaesuModelInfo::new("ftdx-3000", YaesuProfile::Mid, &["ftdx3000"]),
            Self::Ftdx5000 => YaesuModelInfo::new("ftdx-5000", YaesuProfile::Mid, &["ftdx5000"]),
            Self::Ftdx9000 => YaesuModelInfo::new("ftdx-9000", YaesuProfile::Early, &["ftdx9000"]),
            Self::Ft991 => YaesuModelInfo::new("ft-991", YaesuProfile::Ft991, &["ft991"]),
            Self::Ft891 => YaesuModelInfo::new("ft-891", YaesuProfile::Mid, &["ft891"]),
            Self::Ft710 => YaesuModelInfo::new("ft-710", YaesuProfile::Modern, &["ft710"]),
            Self::Ftdx10 => YaesuModelInfo::new("ftdx-10", YaesuProfile::Modern, &["ftdx10"]),
            Self::Ftdx101d => YaesuModelInfo::new("ftdx-101d", YaesuProfile::Modern, &["ftdx101d"]),
            Self::Ftdx101mp => {
                YaesuModelInfo::new("ftdx-101mp", YaesuProfile::Modern, &["ftdx101mp"])
            }
        }
    }
}

#[derive(Clone, Copy)]
struct YaesuModelInfo {
    name: &'static str,
    profile: YaesuProfile,
    aliases: &'static [&'static str],
}

impl YaesuModelInfo {
    const fn new(
        name: &'static str,
        profile: YaesuProfile,
        aliases: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            profile,
            aliases,
        }
    }
}

fn normalize_model_name(value: &str) -> String {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum YaesuProfile {
    Early,
    Mid,
    Ft991,
    Modern,
}

#[derive(Clone, Copy)]
struct ModeCode {
    code: char,
    mode: Mode,
}

#[derive(Clone, Copy)]
struct YaesuDescriptor {
    name: &'static str,
    mode_map: &'static [ModeCode],
}

const MODE_MAP_EARLY: [ModeCode; 11] = [
    ModeCode {
        code: '1',
        mode: Mode::Lsb,
    },
    ModeCode {
        code: '2',
        mode: Mode::Usb,
    },
    ModeCode {
        code: '3',
        mode: Mode::Cw,
    },
    ModeCode {
        code: '4',
        mode: Mode::Fm,
    },
    ModeCode {
        code: '5',
        mode: Mode::Am,
    },
    ModeCode {
        code: '6',
        mode: Mode::Rtty,
    },
    ModeCode {
        code: '7',
        mode: Mode::Cwr,
    },
    ModeCode {
        code: '8',
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: '9',
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: 'A',
        mode: Mode::PktFm,
    },
    ModeCode {
        code: 'C',
        mode: Mode::PktUsb,
    },
];

const MODE_MAP_MID: [ModeCode; 12] = [
    ModeCode {
        code: '1',
        mode: Mode::Lsb,
    },
    ModeCode {
        code: '2',
        mode: Mode::Usb,
    },
    ModeCode {
        code: '3',
        mode: Mode::Cw,
    },
    ModeCode {
        code: '4',
        mode: Mode::Fm,
    },
    ModeCode {
        code: '5',
        mode: Mode::Am,
    },
    ModeCode {
        code: '6',
        mode: Mode::Rtty,
    },
    ModeCode {
        code: '7',
        mode: Mode::Cwr,
    },
    ModeCode {
        code: '8',
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: '9',
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: 'A',
        mode: Mode::PktFm,
    },
    ModeCode {
        code: 'B',
        mode: Mode::Fmn,
    },
    ModeCode {
        code: 'C',
        mode: Mode::PktUsb,
    },
];

const MODE_MAP_FT991: [ModeCode; 14] = [
    ModeCode {
        code: '1',
        mode: Mode::Lsb,
    },
    ModeCode {
        code: '2',
        mode: Mode::Usb,
    },
    ModeCode {
        code: '3',
        mode: Mode::Cw,
    },
    ModeCode {
        code: '4',
        mode: Mode::Fm,
    },
    ModeCode {
        code: '5',
        mode: Mode::Am,
    },
    ModeCode {
        code: '6',
        mode: Mode::Rtty,
    },
    ModeCode {
        code: '7',
        mode: Mode::Cwr,
    },
    ModeCode {
        code: '8',
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: '9',
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: 'A',
        mode: Mode::PktFm,
    },
    ModeCode {
        code: 'B',
        mode: Mode::Fmn,
    },
    ModeCode {
        code: 'C',
        mode: Mode::PktUsb,
    },
    ModeCode {
        code: 'D',
        mode: Mode::Amn,
    },
    ModeCode {
        code: 'E',
        mode: Mode::C4fm,
    },
];

const MODE_MAP_MODERN: [ModeCode; 14] = [
    ModeCode {
        code: '1',
        mode: Mode::Lsb,
    },
    ModeCode {
        code: '2',
        mode: Mode::Usb,
    },
    ModeCode {
        code: '3',
        mode: Mode::Cw,
    },
    ModeCode {
        code: '4',
        mode: Mode::Fm,
    },
    ModeCode {
        code: '5',
        mode: Mode::Am,
    },
    ModeCode {
        code: '6',
        mode: Mode::Rtty,
    },
    ModeCode {
        code: '7',
        mode: Mode::Cwr,
    },
    ModeCode {
        code: '8',
        mode: Mode::PktLsb,
    },
    ModeCode {
        code: '9',
        mode: Mode::Rttyr,
    },
    ModeCode {
        code: 'A',
        mode: Mode::PktFm,
    },
    ModeCode {
        code: 'B',
        mode: Mode::Fmn,
    },
    ModeCode {
        code: 'C',
        mode: Mode::PktUsb,
    },
    ModeCode {
        code: 'D',
        mode: Mode::Amn,
    },
    ModeCode {
        code: 'F',
        mode: Mode::PktFmn,
    },
];

const DESCRIPTOR_EARLY: YaesuDescriptor = YaesuDescriptor {
    name: "yaesu-early",
    mode_map: &MODE_MAP_EARLY,
};

const DESCRIPTOR_MID: YaesuDescriptor = YaesuDescriptor {
    name: "yaesu-mid",
    mode_map: &MODE_MAP_MID,
};

const DESCRIPTOR_FT991: YaesuDescriptor = YaesuDescriptor {
    name: "yaesu-ft991",
    mode_map: &MODE_MAP_FT991,
};

const DESCRIPTOR_MODERN: YaesuDescriptor = YaesuDescriptor {
    name: "yaesu-modern",
    mode_map: &MODE_MAP_MODERN,
};

impl YaesuProfile {
    fn descriptor(self) -> &'static YaesuDescriptor {
        match self {
            Self::Early => &DESCRIPTOR_EARLY,
            Self::Mid => &DESCRIPTOR_MID,
            Self::Ft991 => &DESCRIPTOR_FT991,
            Self::Modern => &DESCRIPTOR_MODERN,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RetryPolicy {
    max_retries: u8,
    backoff: Duration,
}

#[derive(Clone)]
pub struct YaesuNewCatRadio {
    io: Arc<dyn CommandIo>,
    model: YaesuModel,
    profile: YaesuProfile,
    retry: RetryPolicy,
    stop_cw_cmd: Option<String>,
}

impl fmt::Debug for YaesuNewCatRadio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("YaesuNewCatRadio")
            .field("model", &self.model.as_str())
            .field("profile", &self.profile.descriptor().name)
            .field("retry_max", &self.retry.max_retries)
            .field("retry_backoff", &self.retry.backoff)
            .finish_non_exhaustive()
    }
}

impl YaesuNewCatRadio {
    pub(crate) async fn connect(
        connection: ConnectionConfig,
        model: YaesuModel,
        options: &RadioOptions,
    ) -> Result<Self> {
        let profile = model.info().profile;

        debug!(
            ?connection,
            model = model.as_str(),
            profile = profile.descriptor().name,
            "connecting Yaesu New-CAT radio"
        );

        let (io, timeout) = connection.open_io().await?;
        Self::connect_io(io, timeout, model, options).await
    }

    pub(crate) async fn connect_io(
        io: BoxedPort,
        timeout: Duration,
        model: YaesuModel,
        options: &RadioOptions,
    ) -> Result<Self> {
        let profile = model.info().profile;
        let retry_max = parse_u8_option(options, "yaesu.retry_max")?.unwrap_or(DEFAULT_RETRY_MAX);
        let retry_backoff_ms = parse_u64_option(options, "yaesu.retry_backoff_ms")?
            .unwrap_or(DEFAULT_RETRY_BACKOFF_MS);
        let stop_cw_cmd = normalize_stop_cw_command(options.get("yaesu.stop_cw_cmd"));

        let transport = CatTransport::from_io(io, timeout);
        let io: Arc<dyn CommandIo> = Arc::new(transport);

        debug!(
            model = model.as_str(),
            profile = profile.descriptor().name,
            retry_max,
            retry_backoff_ms,
            stop_cw_cmd = stop_cw_cmd.as_deref().unwrap_or("<unset>"),
            timeout = ?timeout,
            "connected Yaesu New-CAT radio over IO"
        );

        Ok(Self {
            io,
            model,
            profile,
            retry: RetryPolicy {
                max_retries: retry_max,
                backoff: Duration::from_millis(retry_backoff_ms),
            },
            stop_cw_cmd,
        })
    }

    #[cfg(test)]
    fn from_io(
        io: Arc<dyn CommandIo>,
        model: YaesuModel,
        profile: YaesuProfile,
        retry: RetryPolicy,
        stop_cw_cmd: Option<String>,
    ) -> Self {
        Self {
            io,
            model,
            profile,
            retry,
            stop_cw_cmd,
        }
    }

    fn descriptor(&self) -> &'static YaesuDescriptor {
        self.profile.descriptor()
    }

    async fn query_with_retry(&self, command: &str, operation: &'static str) -> Result<String> {
        for attempt in 0..=self.retry.max_retries {
            let response = self.io.query(command).await?;

            if !Self::is_retryable_response(&response) {
                return Ok(response);
            }

            if attempt == self.retry.max_retries {
                break;
            }

            sleep(self.retry.backoff).await;
        }

        Err(RadioError::RetriesExhausted { operation })
    }

    fn is_retryable_response(response: &str) -> bool {
        matches!(response.trim(), "?" | "?;")
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

    fn mode_from_code(&self, code: char) -> Result<Mode> {
        let code = code.to_ascii_uppercase();

        self.descriptor()
            .mode_map
            .iter()
            .find(|mapping| mapping.code == code)
            .map(|mapping| mapping.mode)
            .ok_or_else(|| RadioError::UnsupportedModeCode(code.to_string()))
    }

    fn code_from_mode(&self, mode: Mode) -> Option<char> {
        self.descriptor()
            .mode_map
            .iter()
            .find(|mapping| mapping.mode == mode)
            .map(|mapping| mapping.code)
    }

    fn parse_mode_response(&self, response: &str) -> Result<Mode> {
        let body = Self::response_body(response, "MD")?;
        let code = body
            .chars()
            .last()
            .ok_or_else(|| RadioError::InvalidResponse {
                command: "MD",
                response: response.to_string(),
            })?;

        self.mode_from_code(code)
    }

    fn format_frequency_set(frequency: Frequency) -> Result<String> {
        let frequency_hz = frequency.hz();

        if !(1..=MAX_FREQUENCY_HZ).contains(&frequency_hz) {
            return Err(RadioError::FrequencyOutOfRange(frequency_hz));
        }

        Ok(format!("FA{frequency_hz:011};"))
    }

    fn format_mode_set(&self, mode: Mode) -> Result<String> {
        let Some(code) = self.code_from_mode(mode) else {
            return Err(RadioError::UnsupportedModeForRadio {
                mode: mode.to_string(),
                radio: self.model.as_str(),
            });
        };

        Ok(format!("MD{code};"))
    }

    fn format_cw_speed_set(wpm: u16) -> Result<String> {
        if !(MIN_CW_WPM..=MAX_CW_WPM).contains(&wpm) {
            return Err(RadioError::CwSpeedOutOfRange(wpm));
        }

        Ok(format!("KS{wpm:03};"))
    }

    fn format_cw_text_load(text: &str) -> Result<String> {
        if text.len() > MAX_CW_TEXT_BYTES {
            return Err(RadioError::CwTextTooLong(text.len()));
        }

        if !text.is_ascii() || text.contains(';') || text.contains('\r') || text.contains('\n') {
            return Err(RadioError::InvalidCwText);
        }

        Ok(format!("KM1{text};"))
    }

    fn validate_rit_offset(offset_hz: i32) -> Result<()> {
        if (-MAX_RIT_OFFSET_HZ..=MAX_RIT_OFFSET_HZ).contains(&offset_hz) {
            Ok(())
        } else {
            Err(RadioError::RitOffsetOutOfRange(offset_hz))
        }
    }

    fn parse_if_rit_response(response: &str) -> Result<i32> {
        let body = Self::response_body(response, "IF")?;
        if body.len() < 23 {
            return Err(RadioError::InvalidResponse {
                command: "IF",
                response: response.to_string(),
            });
        }

        let offset = body[16..22]
            .parse::<i32>()
            .map_err(|source| RadioError::parse_int(response, source))?;
        let rit_on = body.as_bytes().get(22).copied() == Some(b'1');
        Ok(if rit_on { offset } else { 0 })
    }

    fn format_standard_rit_set(offset_hz: i32) -> Result<String> {
        Self::validate_rit_offset(offset_hz)?;
        let magnitude = offset_hz.unsigned_abs();
        let prefix = if offset_hz < 0 { "RD" } else { "RU" };
        Ok(format!("RC;{prefix}{magnitude:04};"))
    }
}

#[async_trait]
impl ControllableRadio for YaesuNewCatRadio {
    async fn get_frequency(&self) -> Result<Frequency> {
        let response = self.query_with_retry("FA;", "get-frequency").await?;
        Self::parse_frequency_response(&response)
    }

    async fn set_frequency(&self, frequency: Frequency) -> Result<()> {
        let command = Self::format_frequency_set(frequency)?;

        for attempt in 0..=self.retry.max_retries {
            self.io.send(&command).await?;

            let actual = self.get_frequency().await?;
            if actual == frequency {
                return Ok(());
            }

            if attempt == self.retry.max_retries {
                return Err(RadioError::VerificationFailed {
                    operation: "set-frequency",
                    expected: frequency.hz().to_string(),
                    actual: actual.hz().to_string(),
                });
            }

            sleep(self.retry.backoff).await;
        }

        Err(RadioError::RetriesExhausted {
            operation: "set-frequency",
        })
    }

    async fn get_mode(&self) -> Result<Mode> {
        let response = self.query_with_retry("MD;", "get-mode").await?;
        self.parse_mode_response(&response)
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        let command = self.format_mode_set(mode)?;

        for attempt in 0..=self.retry.max_retries {
            self.io.send(&command).await?;

            let actual = self.get_mode().await?;
            if actual == mode {
                return Ok(());
            }

            if attempt == self.retry.max_retries {
                return Err(RadioError::VerificationFailed {
                    operation: "set-mode",
                    expected: mode.to_string(),
                    actual: actual.to_string(),
                });
            }

            sleep(self.retry.backoff).await;
        }

        Err(RadioError::RetriesExhausted {
            operation: "set-mode",
        })
    }

    async fn send_cw(&self, text: &str) -> Result<()> {
        let load_command = Self::format_cw_text_load(text)?;

        self.io.send(&load_command).await?;
        self.io.send("KY6;").await
    }

    async fn stop_cw(&self) -> Result<()> {
        let Some(stop_cw_cmd) = &self.stop_cw_cmd else {
            return Err(RadioError::UnsupportedOperation {
                operation: "stop-cw",
                radio: self.model.as_str(),
            });
        };

        self.io.send(stop_cw_cmd).await
    }

    async fn get_cw_wpm(&self) -> Result<u16> {
        let response = self.query_with_retry("KS;", "get-cw-wpm").await?;
        Self::parse_numeric_response(&response, "KS")
    }

    async fn set_cw_wpm(&self, wpm: u16) -> Result<()> {
        let command = Self::format_cw_speed_set(wpm)?;

        for attempt in 0..=self.retry.max_retries {
            self.io.send(&command).await?;

            let actual = self.get_cw_wpm().await?;
            if actual == wpm {
                return Ok(());
            }

            if attempt == self.retry.max_retries {
                return Err(RadioError::VerificationFailed {
                    operation: "set-cw-wpm",
                    expected: wpm.to_string(),
                    actual: actual.to_string(),
                });
            }

            sleep(self.retry.backoff).await;
        }

        Err(RadioError::RetriesExhausted {
            operation: "set-cw-wpm",
        })
    }

    async fn get_rit(&self) -> Result<i32> {
        if self.model == YaesuModel::Ft710 {
            return Err(RadioError::UnsupportedOperation {
                operation: "get-rit",
                radio: self.model.as_str(),
            });
        }

        let response = self.query_with_retry("IF;", "get-rit").await?;
        Self::parse_if_rit_response(&response)
    }

    async fn set_rit(&self, offset_hz: i32) -> Result<()> {
        Self::validate_rit_offset(offset_hz)?;

        if self.model == YaesuModel::Ft710 {
            return Err(RadioError::UnsupportedOperation {
                operation: "set-rit",
                radio: self.model.as_str(),
            });
        }

        self.io.send("RT1;").await?;
        let command = Self::format_standard_rit_set(offset_hz)?;
        self.io.send(&command).await
    }

    async fn clear_rit(&self) -> Result<()> {
        if self.model == YaesuModel::Ft710 {
            return Err(RadioError::UnsupportedOperation {
                operation: "clear-rit",
                radio: self.model.as_str(),
            });
        }

        self.io.send("RC;").await
    }
}

fn normalize_stop_cw_command(value: Option<&str>) -> Option<String> {
    let value = value?.trim();

    if value.is_empty() {
        return None;
    }

    if value.ends_with(';') {
        Some(value.to_string())
    } else {
        Some(format!("{value};"))
    }
}

fn parse_u8_option(options: &RadioOptions, key: &str) -> Result<Option<u8>> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };

    let value = value.trim();
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u8::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(|_| RadioError::InvalidOption {
        key: key.to_string(),
        value: value.to_string(),
    })?;

    Ok(Some(parsed))
}

fn parse_u64_option(options: &RadioOptions, key: &str) -> Result<Option<u64>> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };

    let parsed = value
        .trim()
        .parse()
        .map_err(|_| RadioError::InvalidOption {
            key: key.to_string(),
            value: value.to_string(),
        })?;

    Ok(Some(parsed))
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

    fn no_wait_retry() -> RetryPolicy {
        RetryPolicy {
            max_retries: 2,
            backoff: Duration::from_millis(0),
        }
    }

    #[test]
    fn parses_model_aliases() {
        assert_eq!(YaesuModel::from_alias("ft-991"), Some(YaesuModel::Ft991));
        assert_eq!(
            YaesuModel::from_alias("FTDX101MP"),
            Some(YaesuModel::Ftdx101mp)
        );
        assert_eq!(
            YaesuModel::from_alias("ftdx-9000"),
            Some(YaesuModel::Ftdx9000)
        );
    }

    #[test]
    fn normalizes_optional_stop_command() {
        assert_eq!(
            normalize_stop_cw_command(Some("RX")),
            Some("RX;".to_string())
        );
        assert_eq!(
            normalize_stop_cw_command(Some("RX;")),
            Some("RX;".to_string())
        );
        assert_eq!(normalize_stop_cw_command(None), None);
    }

    #[test]
    fn modern_profile_rejects_c4fm() {
        let radio = YaesuNewCatRadio::from_io(
            Arc::new(MockIo::default()),
            YaesuModel::Ft710,
            YaesuProfile::Modern,
            no_wait_retry(),
            None,
        );

        let error = radio.format_mode_set(Mode::C4fm).unwrap_err();
        assert!(matches!(error, RadioError::UnsupportedModeForRadio { .. }));
    }

    #[tokio::test]
    async fn uses_expected_commands_with_verification() {
        let io = Arc::new(MockIo::default());

        io.push_query("FA;", "FA00014074000;").await;
        io.push_query("MD;", "MD2;").await;
        io.push_query("KS;", "KS020;").await;

        io.push_query("FA;", "?;").await;
        io.push_query("FA;", "FA00007050000;").await;

        io.push_query("MD;", "MDE;").await;

        io.push_query("KS;", "KS025;").await;

        let radio = YaesuNewCatRadio::from_io(
            io.clone(),
            YaesuModel::Ft991,
            YaesuProfile::Ft991,
            no_wait_retry(),
            Some("RX;".to_string()),
        );

        assert_eq!(
            radio.get_frequency().await.unwrap(),
            Frequency::from_hz(14_074_000)
        );
        assert_eq!(radio.get_mode().await.unwrap(), Mode::Usb);
        assert_eq!(radio.get_cw_wpm().await.unwrap(), 20);

        radio
            .set_frequency(Frequency::from_hz(7_050_000))
            .await
            .unwrap();
        radio.set_mode(Mode::C4fm).await.unwrap();
        radio.set_cw_wpm(25).await.unwrap();
        radio.send_cw("CQ TEST").await.unwrap();
        radio.stop_cw().await.unwrap();

        assert_eq!(
            io.sent_commands().await,
            vec![
                "FA;",
                "MD;",
                "KS;",
                "FA00007050000;",
                "FA;",
                "FA;",
                "MDE;",
                "MD;",
                "KS025;",
                "KS;",
                "KM1CQ TEST;",
                "KY6;",
                "RX;",
            ]
        );
    }

    #[tokio::test]
    async fn stop_cw_is_unsupported_when_not_configured() {
        let radio = YaesuNewCatRadio::from_io(
            Arc::new(MockIo::default()),
            YaesuModel::Ft710,
            YaesuProfile::Modern,
            no_wait_retry(),
            None,
        );

        let error = radio.stop_cw().await.unwrap_err();
        assert!(matches!(
            error,
            RadioError::UnsupportedOperation {
                operation: "stop-cw",
                ..
            }
        ));
    }
}
