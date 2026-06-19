use crate::{
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::{StatePatch, UpdateSource},
    Frequency, LeveledSetting, Mode, Power, RadioState, Result, RitXitOffsetHz,
};

use super::SmartSdrProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCommand {
    pub commands: Vec<String>,
    pub optimistic: Vec<StatePatch>,
}

impl EncodedCommand {
    pub fn new(commands: Vec<String>, optimistic: Vec<StatePatch>) -> Self {
        Self {
            commands,
            optimistic,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub patches: Vec<StatePatch>,
    pub source_hint: Option<UpdateSource>,
}

impl DecodedFrame {
    pub fn new(patches: Vec<StatePatch>) -> Self {
        Self {
            patches,
            source_hint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingLine {
    Version(String),
    Handle(String),
    Response(ResponseLine),
    Status(String),
    Message(String),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseLine {
    pub sequence: u32,
    pub code: u32,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct LineSplitter {
    buffer: Vec<u8>,
}

impl LineSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        for byte in bytes {
            match byte {
                b'\n' | b'\r' => {
                    if self.buffer.is_empty() {
                        continue;
                    }

                    let line =
                        String::from_utf8(std::mem::take(&mut self.buffer)).map_err(|error| {
                            RadioError::Decode {
                                command: "smartsdr-line",
                                message: error.to_string(),
                            }
                        })?;
                    lines.push(line);
                }
                _ => self.buffer.push(*byte),
            }
        }

        Ok(lines)
    }
}

pub fn command_frame(sequence: u32, command: &str) -> Result<String> {
    if command.contains(['\r', '\n']) {
        return Err(RadioError::InvalidValue {
            field: "command",
            message: "SmartSDR command must not contain line breaks".to_string(),
        });
    }

    Ok(format!("C{sequence}|{command}\n"))
}

pub fn parse_line(line: &str) -> Result<IncomingLine> {
    if let Some(version) = line.strip_prefix('V') {
        return Ok(IncomingLine::Version(version.to_string()));
    }
    if let Some(handle) = line.strip_prefix('H') {
        return Ok(IncomingLine::Handle(handle.to_string()));
    }
    if let Some(rest) = line.strip_prefix('R') {
        let mut parts = rest.splitn(3, '|');
        let sequence = parts
            .next()
            .ok_or_else(|| decode_error("smartsdr-response", "missing sequence"))?
            .parse::<u32>()
            .map_err(|error| RadioError::Decode {
                command: "smartsdr-response",
                message: error.to_string(),
            })?;
        let code_text = parts
            .next()
            .ok_or_else(|| decode_error("smartsdr-response", "missing response code"))?;
        let code = u32::from_str_radix(code_text, 16).map_err(|error| RadioError::Decode {
            command: "smartsdr-response",
            message: error.to_string(),
        })?;
        let message = parts.next().unwrap_or_default().to_string();
        return Ok(IncomingLine::Response(ResponseLine {
            sequence,
            code,
            message,
        }));
    }
    if let Some(status) = line.strip_prefix('S') {
        let (_, message) = status
            .split_once('|')
            .ok_or_else(|| decode_error("smartsdr-status", "missing status delimiter"))?;
        return Ok(IncomingLine::Status(message.to_string()));
    }
    if let Some(message) = line.strip_prefix('M') {
        return Ok(IncomingLine::Message(message.to_string()));
    }

    Ok(IncomingLine::Unknown(line.to_string()))
}

pub fn encode(
    profile: &SmartSdrProfile,
    command: &RadioCommand,
    state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetReceiverFrequency {
            receiver,
            frequency,
        } => {
            require_main_receiver(*receiver, "receiver.frequency")?;
            let patch = frequency_patches(*frequency);
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice t {} {:.6} autopan=1",
                    profile.slice,
                    frequency.mhz()
                )],
                patch,
            )))
        }
        RadioCommand::SetReceiverMode { receiver, mode } => {
            require_main_receiver(*receiver, "receiver.mode")?;
            let patch = mode_patches(*mode);
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice s {} mode={}",
                    profile.slice,
                    encode_mode(*mode)?
                )],
                patch,
            )))
        }
        RadioCommand::SetReceiverFilterBandwidth {
            receiver,
            bandwidth_hz,
        } => {
            require_main_receiver(*receiver, "receiver.filter_bandwidth")?;
            let shift_hz = receiver_shift(state).unwrap_or(DEFAULT_FILTER_SHIFT_HZ);
            let (low, high) = filter_edges(*bandwidth_hz, shift_hz)?;
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice s {} filter_lo={} filter_hi={}",
                    profile.slice, low, high
                )],
                vec![
                    StatePatch::MainRxFilterBandwidth(*bandwidth_hz),
                    StatePatch::MainRxFilterShift(shift_hz),
                ],
            )))
        }
        RadioCommand::SetReceiverFilterShift { receiver, shift_hz } => {
            require_main_receiver(*receiver, "receiver.filter_shift")?;
            let bandwidth_hz = receiver_bandwidth(state).unwrap_or(DEFAULT_FILTER_BANDWIDTH_HZ);
            let (low, high) = filter_edges(bandwidth_hz, *shift_hz)?;
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice s {} filter_lo={} filter_hi={}",
                    profile.slice, low, high
                )],
                vec![
                    StatePatch::MainRxFilterBandwidth(bandwidth_hz),
                    StatePatch::MainRxFilterShift(*shift_hz),
                ],
            )))
        }
        RadioCommand::SetReceiverPreamp { .. } => Err(RadioError::UnsupportedCapability {
            capability: "receiver.preamp",
        }),
        RadioCommand::SetReceiverAttenuator { .. } => Err(RadioError::UnsupportedCapability {
            capability: "receiver.attenuator",
        }),
        RadioCommand::SetReceiverNoiseBlanker { receiver, setting } => {
            require_main_receiver(*receiver, "receiver.noise_blanker")?;
            Ok(Some(rf_level_command(
                profile.slice,
                "nb",
                "nb_level",
                *setting,
                StatePatch::MainRxNoiseBlanker(bool_level_patch(*setting)),
            )?))
        }
        RadioCommand::SetReceiverNoiseReduction { receiver, setting } => {
            require_main_receiver(*receiver, "receiver.noise_reduction")?;
            Ok(Some(rf_level_command(
                profile.slice,
                "nr",
                "nr_level",
                *setting,
                StatePatch::MainRxNoiseReduction(bool_level_patch(*setting)),
            )?))
        }
        RadioCommand::SetReceiverAutoNotch { receiver, enabled } => {
            require_main_receiver(*receiver, "receiver.auto_notch")?;
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice s {} anf={}",
                    profile.slice,
                    bool_text(*enabled)
                )],
                vec![StatePatch::MainRxAutoNotch(*enabled)],
            )))
        }
        RadioCommand::SetTxFrequency(frequency) => Ok(Some(EncodedCommand::new(
            vec![format!(
                "slice t {} {:.6} autopan=1",
                profile.slice,
                frequency.mhz()
            )],
            frequency_patches(*frequency),
        ))),
        RadioCommand::SetTxMode(mode) => Ok(Some(EncodedCommand::new(
            vec![format!(
                "slice s {} mode={}",
                profile.slice,
                encode_mode(*mode)?
            )],
            mode_patches(*mode),
        ))),
        RadioCommand::SetTxPower(power) => {
            let watts = encode_rfpower(*power)?;
            Ok(Some(EncodedCommand::new(
                vec![format!("transmit set rfpower={watts}")],
                vec![StatePatch::TxPower(Power::from_watts(watts))],
            )))
        }
        RadioCommand::SetPtt(enabled) => Ok(Some(EncodedCommand::new(
            if *enabled {
                vec![
                    format!("slice s {} tx=1", profile.slice),
                    "xmit 1".to_string(),
                ]
            } else {
                vec!["xmit 0".to_string()]
            },
            Vec::new(),
        ))),
        RadioCommand::SetSplit(_) => Err(RadioError::UnsupportedCapability {
            capability: "tx.split",
        }),
        RadioCommand::SetRitEnabled { receiver, enabled } => {
            require_main_receiver(*receiver, "rit.enabled")?;
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice s {} rit_on={}",
                    profile.slice,
                    bool_digit(*enabled)
                )],
                vec![StatePatch::MainRitEnabled(*enabled)],
            )))
        }
        RadioCommand::SetXitEnabled(enabled) => Ok(Some(EncodedCommand::new(
            vec![format!(
                "slice s {} xit_on={}",
                profile.slice,
                bool_digit(*enabled)
            )],
            vec![StatePatch::XitEnabled(*enabled)],
        ))),
        RadioCommand::SetRitOffset { receiver, offset } => {
            require_main_receiver(*receiver, "rit.offset")?;
            Ok(Some(EncodedCommand::new(
                vec![format!(
                    "slice s {} rit_freq={}",
                    profile.slice,
                    offset.as_hz()
                )],
                vec![StatePatch::RitOffset(*offset)],
            )))
        }
        RadioCommand::SetXitOffset(offset) => Ok(Some(EncodedCommand::new(
            vec![format!(
                "slice s {} xit_freq={}",
                profile.slice,
                offset.as_hz()
            )],
            vec![StatePatch::XitOffset(*offset)],
        ))),
        RadioCommand::SetRitXitOffset(offset) => Ok(Some(EncodedCommand::new(
            vec![format!(
                "slice s {} rit_freq={} xit_freq={}",
                profile.slice,
                offset.as_hz(),
                offset.as_hz()
            )],
            vec![
                StatePatch::RitXitOffset(*offset),
                StatePatch::XitOffset(*offset),
            ],
        ))),
        RadioCommand::SetKeyerSpeed(wpm) => Ok(Some(set_keyer_speed(*wpm)?)),
        RadioCommand::SendCw(text) => Ok(Some(EncodedCommand::new(
            vec![format!("cwx send \"{}\"", encode_cw_text(text)?)],
            Vec::new(),
        ))),
        RadioCommand::StopCw => Ok(Some(EncodedCommand::new(
            vec!["cwx clear".to_string()],
            Vec::new(),
        ))),
        RadioCommand::Refresh => Ok(None),
    }
}

