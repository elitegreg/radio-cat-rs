use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Instant};
use tracing::debug;

use crate::{
    options::RadioOptions, ConnectionConfig, ControllableRadio, Frequency, Mode, RadioError, Result,
};

const DEFAULT_RETRY_MAX: u8 = 3;
const DEFAULT_RETRY_BACKOFF_MS: u64 = 25;
const DEFAULT_VERIFY_TIMEOUT_MS: u64 = 2_000;

const MAX_CW_TEXT_BYTES: usize = 120;
const MAX_RIT_OFFSET_HZ: i32 = 9_999;

type BoxedPort = Box<dyn AsyncPort>;

trait AsyncPort: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T> AsyncPort for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FlexNativeModel {
    SliceA,
    SliceB,
    SliceC,
    SliceD,
    SliceE,
    SliceF,
    SliceG,
    SliceH,
}

impl FlexNativeModel {
    pub const ALL: &'static [Self] = &[
        Self::SliceA,
        Self::SliceB,
        Self::SliceC,
        Self::SliceD,
        Self::SliceE,
        Self::SliceF,
        Self::SliceG,
        Self::SliceH,
    ];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SliceA => "smartsdr-slice-a (native)",
            Self::SliceB => "smartsdr-slice-b (native)",
            Self::SliceC => "smartsdr-slice-c (native)",
            Self::SliceD => "smartsdr-slice-d (native)",
            Self::SliceE => "smartsdr-slice-e (native)",
            Self::SliceF => "smartsdr-slice-f (native)",
            Self::SliceG => "smartsdr-slice-g (native)",
            Self::SliceH => "smartsdr-slice-h (native)",
        }
    }

    pub fn display_name(self) -> String {
        let id = self.as_str().trim_end_matches(" (native)");
        format!("FlexRadio {}", id.to_ascii_uppercase())
    }

    fn slice_index(self) -> u8 {
        match self {
            Self::SliceA => 0,
            Self::SliceB => 1,
            Self::SliceC => 2,
            Self::SliceD => 3,
            Self::SliceE => 4,
            Self::SliceF => 5,
            Self::SliceG => 6,
            Self::SliceH => 7,
        }
    }

    pub(crate) fn from_alias(value: &str) -> Option<Self> {
        let normalized = normalize_name(value);

        let aliases: &[(Self, &[&str])] = &[
            (
                Self::SliceA,
                &["smartsdr-slice-a", "smartsdr-a", "slice-a", "slicea"],
            ),
            (
                Self::SliceB,
                &["smartsdr-slice-b", "smartsdr-b", "slice-b", "sliceb"],
            ),
            (
                Self::SliceC,
                &["smartsdr-slice-c", "smartsdr-c", "slice-c", "slicec"],
            ),
            (
                Self::SliceD,
                &["smartsdr-slice-d", "smartsdr-d", "slice-d", "sliced"],
            ),
            (
                Self::SliceE,
                &["smartsdr-slice-e", "smartsdr-e", "slice-e", "slicee"],
            ),
            (
                Self::SliceF,
                &["smartsdr-slice-f", "smartsdr-f", "slice-f", "slicef"],
            ),
            (
                Self::SliceG,
                &["smartsdr-slice-g", "smartsdr-g", "slice-g", "sliceg"],
            ),
            (
                Self::SliceH,
                &["smartsdr-slice-h", "smartsdr-h", "slice-h", "sliceh"],
            ),
        ];

        aliases.iter().find_map(|(model, model_aliases)| {
            let canonical = normalize_name(model.as_str());
            if canonical == normalized {
                return Some(*model);
            }

            if model_aliases
                .iter()
                .any(|alias| normalize_name(alias) == normalized)
            {
                return Some(*model);
            }

            None
        })
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
                && *character != '.'
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug)]
struct RetryPolicy {
    max_retries: u8,
    backoff: Duration,
}

#[derive(Clone, Copy, Debug, Default)]
struct FlexState {
    frequency_hz: Option<u64>,
    mode: Option<Mode>,
    rit_on: bool,
    rit_freq_hz: i32,
}

#[derive(Clone, Debug)]
struct ResultFrame {
    sequence: u64,
    code: i64,
    message: String,
}