pub fn decode_status(
    profile: &SmartSdrProfile,
    message: &str,
    state: &RadioState,
) -> Result<Option<DecodedFrame>> {
    decode_message(profile, message, state)
}

pub fn decode_response(
    profile: &SmartSdrProfile,
    command: &str,
    message: &str,
    state: &RadioState,
) -> Result<Option<DecodedFrame>> {
    match command {
        command if command.starts_with("slice info ") => decode_message(profile, message, state),
        "cwx" => decode_cwx_status(message.strip_prefix("cwx ").unwrap_or(message)).map(Some),
        "transmit info" => decode_transmit_status(message).map(Some),
        _ => decode_message(profile, message, state),
    }
}

fn decode_message(
    profile: &SmartSdrProfile,
    message: &str,
    state: &RadioState,
) -> Result<Option<DecodedFrame>> {
    if let Some(rest) = message.strip_prefix("slice ") {
        return decode_slice_status(profile, rest, state).map(Some);
    }
    if let Some(rest) = message.strip_prefix("interlock ") {
        return decode_interlock_status(rest).map(Some);
    }
    if let Some(rest) = message.strip_prefix("cwx ") {
        return decode_cwx_status(rest).map(Some);
    }
    if let Some(rest) = message.strip_prefix("transmit ") {
        return decode_transmit_status(rest).map(Some);
    }

    Ok(None)
}