#[async_trait]
trait FlexLineIo: Send + Sync {
    async fn send_line(&self, line: &str) -> Result<()>;
    async fn read_line(&self) -> Result<String>;
}

struct FlexTransport {
    io: Mutex<BoxedPort>,
    timeout: Duration,
}

impl FlexTransport {
    async fn open(connection: &ConnectionConfig) -> Result<Self> {
        let ConnectionConfig::Tcp {
            host,
            port,
            timeout: connect_timeout,
        } = connection
        else {
            return Err(RadioError::UnsupportedOperation {
                operation: "native-flex-requires-tcp",
                radio: "flex-native",
            });
        };

        let stream = timeout(*connect_timeout, TcpStream::connect((host.as_str(), *port)))
            .await
            .map_err(|_| RadioError::Timeout {
                operation: "TCP connect",
            })??;

        Ok(Self {
            io: Mutex::new(Box::new(stream)),
            timeout: *connect_timeout,
        })
    }

    async fn write_line_locked<T>(io: &mut T, line: &str, timeout_duration: Duration) -> Result<()>
    where
        T: AsyncWrite + Unpin + ?Sized,
    {
        timeout(timeout_duration, async {
            io.write_all(line.as_bytes()).await?;
            io.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| RadioError::Timeout {
            operation: "write line",
        })??;

        Ok(())
    }

    async fn read_line_locked<T>(io: &mut T, timeout_duration: Duration) -> Result<String>
    where
        T: AsyncRead + Unpin + ?Sized,
    {
        let line = timeout(timeout_duration, async {
            let mut bytes = Vec::new();

            loop {
                let mut byte = [0_u8; 1];
                let read = io.read(&mut byte).await?;

                if read == 0 {
                    return Err(RadioError::ConnectionClosed);
                }

                bytes.push(byte[0]);

                if byte[0] == b'\n' {
                    break;
                }
            }

            let line = String::from_utf8(bytes)?;
            Ok::<String, RadioError>(line)
        })
        .await
        .map_err(|_| RadioError::Timeout {
            operation: "read line",
        })??;

        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }
}

#[async_trait]
impl FlexLineIo for FlexTransport {
    async fn send_line(&self, line: &str) -> Result<()> {
        let mut io = self.io.lock().await;
        Self::write_line_locked(&mut *io, line, self.timeout).await
    }

    async fn read_line(&self) -> Result<String> {
        let mut io = self.io.lock().await;
        Self::read_line_locked(&mut *io, self.timeout).await
    }
}

#[derive(Clone)]
pub struct FlexNativeRadio {
    io: Arc<dyn FlexLineIo>,
    model: FlexNativeModel,
    slice: u8,
    retry: RetryPolicy,
    verify_timeout: Duration,
    command_lock: Arc<Mutex<()>>,
    next_sequence: Arc<Mutex<u64>>,
    state: Arc<Mutex<FlexState>>,
}

impl fmt::Debug for FlexNativeRadio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FlexNativeRadio")
            .field("model", &self.model.as_str())
            .field("slice", &self.slice)
            .field("retry_max", &self.retry.max_retries)
            .field("retry_backoff", &self.retry.backoff)
            .field("verify_timeout", &self.verify_timeout)
            .finish_non_exhaustive()
    }
}

impl FlexNativeRadio {
    pub(crate) async fn connect(
        connection: ConnectionConfig,
        model: FlexNativeModel,
        options: &RadioOptions,
    ) -> Result<Self> {
        if !matches!(connection, ConnectionConfig::Tcp { .. }) {
            return Err(RadioError::UnsupportedOperation {
                operation: "native-flex-requires-tcp",
                radio: model.as_str(),
            });
        }

        let retry_max = parse_u8_option(options, "flex.retry_max")?.unwrap_or(DEFAULT_RETRY_MAX);
        let retry_backoff_ms =
            parse_u64_option(options, "flex.retry_backoff_ms")?.unwrap_or(DEFAULT_RETRY_BACKOFF_MS);
        let verify_timeout_ms = parse_u64_option(options, "flex.verify_timeout_ms")?
            .unwrap_or(DEFAULT_VERIFY_TIMEOUT_MS);

        let io: Arc<dyn FlexLineIo> = Arc::new(FlexTransport::open(&connection).await?);

        let radio = Self {
            io,
            model,
            slice: model.slice_index(),
            retry: RetryPolicy {
                max_retries: retry_max,
                backoff: Duration::from_millis(retry_backoff_ms),
            },
            verify_timeout: Duration::from_millis(verify_timeout_ms),
            command_lock: Arc::new(Mutex::new(())),
            next_sequence: Arc::new(Mutex::new(0)),
            state: Arc::new(Mutex::new(FlexState::default())),
        };

        debug!(
            model = model.as_str(),
            slice = radio.slice,
            retry_max,
            retry_backoff_ms,
            verify_timeout_ms,
            "connecting native Flex SmartSDR profile"
        );

        radio.bootstrap().await?;

        Ok(radio)
    }