const DEFAULT_FILTER_BANDWIDTH_HZ: u16 = 2_400;
const DEFAULT_FILTER_SHIFT_HZ: i16 = 1_500;

fn decode_slice_status(
    profile: &SmartSdrProfile,
    rest: &str,
    state: &RadioState,
) -> Result<DecodedFrame> {
    let (slice_text, fields_text) = rest
        .split_once(' ')
        .ok_or_else(|| decode_error("smartsdr-slice", "missing field list"))?;
    let slice = slice_text
        .parse::<u8>()
        .map_err(|error| RadioError::Decode {
            command: "smartsdr-slice",
            message: error.to_string(),
        })?;
    if slice != profile.slice {
        return Ok(DecodedFrame::new(Vec::new()));
    }

    let mut frequency = None;
    let mut mode = None;
    let mut filter_lo = None;
    let mut filter_hi = None;
    let mut rit_on = None;
    let mut rit_freq = None;
    let mut xit_on = None;
    let mut xit_freq = None;
    let mut nr_enabled = None;
    let mut nr_level = None;
    let mut nb_enabled = None;
    let mut nb_level = None;
    let mut anf_enabled = None;

    for token in fields_text.split_whitespace() {
        let Some((field, value)) = token.split_once('=') else {
            continue;
        };

        match field {
            "RF_frequency" => frequency = Some(parse_frequency_mhz(value)?),
            "mode" => mode = Some(decode_mode(value)?),
            "filter_lo" => filter_lo = Some(parse_filter_edge(value)?),
            "filter_hi" => filter_hi = Some(parse_filter_edge(value)?),
            "rit_on" => rit_on = Some(parse_bool_text(value, "rit_on")?),
            "rit_freq" => rit_freq = Some(parse_offset(value, "rit_freq")?),
            "xit_on" => xit_on = Some(parse_bool_text(value, "xit_on")?),
            "xit_freq" => xit_freq = Some(parse_offset(value, "xit_freq")?),
            "nr" => nr_enabled = Some(parse_bool_text(value, "nr")?),
            "nr_level" => nr_level = Some(parse_percent(value, "nr_level")?),
            "nb" => nb_enabled = Some(parse_bool_text(value, "nb")?),
            "nb_level" => nb_level = Some(parse_percent(value, "nb_level")?),
            "anf" => anf_enabled = Some(parse_bool_text(value, "anf")?),
            _ => {}
        }
    }

    let mut patches = Vec::new();
    if let Some(frequency) = frequency {
        patches.extend(frequency_patches(frequency));
    }
    if let Some(mode) = mode {
        patches.extend(mode_patches(mode));
    }

    if filter_lo.is_some() || filter_hi.is_some() {
        let (current_lo, current_hi) = current_filter_edges(state);
        let low = filter_lo.unwrap_or(current_lo);
        let high = filter_hi.unwrap_or(current_hi);
        let bandwidth = (high as i32 - low as i32).max(0) as u16;
        let shift = ((high as i32 + low as i32) / 2) as i16;
        patches.push(StatePatch::MainRxFilterBandwidth(bandwidth));
        patches.push(StatePatch::MainRxFilterShift(shift));
    }

    if rit_on.is_some() || rit_freq.is_some() {
        let offset = rit_freq.unwrap_or_else(|| current_rit_offset(state, ReceiverPath::Main));
        patches.push(StatePatch::MainRitEnabled(
            rit_on.unwrap_or(state.rit_xit.main_rit_enabled.unwrap_or(false)),
        ));
        patches.push(StatePatch::RitOffset(offset));
    }

    if xit_on.is_some() || xit_freq.is_some() {
        let offset = xit_freq.unwrap_or_else(|| current_xit_offset(state));
        patches.push(StatePatch::XitEnabled(
            xit_on.unwrap_or(state.rit_xit.xit_enabled.unwrap_or(false)),
        ));
        patches.push(StatePatch::XitOffset(offset));
    }

    if nr_enabled.is_some() || nr_level.is_some() {
        patches.push(StatePatch::MainRxNoiseReduction(normalize_setting_update(
            state.main_rx.rf.noise_reduction,
            nr_enabled,
            nr_level,
        )));
    }

    if nb_enabled.is_some() || nb_level.is_some() {
        patches.push(StatePatch::MainRxNoiseBlanker(normalize_setting_update(
            state.main_rx.rf.noise_blanker,
            nb_enabled,
            nb_level,
        )));
    }

    if let Some(enabled) = anf_enabled {
        patches.push(StatePatch::MainRxAutoNotch(enabled));
    }

    Ok(DecodedFrame::new(patches))
}