    #[cfg(test)]
    fn from_io(
        io: Arc<dyn FlexLineIo>,
        model: FlexNativeModel,
        retry: RetryPolicy,
        verify_timeout: Duration,
        initial_state: FlexState,
    ) -> Self {
        Self {
            io,
            model,
            slice: model.slice_index(),
            retry,
            verify_timeout,
            command_lock: Arc::new(Mutex::new(())),
            next_sequence: Arc::new(Mutex::new(0)),
            state: Arc::new(Mutex::new(initial_state)),
        }
    }

    async fn bootstrap(&self) -> Result<()> {
        let lock = self.command_lock.lock().await;
        self.execute_command_with_retry_locked(
            &lock,
            &format!("sub slice {}", self.slice),
            "flex-subscribe",
        )
        .await?;

        self.wait_for_condition_locked(&lock, "flex-initial-state", |state| {
            state.frequency_hz.is_some() && state.mode.is_some()
        })
        .await
    }

    async fn next_sequence(&self) -> u64 {
        let mut next_sequence = self.next_sequence.lock().await;
        let sequence = *next_sequence;
        *next_sequence = next_sequence.wrapping_add(1);
        sequence
    }

    async fn execute_command_with_retry_locked(
        &self,
        _lock: &tokio::sync::MutexGuard<'_, ()>,
        body: &str,
        operation: &'static str,
    ) -> Result<()> {
        for attempt in 0..=self.retry.max_retries {
            match self.execute_command_once_locked(body).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if attempt == self.retry.max_retries || !self.is_retryable_error(&error) {
                        return Err(error);
                    }

                    sleep(self.retry.backoff).await;
                }
            }
        }

        Err(RadioError::RetriesExhausted { operation })
    }

    fn is_retryable_error(&self, error: &RadioError) -> bool {
        matches!(
            error,
            RadioError::Timeout { .. } | RadioError::FlexCommandFailed { .. }
        )
    }

    async fn execute_command_once_locked(&self, body: &str) -> Result<()> {
        let sequence = self.next_sequence().await;
        let command = format!("C{sequence}|{body}\n");

        self.io.send_line(&command).await?;

        loop {
            let line = self.io.read_line().await?;

            if line.starts_with('S') {
                self.apply_status_line(&line).await;
                continue;
            }

            if let Some(result) = parse_result_line(&line) {
                if result.sequence != sequence {
                    continue;
                }

                if result.code != 0 {
                    return Err(RadioError::FlexCommandFailed {
                        sequence,
                        code: result.code,
                        message: result.message,
                    });
                }

                return Ok(());
            }
        }
    }

    async fn wait_for_condition_locked<F>(
        &self,
        _lock: &tokio::sync::MutexGuard<'_, ()>,
        operation: &'static str,
        predicate: F,
    ) -> Result<()>
    where
        F: Fn(&FlexState) -> bool,
    {
        let deadline = Instant::now() + self.verify_timeout;

        loop {
            {
                let state = self.state.lock().await;
                if predicate(&state) {
                    return Ok(());
                }
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(RadioError::RetriesExhausted { operation });
            }

            let remaining = deadline - now;
            let line = match timeout(remaining, self.io.read_line()).await {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(RadioError::RetriesExhausted { operation });
                }
            };

            if line.starts_with('S') {
                self.apply_status_line(&line).await;
            }
        }
    }

    async fn apply_status_line(&self, line: &str) {
        let payload = status_payload(line);

        let Some(slice) = parse_slice_index(payload) else {
            return;
        };

        if slice != self.slice {
            return;
        }

        let frequency_hz = find_key_value(payload, "RF_frequency")
            .and_then(|value| parse_frequency_value(value).ok());
        let mode = find_key_value(payload, "mode").and_then(mode_from_token);
        let rit_on = find_key_value(payload, "rit_on").and_then(parse_bool_value);
        let rit_freq_hz = find_key_value(payload, "rit_freq").and_then(parse_rit_freq_value);

        if frequency_hz.is_none() && mode.is_none() && rit_on.is_none() && rit_freq_hz.is_none() {
            return;
        }

        let mut state = self.state.lock().await;
        if let Some(frequency_hz) = frequency_hz {
            state.frequency_hz = Some(frequency_hz);
        }
        if let Some(mode) = mode {
            state.mode = Some(mode);
        }
        if let Some(rit_on) = rit_on {
            state.rit_on = rit_on;
        }
        if let Some(rit_freq_hz) = rit_freq_hz {
            state.rit_freq_hz = rit_freq_hz;
        }
    }

    fn mode_token_for_set(&self, mode: Mode) -> Option<&'static str> {
        match mode {
            Mode::Lsb => Some("LSB"),
            Mode::Usb => Some("USB"),
            Mode::Cw => Some("CW"),
            Mode::Am => Some("AM"),
            Mode::Fm => Some("FM"),
            Mode::Fmn => Some("FMN"),
            Mode::PktLsb => Some("DIGL"),
            Mode::PktUsb => Some("DIGU"),
            Mode::Sam => Some("SAM"),
            // RTTY is receive-parse-only in this profile per requested strict behavior.
            Mode::Rtty => None,
            _ => None,
        }
    }

    fn format_cw_payload(text: &str) -> Result<String> {
        if text.is_empty()
            || text.len() > MAX_CW_TEXT_BYTES
            || !text.is_ascii()
            || text.contains('\r')
            || text.contains('\n')
            || text.contains('"')
        {
            if text.len() > MAX_CW_TEXT_BYTES {
                return Err(RadioError::CwTextTooLong(text.len()));
            }
            return Err(RadioError::InvalidCwText);
        }

        Ok(text
            .chars()
            .map(|character| {
                if character == ' ' {
                    '\u{007F}'
                } else {
                    character
                }
            })
            .collect())
    }

    fn validate_rit_offset(offset_hz: i32) -> Result<()> {
        if (-MAX_RIT_OFFSET_HZ..=MAX_RIT_OFFSET_HZ).contains(&offset_hz) {
            Ok(())
        } else {
            Err(RadioError::RitOffsetOutOfRange(offset_hz))
        }
    }
}

#[async_trait]
impl ControllableRadio for FlexNativeRadio {
    async fn get_frequency(&self) -> Result<Frequency> {
        let state = self.state.lock().await;
        let frequency_hz = state
            .frequency_hz
            .ok_or_else(|| RadioError::FlexProtocol("frequency cache is empty".to_string()))?;
        Ok(Frequency::from_hz(frequency_hz))
    }

    async fn set_frequency(&self, frequency: Frequency) -> Result<()> {
        let frequency_hz = frequency.hz();
        if frequency_hz == 0 || frequency_hz > 99_999_999_999 {
            return Err(RadioError::FrequencyOutOfRange(frequency_hz));
        }

        let command = format!(
            "slice tune {} {:.6} autopan=1",
            self.slice,
            frequency_hz as f64 / 1_000_000.0
        );

        let lock = self.command_lock.lock().await;
        self.execute_command_with_retry_locked(&lock, &command, "flex-set-frequency")
            .await?;

        self.wait_for_condition_locked(&lock, "flex-verify-frequency", |state| {
            state.frequency_hz == Some(frequency_hz)
        })
        .await?;

        Ok(())
    }

    async fn get_mode(&self) -> Result<Mode> {
        let state = self.state.lock().await;
        state
            .mode
            .ok_or_else(|| RadioError::FlexProtocol("mode cache is empty".to_string()))
    }