fn decode_interlock_status(rest: &str) -> Result<DecodedFrame> {
    let mut transmitting = None;
    for token in rest.split_whitespace() {
        let Some((field, value)) = token.split_once('=') else {
            continue;
        };
        if field == "state" {
            transmitting = Some(matches!(value, "TRANSMITTING"));
        }
    }

    Ok(DecodedFrame::new(
        transmitting
            .into_iter()
            .map(StatePatch::Transmitting)
            .collect(),
    ))
}

fn set_keyer_speed(wpm: u8) -> Result<EncodedCommand> {
    if !(5..=100).contains(&wpm) {
        return Err(RadioError::InvalidValue {
            field: "keyer.speed_wpm",
            message: "expected 5..=100".to_string(),
        });
    }

    Ok(EncodedCommand::new(
        vec![format!("cwx wpm {wpm}")],
        vec![StatePatch::KeyerSpeed(wpm)],
    ))
}

fn decode_cwx_status(rest: &str) -> Result<DecodedFrame> {
    let mut wpm = None;

    for token in rest.split_whitespace() {
        let Some((field, value)) = token.split_once('=') else {
            continue;
        };

        if field == "wpm" {
            let parsed = value.parse::<u8>().map_err(|error| RadioError::Decode {
                command: "cwx",
                message: error.to_string(),
            })?;
            if !(5..=100).contains(&parsed) {
                return Err(RadioError::Decode {
                    command: "cwx",
                    message: format!("expected wpm in 5..=100, got {parsed}"),
                });
            }
            wpm = Some(parsed);
        }
    }

    Ok(DecodedFrame::new(
        wpm.into_iter().map(StatePatch::KeyerSpeed).collect(),
    ))
}

fn decode_transmit_status(rest: &str) -> Result<DecodedFrame> {
    let mut patches = Vec::new();

    for token in rest.split_whitespace() {
        let Some((field, value)) = token.split_once('=') else {
            continue;
        };

        match field {
            "freq" => patches.push(StatePatch::TxFrequency(parse_frequency_mhz(value)?)),
            "rfpower" => patches.push(StatePatch::TxPower(parse_rfpower(value)?)),
            _ => {}
        }
    }

    Ok(DecodedFrame::new(patches))
}

fn rf_level_command(
    slice: u8,
    enabled_field: &str,
    level_field: &'static str,
    setting: LeveledSetting,
    patch: StatePatch,
) -> Result<EncodedCommand> {
    let enabled = setting_enabled(setting);
    let mut command = format!("slice s {slice} {enabled_field}={}", bool_text(enabled));
    if enabled {
        if let Some(level) = setting.level {
            if level > 100 {
                return Err(RadioError::InvalidValue {
                    field: level_field,
                    message: "expected 0..=100".to_string(),
                });
            }
            command.push(' ');
            command.push_str(level_field);
            command.push('=');
            command.push_str(&level.to_string());
        }
    }

    Ok(EncodedCommand::new(vec![command], vec![patch]))
}

fn require_main_receiver(receiver: ReceiverPath, capability: &'static str) -> Result<()> {
    if matches!(receiver, ReceiverPath::Main) {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability { capability })
    }
}

fn encode_mode(mode: Mode) -> Result<&'static str> {
    match mode {
        Mode::Lsb => Ok("LSB"),
        Mode::Usb => Ok("USB"),
        Mode::Cw => Ok("CW"),
        Mode::Am => Ok("AM"),
        Mode::Fm => Ok("FM"),
        Mode::Rtty => Ok("RTTY"),
        Mode::DataLsb => Ok("DIGL"),
        Mode::DataUsb => Ok("DIGU"),
        _ => Err(RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not supported by SmartSDR"),
        }),
    }
}