    async fn set_mode(&self, mode: Mode) -> Result<()> {
        let Some(token) = self.mode_token_for_set(mode) else {
            return Err(RadioError::UnsupportedModeForRadio {
                mode: mode.to_string(),
                radio: self.model.as_str(),
            });
        };

        let command = format!("slice set {} mode={token}", self.slice);

        let lock = self.command_lock.lock().await;
        self.execute_command_with_retry_locked(&lock, &command, "flex-set-mode")
            .await?;

        self.wait_for_condition_locked(&lock, "flex-verify-mode", |state| state.mode == Some(mode))
            .await?;

        Ok(())
    }

    async fn send_cw(&self, text: &str) -> Result<()> {
        let payload = Self::format_cw_payload(text)?;
        let command = format!("cwx send \"{payload}\"");

        let lock = self.command_lock.lock().await;
        self.execute_command_with_retry_locked(&lock, &command, "flex-send-cw")
            .await
    }

    async fn stop_cw(&self) -> Result<()> {
        let lock = self.command_lock.lock().await;
        self.execute_command_with_retry_locked(&lock, "cwx clear", "flex-stop-cw")
            .await
    }

    async fn get_cw_wpm(&self) -> Result<u16> {
        Err(RadioError::UnsupportedOperation {
            operation: "get-cw-wpm",
            radio: self.model.as_str(),
        })
    }

    async fn set_cw_wpm(&self, _wpm: u16) -> Result<()> {
        Err(RadioError::UnsupportedOperation {
            operation: "set-cw-wpm",
            radio: self.model.as_str(),
        })
    }

    async fn get_rit(&self) -> Result<i32> {
        let state = self.state.lock().await;
        if state.rit_on {
            Ok(state.rit_freq_hz)
        } else {
            Ok(0)
        }
    }

    async fn set_rit(&self, offset_hz: i32) -> Result<()> {
        Self::validate_rit_offset(offset_hz)?;

        let lock = self.command_lock.lock().await;

        let enable_command = format!("slice s {} rit_on=1", self.slice);
        self.execute_command_with_retry_locked(&lock, &enable_command, "flex-set-rit-on")
            .await?;

        self.wait_for_condition_locked(&lock, "flex-verify-rit-on", |state| state.rit_on)
            .await?;

        let set_command = format!("slice s {} rit_freq={offset_hz}", self.slice);
        self.execute_command_with_retry_locked(&lock, &set_command, "flex-set-rit-freq")
            .await?;

        self.wait_for_condition_locked(&lock, "flex-verify-rit-freq", |state| {
            state.rit_on && state.rit_freq_hz == offset_hz
        })
        .await
    }

    async fn clear_rit(&self) -> Result<()> {
        let lock = self.command_lock.lock().await;
        let command = format!("slice s {} rit_freq=0", self.slice);
        self.execute_command_with_retry_locked(&lock, &command, "flex-clear-rit")
            .await?;

        self.wait_for_condition_locked(&lock, "flex-verify-clear-rit", |state| {
            state.rit_freq_hz == 0
        })
        .await
    }
}

fn status_payload(line: &str) -> &str {
    let line = line.strip_prefix('S').unwrap_or(line);
    if let Some((_, payload)) = line.split_once('|') {
        payload
    } else {
        line
    }
}

fn parse_slice_index(payload: &str) -> Option<u8> {
    let slice_prefix_index = payload.find("slice ")?;
    let rest = &payload[slice_prefix_index + 6..];
    let digits = rest
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();

    if digits.is_empty() {
        return None;
    }

    digits.parse().ok()
}

fn find_key_value<'a>(payload: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("{key}=");
    let start = payload.find(&pattern)? + pattern.len();
    let rest = &payload[start..];
    let end = rest
        .char_indices()
        .find(|(_, character)| character.is_ascii_whitespace() || *character == ',')
        .map(|(index, _)| index)
        .unwrap_or(rest.len());

    Some(rest[..end].trim_matches('"'))
}

fn parse_frequency_value(value: &str) -> Result<u64> {
    let numeric = value
        .trim()
        .parse::<f64>()
        .map_err(|_| RadioError::FlexProtocol(format!("invalid RF_frequency value `{value}`")))?;

    let hz = if numeric >= 1_000_000.0 {
        numeric.round() as u64
    } else {
        (numeric * 1_000_000.0).round() as u64
    };

    Ok(hz)
}