fn decode_mode(value: &str) -> Result<Mode> {
    match value.trim().to_ascii_uppercase().as_str() {
        "LSB" => Ok(Mode::Lsb),
        "USB" => Ok(Mode::Usb),
        "CW" => Ok(Mode::Cw),
        "AM" => Ok(Mode::Am),
        "FM" | "FMN" => Ok(Mode::Fm),
        "RTTY" => Ok(Mode::Rtty),
        "DIGL" => Ok(Mode::DataLsb),
        "DIGU" => Ok(Mode::DataUsb),
        other => Err(RadioError::Decode {
            command: "mode",
            message: format!("unsupported SmartSDR mode {other:?}"),
        }),
    }
}

fn encode_cw_text(text: &str) -> Result<String> {
    if text.is_empty() {
        return Err(RadioError::InvalidValue {
            field: "keyer.cw",
            message: "CW text must not be empty".to_string(),
        });
    }
    if !text
        .chars()
        .all(|ch| ch.is_ascii_graphic() || ch == ' ' || ch == '\u{7f}')
    {
        return Err(RadioError::InvalidValue {
            field: "keyer.cw",
            message: "CW text must be printable ASCII".to_string(),
        });
    }
    if text.contains('"') {
        return Err(RadioError::InvalidValue {
            field: "keyer.cw",
            message: "CW text must not contain double quotes".to_string(),
        });
    }

    Ok(text.replace(' ', "\u{7f}"))
}

fn frequency_patches(frequency: Frequency) -> Vec<StatePatch> {
    vec![
        StatePatch::MainRxFrequency(frequency),
        StatePatch::TxFrequency(frequency),
    ]
}

fn mode_patches(mode: Mode) -> Vec<StatePatch> {
    vec![StatePatch::MainRxMode(mode), StatePatch::TxMode(mode)]
}

fn receiver_bandwidth(state: &RadioState) -> Option<u16> {
    state.main_rx.filter.bandwidth_hz
}

fn receiver_shift(state: &RadioState) -> Option<i16> {
    state.main_rx.filter.shift_hz
}

fn filter_edges(bandwidth_hz: u16, shift_hz: i16) -> Result<(i16, i16)> {
    let half = (bandwidth_hz / 2) as i32;
    let shift = shift_hz as i32;
    let low = shift - half;
    let high = shift + half;
    if low < i16::MIN as i32 || high > i16::MAX as i32 {
        return Err(RadioError::InvalidValue {
            field: "receiver.filter",
            message: "filter edges overflow SmartSDR parameter range".to_string(),
        });
    }
    Ok((low as i16, high as i16))
}

fn current_filter_edges(state: &RadioState) -> (i16, i16) {
    let bandwidth = receiver_bandwidth(state).unwrap_or(DEFAULT_FILTER_BANDWIDTH_HZ);
    let shift = receiver_shift(state).unwrap_or(DEFAULT_FILTER_SHIFT_HZ);
    filter_edges(bandwidth, shift).unwrap_or((300, 2_700))
}

fn parse_frequency_mhz(value: &str) -> Result<Frequency> {
    let mhz = value.parse::<f64>().map_err(|error| RadioError::Decode {
        command: "RF_frequency",
        message: error.to_string(),
    })?;
    Ok(Frequency::from_decimal_mhz(mhz))
}

fn parse_filter_edge(value: &str) -> Result<i16> {
    if value.contains('.') {
        let mhz = value.parse::<f64>().map_err(|error| RadioError::Decode {
            command: "filter",
            message: error.to_string(),
        })?;
        let hz = (mhz * 1_000_000.0).round() as i32;
        return i16::try_from(hz).map_err(|_| RadioError::Decode {
            command: "filter",
            message: format!("filter edge {value:?} is outside i16 range"),
        });
    }

    value.parse::<i16>().map_err(|error| RadioError::Decode {
        command: "filter",
        message: error.to_string(),
    })
}

fn parse_offset(value: &str, command: &'static str) -> Result<RitXitOffsetHz> {
    let offset = value.parse::<i16>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })?;
    RitXitOffsetHz::new(offset).map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })
}

fn parse_percent(value: &str, command: &'static str) -> Result<u8> {
    let level = value.parse::<u8>().map_err(|error| RadioError::Decode {
        command,
        message: error.to_string(),
    })?;
    if level > 100 {
        return Err(RadioError::Decode {
            command,
            message: format!("expected 0..=100, got {level}"),
        });
    }
    Ok(level)
}

fn parse_rfpower(value: &str) -> Result<Power> {
    let watts = value.parse::<u16>().map_err(|error| RadioError::Decode {
        command: "rfpower",
        message: error.to_string(),
    })?;
    Ok(Power::from_watts(watts))
}

fn encode_rfpower(power: Power) -> Result<u16> {
    let microwatts = power.as_microwatts();
    if microwatts % 1_000_000 != 0 {
        return Err(RadioError::InvalidValue {
            field: "tx.power",
            message: "SmartSDR rfpower requires whole-watt values".to_string(),
        });
    }

    Ok((microwatts / 1_000_000) as u16)
}

fn parse_bool_text(value: &str, command: &'static str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "t" => Ok(true),
        "0" | "off" | "false" | "f" => Ok(false),
        other => Err(RadioError::Decode {
            command,
            message: format!("unsupported boolean value {other:?}"),
        }),
    }
}

fn normalize_setting_update(
    current: Option<LeveledSetting>,
    enabled: Option<bool>,
    level: Option<u8>,
) -> LeveledSetting {
    let current = current.unwrap_or_default();
    match enabled {
        Some(false) => LeveledSetting::disabled(),
        Some(true) => LeveledSetting::enabled(level.or(current.level).unwrap_or(1)),
        None => {
            let setting = LeveledSetting::new(current.enabled, level.or(current.level));
            bool_level_patch(setting)
        }
    }
}

fn setting_enabled(setting: LeveledSetting) -> bool {
    setting
        .enabled
        .unwrap_or_else(|| setting.level.is_some_and(|level| level > 0))
}

fn bool_level_patch(setting: LeveledSetting) -> LeveledSetting {
    if setting_enabled(setting) {
        LeveledSetting::enabled(setting.level.unwrap_or(1))
    } else {
        LeveledSetting::disabled()
    }
}

fn current_rit_offset(state: &RadioState, receiver: ReceiverPath) -> RitXitOffsetHz {
    match receiver {
        ReceiverPath::Main => state.rit_xit.offset_hz,
        ReceiverPath::Sub => state.rit_xit.sub_offset_hz,
    }
    .unwrap_or_else(zero_offset)
}

fn current_xit_offset(state: &RadioState) -> RitXitOffsetHz {
    state.rit_xit.xit_offset_hz.unwrap_or_else(zero_offset)
}