fn parse_bool_value(value: &str) -> Option<bool> {
    let normalized = value.trim().trim_matches('"').to_ascii_lowercase();

    match normalized.as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn parse_rit_freq_value(value: &str) -> Option<i32> {
    let value = value.trim().trim_matches('"');

    let parsed = value.parse::<i32>().ok().or_else(|| {
        value
            .parse::<f64>()
            .ok()
            .map(|number| number.round() as i32)
    })?;

    if (-MAX_RIT_OFFSET_HZ..=MAX_RIT_OFFSET_HZ).contains(&parsed) {
        Some(parsed)
    } else {
        None
    }
}

fn mode_from_token(token: &str) -> Option<Mode> {
    let token = token.trim().trim_matches('"').to_ascii_uppercase();

    match token.as_str() {
        "LSB" => Some(Mode::Lsb),
        "USB" => Some(Mode::Usb),
        "CW" => Some(Mode::Cw),
        "CWR" => Some(Mode::Cwr),
        "AM" => Some(Mode::Am),
        "FM" => Some(Mode::Fm),
        "FMN" | "NFM" => Some(Mode::Fmn),
        "DIGL" => Some(Mode::PktLsb),
        "DIGU" => Some(Mode::PktUsb),
        "SAM" => Some(Mode::Sam),
        "RTTY" => Some(Mode::Rtty),
        _ => None,
    }
}

fn parse_result_line(line: &str) -> Option<ResultFrame> {
    let line = line.strip_prefix('R')?;
    let (sequence, remainder) = line.split_once('|')?;
    let sequence = sequence.parse::<u64>().ok()?;

    let mut parts = remainder.split('|');
    let code = parts
        .next()
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let message = parts.collect::<Vec<_>>().join("|");

    Some(ResultFrame {
        sequence,
        code,
        message,
    })
}

fn parse_u8_option(options: &RadioOptions, key: &str) -> Result<Option<u8>> {
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
    struct MockLineIo {
        sent: Mutex<Vec<String>>,
        inbound: Mutex<VecDeque<String>>,
    }

    impl MockLineIo {
        async fn push_line(&self, line: &str) {
            self.inbound.lock().await.push_back(line.to_string());
        }

        async fn sent_lines(&self) -> Vec<String> {
            self.sent.lock().await.clone()
        }
    }

    #[async_trait]
    impl FlexLineIo for MockLineIo {
        async fn send_line(&self, line: &str) -> Result<()> {
            self.sent.lock().await.push(line.to_string());
            Ok(())
        }

        async fn read_line(&self) -> Result<String> {
            self.inbound
                .lock()
                .await
                .pop_front()
                .ok_or_else(|| RadioError::FlexProtocol("missing mock line".to_string()))
        }
    }

    fn default_retry() -> RetryPolicy {
        RetryPolicy {
            max_retries: 1,
            backoff: Duration::from_millis(0),
        }
    }

    #[test]
    fn parses_aliases() {
        assert_eq!(
            FlexNativeModel::from_alias("smartsdr-slice-a"),
            Some(FlexNativeModel::SliceA)
        );
        assert_eq!(
            FlexNativeModel::from_alias("slice-h"),
            Some(FlexNativeModel::SliceH)
        );
        assert_eq!(
            FlexNativeModel::from_alias("smartsdr-slice-b (native)"),
            Some(FlexNativeModel::SliceB)
        );
    }

    #[test]
    fn parses_result_frames() {
        let frame = parse_result_line("R12|0|ok|").unwrap();
        assert_eq!(frame.sequence, 12);
        assert_eq!(frame.code, 0);

        let frame = parse_result_line("R42|500|busy|").unwrap();
        assert_eq!(frame.sequence, 42);
        assert_eq!(frame.code, 500);
    }

    #[tokio::test]
    async fn sets_mode_and_frequency_with_verification() {
        let io = Arc::new(MockLineIo::default());

        io.push_line("S0|slice 0 mode=USB RF_frequency=14.074000 rit_on=0 rit_freq=0")
            .await;
        io.push_line("R0|0|").await;

        io.push_line("S0|slice 0 RF_frequency=7.050000").await;
        io.push_line("R1|0|").await;

        io.push_line("S0|slice 0 mode=DIGL").await;
        io.push_line("R2|0|").await;

        io.push_line("R3|0|").await;
        io.push_line("R4|0|").await;

        let radio = FlexNativeRadio::from_io(
            io.clone(),
            FlexNativeModel::SliceA,
            default_retry(),
            Duration::from_millis(250),
            FlexState::default(),
        );

        radio.bootstrap().await.unwrap();
        assert_eq!(
            radio.get_frequency().await.unwrap(),
            Frequency::from_hz(14_074_000)
        );
        assert_eq!(radio.get_mode().await.unwrap(), Mode::Usb);

        radio
            .set_frequency(Frequency::from_hz(7_050_000))
            .await
            .unwrap();
        radio.set_mode(Mode::PktLsb).await.unwrap();
        radio.send_cw("CQ TEST").await.unwrap();
        radio.stop_cw().await.unwrap();

        assert_eq!(
            io.sent_lines().await,
            vec![
                "C0|sub slice 0\n",
                "C1|slice tune 0 7.050000 autopan=1\n",
                "C2|slice set 0 mode=DIGL\n",
                "C3|cwx send \"CQ\u{7f}TEST\"\n",
                "C4|cwx clear\n",
            ]
        );
    }

    #[tokio::test]
    async fn sets_and_clears_rit_with_verification() {
        let io = Arc::new(MockLineIo::default());

        io.push_line("S0|slice 0 mode=USB RF_frequency=14.074000 rit_on=0 rit_freq=0")
            .await;
        io.push_line("R0|0|").await;

        io.push_line("S0|slice 0 rit_on=1").await;
        io.push_line("R1|0|").await;

        io.push_line("S0|slice 0 rit_freq=40").await;
        io.push_line("R2|0|").await;

        io.push_line("S0|slice 0 rit_freq=0").await;
        io.push_line("R3|0|").await;

        let radio = FlexNativeRadio::from_io(
            io.clone(),
            FlexNativeModel::SliceA,
            default_retry(),
            Duration::from_millis(250),
            FlexState::default(),
        );

        radio.bootstrap().await.unwrap();
        assert_eq!(radio.get_rit().await.unwrap(), 0);

        radio.set_rit(40).await.unwrap();
        assert_eq!(radio.get_rit().await.unwrap(), 40);

        radio.clear_rit().await.unwrap();
        assert_eq!(radio.get_rit().await.unwrap(), 0);

        assert_eq!(
            io.sent_lines().await,
            vec![
                "C0|sub slice 0\n",
                "C1|slice s 0 rit_on=1\n",
                "C2|slice s 0 rit_freq=40\n",
                "C3|slice s 0 rit_freq=0\n",
            ]
        );
    }

    #[tokio::test]
    async fn rejects_out_of_range_rit() {
        let radio = FlexNativeRadio::from_io(
            Arc::new(MockLineIo::default()),
            FlexNativeModel::SliceA,
            default_retry(),
            Duration::from_millis(250),
            FlexState::default(),
        );

        let error = radio.set_rit(10_000).await.unwrap_err();
        assert!(matches!(error, RadioError::RitOffsetOutOfRange(10_000)));
    }

    #[tokio::test]
    async fn rtty_set_is_strictly_unsupported() {
        let radio = FlexNativeRadio::from_io(
            Arc::new(MockLineIo::default()),
            FlexNativeModel::SliceA,
            default_retry(),
            Duration::from_millis(250),
            FlexState {
                frequency_hz: Some(14_074_000),
                mode: Some(Mode::Usb),
                ..FlexState::default()
            },
        );

        let error = radio.set_mode(Mode::Rtty).await.unwrap_err();
        assert!(matches!(error, RadioError::UnsupportedModeForRadio { .. }));
    }

    #[tokio::test]
    async fn keyer_is_unsupported() {
        let radio = FlexNativeRadio::from_io(
            Arc::new(MockLineIo::default()),
            FlexNativeModel::SliceA,
            default_retry(),
            Duration::from_millis(250),
            FlexState {
                frequency_hz: Some(14_074_000),
                mode: Some(Mode::Usb),
                ..FlexState::default()
            },
        );

        assert!(matches!(
            radio.get_cw_wpm().await.unwrap_err(),
            RadioError::UnsupportedOperation {
                operation: "get-cw-wpm",
                ..
            }
        ));
    }
}