fn zero_offset() -> RitXitOffsetHz {
    RitXitOffsetHz::new(0).expect("zero is always a valid SmartSDR offset")
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn bool_digit(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}

fn decode_error(command: &'static str, message: &str) -> RadioError {
    RadioError::Decode {
        command,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::smartsdr::profile_by_id, ReceiverState, TransmitterState};

    fn profile() -> &'static SmartSdrProfile {
        profile_by_id("flexradio-smartsdr").unwrap()
    }

    fn state() -> RadioState {
        RadioState {
            main_rx: ReceiverState {
                filter: crate::ReceiverFilterState {
                    bandwidth_hz: Some(2_400),
                    shift_hz: Some(1_500),
                },
                ..ReceiverState::default()
            },
            tx: Some(TransmitterState::default()),
            ..RadioState::default()
        }
    }

    #[test]
    fn encodes_filter_bandwidth_as_lo_hi_pair() {
        let encoded = encode(
            profile(),
            &RadioCommand::SetReceiverFilterBandwidth {
                receiver: ReceiverPath::Main,
                bandwidth_hz: 2_800,
            },
            &state(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            encoded.commands,
            vec!["slice s 0 filter_lo=100 filter_hi=2900"]
        );
        assert_eq!(
            encoded.optimistic,
            vec![
                StatePatch::MainRxFilterBandwidth(2_800),
                StatePatch::MainRxFilterShift(1_500),
            ]
        );
    }

    #[test]
    fn encodes_cw_text_with_smart_sdr_space_substitution() {
        let encoded = encode(
            profile(),
            &RadioCommand::SendCw("CQ TEST".to_string()),
            &state(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.commands, vec!["cwx send \"CQ\u{7f}TEST\""]);
    }

    #[test]
    fn encodes_keyer_speed_for_cwx() {
        let encoded = encode(profile(), &RadioCommand::SetKeyerSpeed(32), &state())
            .unwrap()
            .unwrap();

        assert_eq!(encoded.commands, vec!["cwx wpm 32"]);
        assert_eq!(encoded.optimistic, vec![StatePatch::KeyerSpeed(32)]);
    }

    #[test]
    fn encodes_tx_power_as_transmit_set_rfpower() {
        let encoded = encode(
            profile(),
            &RadioCommand::SetTxPower(Power::from_watts(50)),
            &state(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(encoded.commands, vec!["transmit set rfpower=50"]);
        assert_eq!(encoded.optimistic, vec![StatePatch::TxPower(Power::from_watts(50))]);
    }

    #[test]
    fn decodes_cwx_wpm_status() {
        let decoded = decode_status(profile(), "cwx wpm=27 break_in_delay=100", &state())
            .unwrap()
            .unwrap();

        assert_eq!(decoded.patches, vec![StatePatch::KeyerSpeed(27)]);
    }

    #[test]
    fn decodes_cwx_query_response_without_prefix() {
        let decoded = decode_response(profile(), "cwx", "wpm=31 break_in_delay=100", &state())
            .unwrap()
            .unwrap();

        assert_eq!(decoded.patches, vec![StatePatch::KeyerSpeed(31)]);
    }

    #[test]
    fn decodes_transmit_info_response() {
        let decoded = decode_response(
            profile(),
            "transmit info",
            "transmit freq=14.025000 rfpower=100 tunepower=10 vox_enable=0 speed=25",
            &state(),
        )
        .unwrap()
        .unwrap();

        assert!(decoded
            .patches
            .contains(&StatePatch::TxFrequency(Frequency::from_hz(14_025_000))));
        assert!(decoded
            .patches
            .contains(&StatePatch::TxPower(Power::from_watts(100))));
    }

    #[test]
    fn decodes_slice_status_into_main_and_tx_state() {
        let decoded = decode_status(
            profile(),
            "slice 0 RF_frequency=14.074 mode=DIGU filter_lo=300 filter_hi=2700 rit_on=1 rit_freq=50 xit_on=0 xit_freq=-25 nr=on nr_level=25 nb=off anf=on",
            &state(),
        )
        .unwrap()
        .unwrap();

        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxFrequency(Frequency::from_hz(14_074_000))));
        assert!(decoded.patches.contains(&StatePatch::TxMode(Mode::DataUsb)));
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxFilterBandwidth(2_400)));
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxFilterShift(1_500)));
        assert!(decoded.patches.contains(&StatePatch::MainRxNoiseReduction(
            LeveledSetting::enabled(25)
        )));
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxNoiseBlanker(LeveledSetting::disabled())));
        assert!(decoded.patches.contains(&StatePatch::MainRxAutoNotch(true)));
    }

    #[test]
    fn line_splitter_handles_mixed_newlines() {
        let mut splitter = LineSplitter::new();
        let lines = splitter
            .push(b"V1.0.0.0\rH123\nS0|slice 0 RF_frequency=14.074\r\n")
            .unwrap();
        assert_eq!(
            lines,
            vec![
                "V1.0.0.0".to_string(),
                "H123".to_string(),
                "S0|slice 0 RF_frequency=14.074".to_string()
            ]
        );
    }
}
