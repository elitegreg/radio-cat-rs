use crate::{
    command::{RadioCommand, ReceiverPath},
    error::RadioError,
    update::{StatePatch, UpdateSource},
    Frequency, LeveledSetting, Mode, Power, PowerUnit, RadioState, Result, RitXitOffsetHz,
};

use super::{CivFrame, IcomCivOptions, IcomCivProfile, ResponseMatcher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCommand {
    pub frames: Vec<CivFrame>,
    pub matcher: ResponseMatcher,
    pub response_receiver: Option<ReceiverPath>,
    pub optimistic: Vec<StatePatch>,
}

impl EncodedCommand {
    pub fn new(
        frames: Vec<CivFrame>,
        matcher: ResponseMatcher,
        response_receiver: Option<ReceiverPath>,
        optimistic: Vec<StatePatch>,
    ) -> Self {
        Self {
            frames,
            matcher,
            response_receiver,
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

pub fn encode(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    command: &RadioCommand,
    state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    match command {
        RadioCommand::SetReceiverFrequency {
            receiver,
            frequency,
        } => Ok(Some(set_vfo_frequency(
            options,
            receiver_selector(*receiver),
            *frequency,
            frequency_patches(*receiver, *frequency, state),
        )?)),
        RadioCommand::SetReceiverMode { receiver, mode } => Ok(Some(set_vfo_mode(
            profile,
            options,
            receiver_selector(*receiver),
            *mode,
            mode_patches(*receiver, *mode, state),
        )?)),
        RadioCommand::SetReceiverFilterBandwidth {
            receiver,
            bandwidth_hz,
        } => {
            require_receiver_capability(profile, *receiver, "receiver.filter_bandwidth")?;
            require_main_receiver(*receiver, "receiver.filter_bandwidth")?;
            Ok(Some(set_filter_bandwidth(options, *bandwidth_hz, state)?))
        }
        RadioCommand::SetReceiverFilterShift { .. } => Err(RadioError::UnsupportedCapability {
            capability: "receiver.filter_shift",
        }),
        RadioCommand::SetReceiverPreamp { receiver, setting } => {
            require_receiver_capability(profile, *receiver, "receiver.preamp")?;
            Ok(Some(set_preamp(profile, options, *receiver, *setting)?))
        }
        RadioCommand::SetReceiverAttenuator { receiver, setting } => {
            require_receiver_capability(profile, *receiver, "receiver.attenuator")?;
            Ok(Some(set_attenuator(profile, options, *receiver, *setting)?))
        }
        RadioCommand::SetReceiverNoiseBlanker { receiver, setting } => {
            require_receiver_capability(profile, *receiver, "receiver.noise_blanker")?;
            Ok(Some(set_receiver_bool(
                profile,
                options,
                *receiver,
                &[0x16, 0x22],
                setting_enabled(*setting),
                bool_level_patch(*setting),
                receiver_rf_patch(
                    *receiver,
                    ReceiverRfField::NoiseBlanker,
                    bool_level_patch(*setting),
                ),
            )?))
        }
        RadioCommand::SetReceiverNoiseReduction { receiver, setting } => {
            require_receiver_capability(profile, *receiver, "receiver.noise_reduction")?;
            Ok(Some(set_receiver_bool(
                profile,
                options,
                *receiver,
                &[0x16, 0x40],
                setting_enabled(*setting),
                bool_level_patch(*setting),
                receiver_rf_patch(
                    *receiver,
                    ReceiverRfField::NoiseReduction,
                    bool_level_patch(*setting),
                ),
            )?))
        }
        RadioCommand::SetReceiverAutoNotch { receiver, enabled } => {
            require_receiver_capability(profile, *receiver, "receiver.auto_notch")?;
            Ok(Some(set_receiver_bool(
                profile,
                options,
                *receiver,
                &[0x16, 0x41],
                *enabled,
                *enabled,
                receiver_auto_notch_patch(*receiver, *enabled),
            )?))
        }
        RadioCommand::SetTxFrequency(frequency) => {
            let receiver = tx_receiver_from_state(state);
            Ok(Some(set_vfo_frequency(
                options,
                receiver_selector(receiver),
                *frequency,
                frequency_patches(receiver, *frequency, state),
            )?))
        }
        RadioCommand::SetTxMode(mode) => {
            let receiver = tx_receiver_from_state(state);
            Ok(Some(set_vfo_mode(
                profile,
                options,
                receiver_selector(receiver),
                *mode,
                mode_patches(receiver, *mode, state),
            )?))
        }
        RadioCommand::SetTxPower(power) => Ok(Some(set_tx_power(profile, options, *power)?)),
        RadioCommand::SetPtt(enabled) => Ok(Some(set_ptt(options, *enabled)?)),
        RadioCommand::SetSplit(enabled) => Ok(Some(set_split(options, *enabled)?)),
        RadioCommand::SetRitEnabled { receiver, enabled } => {
            require_main_receiver(*receiver, "rit")?;
            Ok(Some(set_bool_level(
                options,
                &[0x21, 0x01],
                *enabled,
                vec![StatePatch::MainRitEnabled(*enabled)],
            )?))
        }
        RadioCommand::SetXitEnabled(enabled) => {
            require_xit(profile)?;
            Ok(Some(set_bool_level(
                options,
                &[0x21, 0x02],
                *enabled,
                vec![StatePatch::XitEnabled(*enabled)],
            )?))
        }
        RadioCommand::SetRitOffset { receiver, offset } => {
            require_main_receiver(*receiver, "rit.offset")?;
            Ok(Some(set_rit_offset(profile, options, *offset, false)?))
        }
        RadioCommand::SetXitOffset(offset) | RadioCommand::SetRitXitOffset(offset) => {
            require_xit(profile)?;
            Ok(Some(set_rit_offset(profile, options, *offset, true)?))
        }
        RadioCommand::SetKeyerSpeed(wpm) => Ok(Some(set_keyer_speed(options, *wpm)?)),
        RadioCommand::SendCw(text) => Ok(Some(send_cw(options, text)?)),
        RadioCommand::StopCw => Ok(Some(stop_cw(options)?)),
        RadioCommand::Refresh => Ok(None),
    }
}

pub fn encode_query(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    semantic: &'static str,
) -> Result<Option<EncodedCommand>> {
    let encoded = match semantic {
        "freq-main" => query(
            options,
            vec![0x25, 0x00],
            ResponseMatcher::PayloadPrefix(vec![0x25, 0x00]),
        ),
        "freq-sub" => query(
            options,
            vec![0x25, 0x01],
            ResponseMatcher::PayloadPrefix(vec![0x25, 0x01]),
        ),
        "mode-main" => query(
            options,
            vec![0x26, 0x00],
            ResponseMatcher::PayloadPrefix(vec![0x26, 0x00]),
        ),
        "mode-sub" => query(
            options,
            vec![0x26, 0x01],
            ResponseMatcher::PayloadPrefix(vec![0x26, 0x01]),
        ),
        "tx-frequency" => query(
            options,
            vec![0x1c, 0x03],
            ResponseMatcher::PayloadPrefix(vec![0x1c, 0x03]),
        ),
        "ptt" => query(
            options,
            vec![0x1c, 0x00],
            ResponseMatcher::PayloadPrefix(vec![0x1c, 0x00]),
        ),
        "split" => query(
            options,
            vec![0x0f],
            ResponseMatcher::PayloadPrefix(vec![0x0f]),
        ),
        "rit-offset" => query(
            options,
            vec![0x21, 0x00],
            ResponseMatcher::PayloadPrefix(vec![0x21, 0x00]),
        ),
        "rit" => query(
            options,
            vec![0x21, 0x01],
            ResponseMatcher::PayloadPrefix(vec![0x21, 0x01]),
        ),
        "xit" if profile.capabilities.rit_xit.xit_enabled.can_read() => query(
            options,
            vec![0x21, 0x02],
            ResponseMatcher::PayloadPrefix(vec![0x21, 0x02]),
        ),
        "filter-bandwidth" if profile.capabilities.main_rx.filter_bandwidth.can_read() => query(
            options,
            vec![0x1a, 0x03],
            ResponseMatcher::PayloadPrefix(vec![0x1a, 0x03]),
        ),
        "preamp-main" if profile.capabilities.main_rx.rf.preamp.can_read() => {
            receiver_query(profile, options, ReceiverPath::Main, vec![0x16, 0x02])
        }
        "preamp-sub"
            if receiver_capabilities(profile, ReceiverPath::Sub)
                .rf
                .preamp
                .can_read() =>
        {
            receiver_query(profile, options, ReceiverPath::Sub, vec![0x16, 0x02])
        }
        "attenuator-main" if profile.capabilities.main_rx.rf.attenuator.can_read() => {
            receiver_query(profile, options, ReceiverPath::Main, vec![0x11])
        }
        "attenuator-sub"
            if receiver_capabilities(profile, ReceiverPath::Sub)
                .rf
                .attenuator
                .can_read() =>
        {
            receiver_query(profile, options, ReceiverPath::Sub, vec![0x11])
        }
        "noise-blanker-main" if profile.capabilities.main_rx.rf.noise_blanker.can_read() => {
            receiver_query(profile, options, ReceiverPath::Main, vec![0x16, 0x22])
        }
        "noise-blanker-sub"
            if receiver_capabilities(profile, ReceiverPath::Sub)
                .rf
                .noise_blanker
                .can_read() =>
        {
            receiver_query(profile, options, ReceiverPath::Sub, vec![0x16, 0x22])
        }
        "noise-reduction-main" if profile.capabilities.main_rx.rf.noise_reduction.can_read() => {
            receiver_query(profile, options, ReceiverPath::Main, vec![0x16, 0x40])
        }
        "noise-reduction-sub"
            if receiver_capabilities(profile, ReceiverPath::Sub)
                .rf
                .noise_reduction
                .can_read() =>
        {
            receiver_query(profile, options, ReceiverPath::Sub, vec![0x16, 0x40])
        }
        "auto-notch-main" if profile.capabilities.main_rx.rf.auto_notch.can_read() => {
            receiver_query(profile, options, ReceiverPath::Main, vec![0x16, 0x41])
        }
        "auto-notch-sub"
            if receiver_capabilities(profile, ReceiverPath::Sub)
                .rf
                .auto_notch
                .can_read() =>
        {
            receiver_query(profile, options, ReceiverPath::Sub, vec![0x16, 0x41])
        }
        "tx-power" => query(
            options,
            vec![0x14, 0x0a],
            ResponseMatcher::PayloadPrefix(vec![0x14, 0x0a]),
        ),
        "keyer-speed" => query(
            options,
            vec![0x14, 0x0c],
            ResponseMatcher::PayloadPrefix(vec![0x14, 0x0c]),
        ),
        _ => return Ok(None),
    }?;

    Ok(Some(encoded))
}

pub fn decode(
    profile: &IcomCivProfile,
    frame: &CivFrame,
    state: &RadioState,
    receiver_hint: Option<ReceiverPath>,
) -> Result<Option<DecodedFrame>> {
    let payload = frame.payload();
    let patches = match payload {
        [0x29, target, inner @ ..] if profile.supports_command_29 => {
            let receiver = selector_receiver(*target)?;
            return decode(
                profile,
                &CivFrame::new(frame.to(), frame.from(), inner.to_vec())?,
                state,
                Some(receiver),
            );
        }
        [0x25, selector, freq @ ..] => decode_vfo_frequency(*selector, freq, state)?,
        [0x26, selector, mode, data_mode, _filter] => {
            decode_vfo_mode(profile, *selector, *mode, *data_mode, state)?
        }
        [0x1c, 0x03, freq @ ..] => vec![StatePatch::TxFrequency(Frequency::from_hz(
            decode_frequency_bcd(freq)?,
        ))],
        [0x1c, 0x00, value] => vec![StatePatch::Transmitting(decode_bool(*value, "PTT")?)],
        [0x0f, value] => vec![StatePatch::Split(matches!(*value, 0x01 | 0x11 | 0x12))],
        [0x21, 0x00, data @ ..] => vec![decode_rit_offset_patch(profile, decode_rit_offset(data)?)],
        [0x21, 0x01, value] => vec![StatePatch::MainRitEnabled(decode_bool(*value, "RIT")?)],
        [0x21, 0x02, value] => vec![StatePatch::XitEnabled(decode_bool(*value, "XIT")?)],
        [0x1a, 0x03, value] => decode_filter_bandwidth(*value, state)?,
        [0x16, 0x02, value] => vec![receiver_rf_patch(
            receiver_hint.unwrap_or(ReceiverPath::Main),
            ReceiverRfField::Preamp,
            decode_preamp(*value)?,
        )],
        [0x11, value] => vec![receiver_rf_patch(
            receiver_hint.unwrap_or(ReceiverPath::Main),
            ReceiverRfField::Attenuator,
            decode_attenuator(profile, *value)?,
        )],
        [0x16, 0x22, value] => vec![receiver_rf_patch(
            receiver_hint.unwrap_or(ReceiverPath::Main),
            ReceiverRfField::NoiseBlanker,
            decode_bool_level(*value, "noise-blanker")?,
        )],
        [0x16, 0x40, value] => vec![receiver_rf_patch(
            receiver_hint.unwrap_or(ReceiverPath::Main),
            ReceiverRfField::NoiseReduction,
            decode_bool_level(*value, "noise-reduction")?,
        )],
        [0x16, 0x41, value] => vec![receiver_auto_notch_patch(
            receiver_hint.unwrap_or(ReceiverPath::Main),
            decode_bool(*value, "auto-notch")?,
        )],
        [0x14, 0x0a, raw @ ..] => vec![StatePatch::TxPower(raw_to_power(
            profile,
            decode_bcd_decimal_0000_0255(raw)?,
        ))],
        [0x14, 0x0c, raw @ ..] => vec![StatePatch::KeyerSpeed(raw_to_wpm(
            decode_bcd_decimal_0000_0255(raw)?,
        ))],
        _ => return Ok(None),
    };

    Ok(Some(DecodedFrame::new(patches)))
}

#[derive(Debug, Clone, Copy)]
enum ReceiverRfField {
    Preamp,
    Attenuator,
    NoiseBlanker,
    NoiseReduction,
}

fn query(
    options: IcomCivOptions,
    payload: Vec<u8>,
    matcher: ResponseMatcher,
) -> Result<EncodedCommand> {
    Ok(EncodedCommand::new(
        vec![frame(options, payload)?],
        matcher,
        None,
        Vec::new(),
    ))
}

fn receiver_query(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    receiver: ReceiverPath,
    payload: Vec<u8>,
) -> Result<EncodedCommand> {
    if profile.supports_command_29 {
        let wrapped = wrap_command_29(receiver, payload.clone());
        return Ok(EncodedCommand::new(
            vec![frame(options, wrapped.clone())?],
            ResponseMatcher::OneOf(vec![wrapped, payload]),
            Some(receiver),
            Vec::new(),
        ));
    }

    require_main_receiver(receiver, "receiver")?;
    query(
        options,
        payload.clone(),
        ResponseMatcher::PayloadPrefix(payload),
    )
}

fn wrap_command_29(receiver: ReceiverPath, payload: Vec<u8>) -> Vec<u8> {
    let mut wrapped = vec![0x29, receiver_selector(receiver)];
    wrapped.extend_from_slice(&payload);
    wrapped
}

fn set_receiver_value(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    receiver: ReceiverPath,
    command: &[u8],
    value: u8,
    optimistic: StatePatch,
) -> Result<EncodedCommand> {
    let mut payload = command.to_vec();
    payload.push(value);

    let frame_payload = if profile.supports_command_29 {
        wrap_command_29(receiver, payload)
    } else {
        require_main_receiver(receiver, "receiver")?;
        payload
    };

    Ok(EncodedCommand::new(
        vec![frame(options, frame_payload)?],
        ResponseMatcher::Ack,
        Some(receiver),
        vec![optimistic],
    ))
}

fn require_receiver_capability(
    profile: &IcomCivProfile,
    receiver: ReceiverPath,
    capability: &'static str,
) -> Result<()> {
    let supported = match capability {
        "receiver.filter_bandwidth" => receiver_capabilities(profile, receiver).filter_bandwidth,
        "receiver.preamp" => receiver_capabilities(profile, receiver).rf.preamp,
        "receiver.attenuator" => receiver_capabilities(profile, receiver).rf.attenuator,
        "receiver.noise_blanker" => receiver_capabilities(profile, receiver).rf.noise_blanker,
        "receiver.noise_reduction" => receiver_capabilities(profile, receiver).rf.noise_reduction,
        "receiver.auto_notch" => receiver_capabilities(profile, receiver).rf.auto_notch,
        _ => {
            return Err(RadioError::UnsupportedCapability { capability });
        }
    };

    if supported.can_write() {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability { capability })
    }
}

fn receiver_capabilities(
    profile: &IcomCivProfile,
    receiver: ReceiverPath,
) -> &crate::ReceiverCapabilities {
    match receiver {
        ReceiverPath::Main => &profile.capabilities.main_rx,
        ReceiverPath::Sub => profile
            .capabilities
            .sub_rx
            .as_ref()
            .unwrap_or(&profile.capabilities.main_rx),
    }
}

fn require_xit(profile: &IcomCivProfile) -> Result<()> {
    if profile.capabilities.rit_xit.xit_enabled.can_write() {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability {
            capability: "rit_xit.xit_enabled",
        })
    }
}

fn receiver_rf_patch(
    receiver: ReceiverPath,
    field: ReceiverRfField,
    value: LeveledSetting,
) -> StatePatch {
    match (receiver, field) {
        (ReceiverPath::Main, ReceiverRfField::Preamp) => StatePatch::MainRxPreamp(value),
        (ReceiverPath::Sub, ReceiverRfField::Preamp) => StatePatch::SubRxPreamp(value),
        (ReceiverPath::Main, ReceiverRfField::Attenuator) => StatePatch::MainRxAttenuator(value),
        (ReceiverPath::Sub, ReceiverRfField::Attenuator) => StatePatch::SubRxAttenuator(value),
        (ReceiverPath::Main, ReceiverRfField::NoiseBlanker) => {
            StatePatch::MainRxNoiseBlanker(value)
        }
        (ReceiverPath::Sub, ReceiverRfField::NoiseBlanker) => StatePatch::SubRxNoiseBlanker(value),
        (ReceiverPath::Main, ReceiverRfField::NoiseReduction) => {
            StatePatch::MainRxNoiseReduction(value)
        }
        (ReceiverPath::Sub, ReceiverRfField::NoiseReduction) => {
            StatePatch::SubRxNoiseReduction(value)
        }
    }
}

fn receiver_auto_notch_patch(receiver: ReceiverPath, enabled: bool) -> StatePatch {
    match receiver {
        ReceiverPath::Main => StatePatch::MainRxAutoNotch(enabled),
        ReceiverPath::Sub => StatePatch::SubRxAutoNotch(enabled),
    }
}

fn decode_rit_offset_patch(profile: &IcomCivProfile, offset: RitXitOffsetHz) -> StatePatch {
    if profile.capabilities.rit_xit.xit_enabled.can_read() {
        StatePatch::RitXitOffset(offset)
    } else {
        StatePatch::RitOffset(offset)
    }
}

fn set_vfo_frequency(
    options: IcomCivOptions,
    selector: u8,
    frequency: Frequency,
    optimistic: Vec<StatePatch>,
) -> Result<EncodedCommand> {
    let mut payload = vec![0x25, selector];
    payload.extend_from_slice(&encode_frequency_bcd(frequency)?);
    Ok(EncodedCommand::new(
        vec![frame(options, payload)?],
        ResponseMatcher::Ack,
        None,
        optimistic,
    ))
}

fn set_vfo_mode(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    selector: u8,
    mode: Mode,
    optimistic: Vec<StatePatch>,
) -> Result<EncodedCommand> {
    let (mode_byte, data_mode) = encode_mode(profile, mode)?;
    Ok(EncodedCommand::new(
        vec![frame(
            options,
            vec![0x26, selector, mode_byte, data_mode, options.mode_filter],
        )?],
        ResponseMatcher::Ack,
        None,
        optimistic,
    ))
}

fn set_filter_bandwidth(
    options: IcomCivOptions,
    bandwidth_hz: u16,
    state: &RadioState,
) -> Result<EncodedCommand> {
    let mode = state.main_rx.mode.unwrap_or(Mode::Usb);
    let code = select_filter_width_code(mode, bandwidth_hz)?;
    let actual = filter_width_hz(mode, code)?;
    Ok(EncodedCommand::new(
        vec![frame(options, vec![0x1a, 0x03, encode_bcd_byte(code)?])?],
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::MainRxFilterBandwidth(actual)],
    ))
}

fn set_preamp(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    receiver: ReceiverPath,
    setting: LeveledSetting,
) -> Result<EncodedCommand> {
    let value = if setting.enabled == Some(false) {
        0x00
    } else {
        match setting.level.unwrap_or(1) {
            0 => 0x00,
            1 => 0x01,
            2 => 0x02,
            other => {
                return Err(RadioError::InvalidValue {
                    field: "preamp.level",
                    message: format!("expected 0, 1, or 2, got {other}"),
                })
            }
        }
    };

    set_receiver_value(
        profile,
        options,
        receiver,
        &[0x16, 0x02],
        value,
        receiver_rf_patch(receiver, ReceiverRfField::Preamp, decode_preamp(value)?),
    )
}

fn set_attenuator(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    receiver: ReceiverPath,
    setting: LeveledSetting,
) -> Result<EncodedCommand> {
    let value = encode_attenuator(profile, setting)?;
    set_receiver_value(
        profile,
        options,
        receiver,
        &[0x11],
        value,
        receiver_rf_patch(
            receiver,
            ReceiverRfField::Attenuator,
            decode_attenuator(profile, value)?,
        ),
    )
}

fn set_bool_level(
    options: IcomCivOptions,
    command: &[u8],
    enabled: bool,
    optimistic: Vec<StatePatch>,
) -> Result<EncodedCommand> {
    let mut payload = command.to_vec();
    payload.push(if enabled { 0x01 } else { 0x00 });
    Ok(EncodedCommand::new(
        vec![frame(options, payload)?],
        ResponseMatcher::Ack,
        None,
        optimistic,
    ))
}

fn set_receiver_bool<T>(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    receiver: ReceiverPath,
    command: &[u8],
    enabled: bool,
    _receiver_hint: T,
    optimistic: StatePatch,
) -> Result<EncodedCommand> {
    let value = if enabled { 0x01 } else { 0x00 };
    set_receiver_value(profile, options, receiver, command, value, optimistic)
}

fn set_tx_power(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    power: Power,
) -> Result<EncodedCommand> {
    let raw = power_to_raw(profile, power);
    let mut payload = vec![0x14, 0x0a];
    payload.extend_from_slice(&encode_bcd_decimal_0000_0255(raw)?);
    Ok(EncodedCommand::new(
        vec![frame(options, payload)?],
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::TxPower(raw_to_power(profile, raw))],
    ))
}

fn set_ptt(options: IcomCivOptions, enabled: bool) -> Result<EncodedCommand> {
    Ok(EncodedCommand::new(
        vec![frame(
            options,
            vec![0x1c, 0x00, if enabled { 0x01 } else { 0x00 }],
        )?],
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::Transmitting(enabled)],
    ))
}

fn set_split(options: IcomCivOptions, enabled: bool) -> Result<EncodedCommand> {
    Ok(EncodedCommand::new(
        vec![frame(
            options,
            vec![0x0f, if enabled { 0x01 } else { 0x00 }],
        )?],
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::Split(enabled)],
    ))
}

fn set_rit_offset(
    profile: &IcomCivProfile,
    options: IcomCivOptions,
    offset: RitXitOffsetHz,
    xit_requested: bool,
) -> Result<EncodedCommand> {
    let mut payload = vec![0x21, 0x00];
    payload.extend_from_slice(&encode_rit_offset(offset)?);
    Ok(EncodedCommand::new(
        vec![frame(options, payload)?],
        ResponseMatcher::Ack,
        None,
        vec![
            if xit_requested && profile.capabilities.rit_xit.xit_enabled.can_write() {
                StatePatch::RitXitOffset(offset)
            } else {
                StatePatch::RitOffset(offset)
            },
        ],
    ))
}

fn set_keyer_speed(options: IcomCivOptions, wpm: u8) -> Result<EncodedCommand> {
    let raw = wpm_to_raw(wpm)?;
    let mut payload = vec![0x14, 0x0c];
    payload.extend_from_slice(&encode_bcd_decimal_0000_0255(raw)?);
    Ok(EncodedCommand::new(
        vec![frame(options, payload)?],
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::KeyerSpeed(raw_to_wpm(raw))],
    ))
}

fn send_cw(options: IcomCivOptions, text: &str) -> Result<EncodedCommand> {
    let bytes = validate_cw_text(text)?;
    let mut frames = Vec::new();
    for chunk in bytes.chunks(30) {
        let mut payload = vec![0x17];
        payload.extend_from_slice(chunk);
        frames.push(frame(options, payload)?);
    }

    if frames.is_empty() {
        return Err(RadioError::InvalidValue {
            field: "cw",
            message: "cannot send an empty CW string".to_string(),
        });
    }

    Ok(EncodedCommand::new(
        frames,
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::KeyerSending(true)],
    ))
}

fn stop_cw(options: IcomCivOptions) -> Result<EncodedCommand> {
    Ok(EncodedCommand::new(
        vec![frame(options, vec![0x17, 0xff])?],
        ResponseMatcher::Ack,
        None,
        vec![StatePatch::KeyerSending(false)],
    ))
}

fn decode_vfo_frequency(selector: u8, data: &[u8], state: &RadioState) -> Result<Vec<StatePatch>> {
    let frequency = Frequency::from_hz(decode_frequency_bcd(data)?);
    let receiver = selector_receiver(selector)?;
    Ok(frequency_patches(receiver, frequency, state))
}

fn decode_vfo_mode(
    profile: &IcomCivProfile,
    selector: u8,
    mode_byte: u8,
    data_mode: u8,
    state: &RadioState,
) -> Result<Vec<StatePatch>> {
    let mode = decode_mode(profile, mode_byte, data_mode)?;
    let receiver = selector_receiver(selector)?;
    Ok(mode_patches(receiver, mode, state))
}

fn decode_filter_bandwidth(value: u8, state: &RadioState) -> Result<Vec<StatePatch>> {
    let mode = state.main_rx.mode.unwrap_or(Mode::Usb);
    let code = decode_bcd_byte(value)?;
    let bandwidth_hz = filter_width_hz(mode, code)?;
    Ok(vec![StatePatch::MainRxFilterBandwidth(bandwidth_hz)])
}

fn frequency_patches(
    receiver: ReceiverPath,
    frequency: Frequency,
    state: &RadioState,
) -> Vec<StatePatch> {
    let mut patches = match receiver {
        ReceiverPath::Main => vec![StatePatch::MainRxFrequency(frequency)],
        ReceiverPath::Sub => vec![StatePatch::SubRxFrequency(frequency)],
    };

    if tx_receiver_from_state(state) == receiver {
        patches.push(StatePatch::TxFrequency(frequency));
    }

    patches
}

fn mode_patches(receiver: ReceiverPath, mode: Mode, state: &RadioState) -> Vec<StatePatch> {
    let mut patches = match receiver {
        ReceiverPath::Main => vec![StatePatch::MainRxMode(mode)],
        ReceiverPath::Sub => vec![StatePatch::SubRxMode(mode)],
    };

    if tx_receiver_from_state(state) == receiver {
        patches.push(StatePatch::TxMode(mode));
    }

    patches
}

fn tx_receiver_from_state(state: &RadioState) -> ReceiverPath {
    if state.tx.as_ref().and_then(|tx| tx.split) == Some(true) {
        ReceiverPath::Sub
    } else {
        ReceiverPath::Main
    }
}

fn receiver_selector(receiver: ReceiverPath) -> u8 {
    match receiver {
        ReceiverPath::Main => 0x00,
        ReceiverPath::Sub => 0x01,
    }
}

fn selector_receiver(selector: u8) -> Result<ReceiverPath> {
    match selector {
        0x00 => Ok(ReceiverPath::Main),
        0x01 => Ok(ReceiverPath::Sub),
        other => Err(RadioError::Decode {
            command: "vfo-selector",
            message: format!("unsupported selected/unselected selector 0x{other:02x}"),
        }),
    }
}

fn frame(options: IcomCivOptions, payload: Vec<u8>) -> Result<CivFrame> {
    CivFrame::new(options.radio_address, options.controller_address, payload)
}

fn require_main_receiver(receiver: ReceiverPath, capability: &'static str) -> Result<()> {
    if matches!(receiver, ReceiverPath::Main) {
        Ok(())
    } else {
        Err(RadioError::UnsupportedCapability { capability })
    }
}

fn encode_frequency_bcd(frequency: Frequency) -> Result<[u8; 5]> {
    if frequency.hz() > 9_999_999_999 {
        return Err(RadioError::InvalidValue {
            field: "frequency",
            message: format!(
                "frequency {} Hz exceeds 10-digit CI-V BCD field",
                frequency.hz()
            ),
        });
    }

    let digits = format!("{:010}", frequency.hz());
    let bytes = digits.as_bytes();
    Ok([
        pair_to_bcd(bytes[8], bytes[9])?,
        pair_to_bcd(bytes[6], bytes[7])?,
        pair_to_bcd(bytes[4], bytes[5])?,
        pair_to_bcd(bytes[2], bytes[3])?,
        pair_to_bcd(bytes[0], bytes[1])?,
    ])
}

fn decode_frequency_bcd(bytes: &[u8]) -> Result<u64> {
    if bytes.len() != 5 {
        return Err(RadioError::Decode {
            command: "frequency",
            message: format!("expected 5 BCD bytes, got {}", bytes.len()),
        });
    }

    let mut digits = String::with_capacity(10);
    for byte in bytes.iter().rev() {
        let value = decode_bcd_byte(*byte)?;
        digits.push(char::from(b'0' + value / 10));
        digits.push(char::from(b'0' + value % 10));
    }

    digits.parse::<u64>().map_err(|error| RadioError::Decode {
        command: "frequency",
        message: error.to_string(),
    })
}

fn encode_mode(profile: &IcomCivProfile, mode: Mode) -> Result<(u8, u8)> {
    let (base_mode, data_mode) = match mode {
        Mode::DataLsb => (Mode::Lsb, 0x01),
        Mode::DataUsb => (Mode::Usb, 0x01),
        Mode::DataFm => (Mode::Fm, 0x01),
        Mode::DataAm => (Mode::Am, 0x01),
        other => (other, 0x00),
    };

    let mode_byte = profile
        .mode_map
        .iter()
        .find_map(|(raw, mapped)| (*mapped == base_mode).then_some(*raw))
        .ok_or_else(|| RadioError::InvalidValue {
            field: "mode",
            message: format!("mode {mode} is not supported by {}", profile.id()),
        })?;

    Ok((mode_byte, data_mode))
}

fn decode_mode(profile: &IcomCivProfile, mode: u8, data_mode: u8) -> Result<Mode> {
    let base_mode = profile
        .mode_map
        .iter()
        .find_map(|(raw, mapped)| (*raw == mode).then_some(*mapped))
        .ok_or_else(|| RadioError::Decode {
            command: "mode",
            message: format!("unsupported {} mode byte 0x{mode:02x}", profile.id()),
        })?;

    if data_mode == 0x00 {
        return Ok(base_mode);
    }

    match base_mode {
        Mode::Lsb => Ok(Mode::DataLsb),
        Mode::Usb => Ok(Mode::DataUsb),
        Mode::Am => Ok(Mode::DataAm),
        Mode::Fm => Ok(Mode::DataFm),
        _ => Ok(base_mode),
    }
}

fn decode_bool(value: u8, command: &'static str) -> Result<bool> {
    match value {
        0x00 => Ok(false),
        0x01 => Ok(true),
        other => Err(RadioError::Decode {
            command,
            message: format!("expected 00 or 01, got 0x{other:02x}"),
        }),
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

fn decode_bool_level(value: u8, command: &'static str) -> Result<LeveledSetting> {
    Ok(if decode_bool(value, command)? {
        LeveledSetting::enabled(1)
    } else {
        LeveledSetting::disabled()
    })
}

fn decode_preamp(value: u8) -> Result<LeveledSetting> {
    match value {
        0x00 => Ok(LeveledSetting::disabled()),
        0x01 => Ok(LeveledSetting::enabled(1)),
        0x02 => Ok(LeveledSetting::enabled(2)),
        other => Err(RadioError::Decode {
            command: "preamp",
            message: format!("unsupported preamp byte 0x{other:02x}"),
        }),
    }
}

fn encode_attenuator(profile: &IcomCivProfile, setting: LeveledSetting) -> Result<u8> {
    if !setting_enabled(setting) {
        return Ok(0x00);
    }

    let desired = setting.level.unwrap_or_else(|| {
        profile
            .attenuator_values_db
            .iter()
            .copied()
            .find(|value| *value > 0)
            .unwrap_or(0)
    });
    if desired == 0 {
        return Ok(0x00);
    }

    if profile.attenuator_values_db.contains(&desired) {
        Ok(desired)
    } else {
        Err(RadioError::InvalidValue {
            field: "attenuator.level",
            message: format!(
                "attenuator level {desired} dB is not supported by {}",
                profile.id()
            ),
        })
    }
}

fn decode_attenuator(profile: &IcomCivProfile, value: u8) -> Result<LeveledSetting> {
    if value == 0x00 {
        return Ok(LeveledSetting::disabled());
    }

    if profile.attenuator_values_db.contains(&value) {
        Ok(LeveledSetting::enabled(value))
    } else {
        Err(RadioError::Decode {
            command: "attenuator",
            message: format!(
                "unsupported attenuator value 0x{value:02x} for {}",
                profile.id()
            ),
        })
    }
}

fn encode_rit_offset(offset: RitXitOffsetHz) -> Result<[u8; 3]> {
    let value = offset.as_hz();
    let abs = value.unsigned_abs();
    if abs > 9_999 {
        return Err(RadioError::InvalidValue {
            field: "rit.offset",
            message: format!("offset {value} is outside CI-V RIT range"),
        });
    }

    let digits = format!("{abs:04}");
    let bytes = digits.as_bytes();
    Ok([
        pair_to_bcd(bytes[2], bytes[3])?,
        pair_to_bcd(bytes[0], bytes[1])?,
        if value < 0 { 0x01 } else { 0x00 },
    ])
}

fn decode_rit_offset(bytes: &[u8]) -> Result<RitXitOffsetHz> {
    if bytes.len() != 3 {
        return Err(RadioError::Decode {
            command: "rit-offset",
            message: format!("expected 3 bytes, got {}", bytes.len()),
        });
    }

    let low = decode_bcd_byte(bytes[0])? as i16;
    let high = decode_bcd_byte(bytes[1])? as i16;
    let magnitude = high * 100 + low;
    let value = match bytes[2] {
        0x00 => magnitude,
        0x01 => -magnitude,
        other => {
            return Err(RadioError::Decode {
                command: "rit-offset",
                message: format!("unsupported sign byte 0x{other:02x}"),
            })
        }
    };

    RitXitOffsetHz::new(value).map_err(|error| RadioError::Decode {
        command: "rit-offset",
        message: error.to_string(),
    })
}

fn encode_bcd_decimal_0000_0255(value: u16) -> Result<[u8; 2]> {
    if value > 255 {
        return Err(RadioError::InvalidValue {
            field: "bcd-decimal",
            message: format!("expected 0..=255, got {value}"),
        });
    }

    let digits = format!("{value:04}");
    let bytes = digits.as_bytes();
    Ok([
        pair_to_bcd(bytes[0], bytes[1])?,
        pair_to_bcd(bytes[2], bytes[3])?,
    ])
}

fn decode_bcd_decimal_0000_0255(bytes: &[u8]) -> Result<u16> {
    if bytes.len() != 2 {
        return Err(RadioError::Decode {
            command: "bcd-decimal",
            message: format!("expected 2 bytes, got {}", bytes.len()),
        });
    }

    let value = decode_bcd_byte(bytes[0])? as u16 * 100 + decode_bcd_byte(bytes[1])? as u16;
    if value > 255 {
        return Err(RadioError::Decode {
            command: "bcd-decimal",
            message: format!("expected 0000..0255, got {value:04}"),
        });
    }
    Ok(value)
}

fn encode_bcd_byte(value: u8) -> Result<u8> {
    if value > 99 {
        return Err(RadioError::InvalidValue {
            field: "bcd-byte",
            message: format!("expected 0..=99, got {value}"),
        });
    }
    Ok(((value / 10) << 4) | (value % 10))
}

fn decode_bcd_byte(value: u8) -> Result<u8> {
    let high = value >> 4;
    let low = value & 0x0f;
    if high > 9 || low > 9 {
        return Err(RadioError::Decode {
            command: "bcd",
            message: format!("invalid BCD byte 0x{value:02x}"),
        });
    }
    Ok(high * 10 + low)
}

fn pair_to_bcd(tens: u8, ones: u8) -> Result<u8> {
    if !tens.is_ascii_digit() || !ones.is_ascii_digit() {
        return Err(RadioError::InvalidValue {
            field: "bcd",
            message: "non-digit in BCD encoder".to_string(),
        });
    }
    Ok(((tens - b'0') << 4) | (ones - b'0'))
}

fn select_filter_width_code(mode: Mode, requested_hz: u16) -> Result<u8> {
    let max = match filter_family(mode)? {
        FilterFamily::SsbCw => 40,
        FilterFamily::Rtty => 31,
        FilterFamily::Am => 49,
    };

    let mut best_code = 0;
    let mut best_delta = u16::MAX;
    for code in 0..=max {
        let width = filter_width_hz(mode, code)?;
        let delta = width.abs_diff(requested_hz);
        if delta < best_delta {
            best_code = code;
            best_delta = delta;
        }
    }
    Ok(best_code)
}

fn filter_width_hz(mode: Mode, code: u8) -> Result<u16> {
    match filter_family(mode)? {
        FilterFamily::SsbCw => match code {
            0..=9 => Ok((code as u16 + 1) * 50),
            10..=40 => Ok(600 + (code as u16 - 10) * 100),
            _ => Err(invalid_filter_code(code, mode)),
        },
        FilterFamily::Rtty => match code {
            0..=9 => Ok((code as u16 + 1) * 50),
            10..=31 => Ok(600 + (code as u16 - 10) * 100),
            _ => Err(invalid_filter_code(code, mode)),
        },
        FilterFamily::Am => match code {
            0..=49 => Ok((code as u16 + 1) * 200),
            _ => Err(invalid_filter_code(code, mode)),
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum FilterFamily {
    SsbCw,
    Rtty,
    Am,
}

fn filter_family(mode: Mode) -> Result<FilterFamily> {
    match mode {
        Mode::Lsb
        | Mode::Usb
        | Mode::DataLsb
        | Mode::DataUsb
        | Mode::Cw
        | Mode::CwReverse
        | Mode::Psk
        | Mode::PskReverse => Ok(FilterFamily::SsbCw),
        Mode::Rtty | Mode::RttyReverse => Ok(FilterFamily::Rtty),
        Mode::Am | Mode::DataAm => Ok(FilterFamily::Am),
        other => Err(RadioError::UnsupportedCapability {
            capability: match other {
                Mode::Fm | Mode::DataFm | Mode::Wfm | Mode::DigitalVoice => {
                    "receiver.filter_bandwidth"
                }
                _ => "receiver.filter_bandwidth",
            },
        }),
    }
}

fn invalid_filter_code(code: u8, mode: Mode) -> RadioError {
    RadioError::Decode {
        command: "filter-bandwidth",
        message: format!("unsupported filter code {code} for {mode}"),
    }
}

fn power_to_raw(profile: &IcomCivProfile, power: Power) -> u16 {
    let max_microwatts = profile.max_tx_power_watts as u64 * 1_000_000;
    let requested = power.as_microwatts().min(max_microwatts);
    ((requested * 255 + max_microwatts / 2) / max_microwatts) as u16
}

fn raw_to_power(profile: &IcomCivProfile, raw: u16) -> Power {
    let max_milliwatts = profile.max_tx_power_watts as u64 * 1_000;
    let milliwatts = (raw as u64 * max_milliwatts + 127) / 255;
    if milliwatts % 1_000 == 0 || milliwatts > u16::MAX as u64 {
        Power::from_watts(((milliwatts + 500) / 1_000) as u16)
    } else {
        Power::new(milliwatts as u16, PowerUnit::Milliwatts)
    }
}

fn wpm_to_raw(wpm: u8) -> Result<u16> {
    if !(6..=48).contains(&wpm) {
        return Err(RadioError::InvalidValue {
            field: "keyer.speed_wpm",
            message: format!("expected 6..=48 WPM, got {wpm}"),
        });
    }
    Ok(((u16::from(wpm - 6) * 255 + 21) / 42).min(255))
}

fn raw_to_wpm(raw: u16) -> u8 {
    (6 + ((raw.min(255) * 42 + 127) / 255)) as u8
}

fn validate_cw_text(text: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.bytes() {
        let valid = ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                b' ' | b'/'
                    | b'?'
                    | b'.'
                    | b'-'
                    | b','
                    | b':'
                    | b'\''
                    | b'('
                    | b')'
                    | b'='
                    | b'+'
                    | b'"'
                    | b'@'
                    | b'^'
            );
        if !valid {
            return Err(RadioError::InvalidValue {
                field: "cw",
                message: format!("unsupported CW character {:?}", ch as char),
            });
        }
        bytes.push(ch);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectionState, ReceiverState, TransmitterState};
    use std::time::Duration;

    fn profile() -> &'static IcomCivProfile {
        crate::protocol::icom_civ::profile_by_id("icom-ic705").unwrap()
    }

    fn options() -> IcomCivOptions {
        IcomCivOptions {
            radio_address: 0xa4,
            controller_address: 0xe0,
            mode_filter: 3,
            poll_interval: Duration::from_millis(200),
        }
    }

    #[test]
    fn frequency_bcd_matches_ic705_examples() {
        assert_eq!(
            encode_frequency_bcd(Frequency::from_hz(14_074_000)).unwrap(),
            [0x00, 0x40, 0x07, 0x14, 0x00]
        );
        assert_eq!(
            encode_frequency_bcd(Frequency::from_hz(145_500_000)).unwrap(),
            [0x00, 0x00, 0x50, 0x45, 0x01]
        );
        assert_eq!(
            decode_frequency_bcd(&[0x00, 0x00, 0x03, 0x07, 0x00]).unwrap(),
            7_030_000
        );
    }

    #[test]
    fn rit_offset_bcd_round_trips() {
        assert_eq!(
            encode_rit_offset(RitXitOffsetHz::new(1234).unwrap()).unwrap(),
            [0x34, 0x12, 0x00]
        );
        assert_eq!(
            encode_rit_offset(RitXitOffsetHz::new(-250).unwrap()).unwrap(),
            [0x50, 0x02, 0x01]
        );
        assert_eq!(
            decode_rit_offset(&[0x50, 0x02, 0x01]).unwrap(),
            RitXitOffsetHz::new(-250).unwrap()
        );
    }

    #[test]
    fn xit_and_shared_offset_commands_encode_identically_for_shared_radios() {
        let target = RitXitOffsetHz::new(250).unwrap();

        let xit = encode(
            profile(),
            options(),
            &RadioCommand::SetXitOffset(target),
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();
        let both = encode(
            profile(),
            options(),
            &RadioCommand::SetRitXitOffset(target),
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(xit.frames, both.frames);
        assert_eq!(xit.optimistic, both.optimistic);
    }

    #[test]
    fn rit_only_profiles_reject_xit_commands() {
        let ic7100 = crate::protocol::icom_civ::profile_by_id("icom-ic7100").unwrap();
        let result = encode(
            ic7100,
            IcomCivOptions::defaults(ic7100),
            &RadioCommand::SetXitEnabled(true),
            &RadioState::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn encode_frequency_targets_selected_and_unselected_vfos() {
        let encoded = encode(
            profile(),
            options(),
            &RadioCommand::SetReceiverFrequency {
                receiver: ReceiverPath::Sub,
                frequency: Frequency::from_hz(7_030_000),
            },
            &RadioState::default(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            encoded.frames[0].as_bytes(),
            &[0xfe, 0xfe, 0xa4, 0xe0, 0x25, 0x01, 0x00, 0x00, 0x03, 0x07, 0x00, 0xfd]
        );
        assert_eq!(encoded.matcher, ResponseMatcher::Ack);
    }

    #[test]
    fn modes_include_wfm_and_digital_voice() {
        let ic7100 = crate::protocol::icom_civ::profile_by_id("icom-ic7100").unwrap();
        assert_eq!(encode_mode(ic7100, Mode::Wfm).unwrap(), (0x06, 0x00));
        assert_eq!(
            encode_mode(ic7100, Mode::DigitalVoice).unwrap(),
            (0x17, 0x00)
        );
        assert_eq!(decode_mode(ic7100, 0x06, 0x00).unwrap(), Mode::Wfm);
        assert_eq!(decode_mode(ic7100, 0x17, 0x00).unwrap(), Mode::DigitalVoice);
        assert_eq!(decode_mode(ic7100, 0x01, 0x01).unwrap(), Mode::DataUsb);

        let ic7610 = crate::protocol::icom_civ::profile_by_id("icom-ic7610").unwrap();
        assert_eq!(encode_mode(ic7610, Mode::Psk).unwrap(), (0x12, 0x00));
        assert_eq!(decode_mode(ic7610, 0x17, 0x00).unwrap(), Mode::PskReverse);

        let ic7760 = crate::protocol::icom_civ::profile_by_id("icom-ic7760").unwrap();
        assert_eq!(encode_mode(ic7760, Mode::PskReverse).unwrap(), (0x13, 0x00));
        assert_eq!(decode_mode(ic7760, 0x13, 0x00).unwrap(), Mode::PskReverse);
    }

    #[test]
    fn decodes_vfo_frequency_and_mode_into_state_patches() {
        let state = RadioState {
            connection: ConnectionState::Ready,
            main_rx: ReceiverState::default(),
            sub_rx: Some(ReceiverState::default()),
            tx: Some(TransmitterState {
                split: Some(true),
                ..TransmitterState::default()
            }),
            ..RadioState::default()
        };

        let frame = CivFrame::new(0xe0, 0xa4, [0x25, 0x01, 0x00, 0x00, 0x03, 0x07, 0x00]).unwrap();
        let decoded = decode(profile(), &frame, &state, None).unwrap().unwrap();
        assert_eq!(
            decoded.patches,
            vec![
                StatePatch::SubRxFrequency(Frequency::from_hz(7_030_000)),
                StatePatch::TxFrequency(Frequency::from_hz(7_030_000)),
            ]
        );

        let frame = CivFrame::new(0xe0, 0xa4, [0x26, 0x00, 0x17, 0x00, 0x03]).unwrap();
        let decoded = decode(profile(), &frame, &state, None).unwrap().unwrap();
        assert!(decoded
            .patches
            .contains(&StatePatch::MainRxMode(Mode::DigitalVoice)));
    }

    #[test]
    fn attenuator_mapping_follows_profile_tables() {
        let ic7100 = crate::protocol::icom_civ::profile_by_id("icom-ic7100").unwrap();
        assert_eq!(
            decode_attenuator(ic7100, 12).unwrap(),
            LeveledSetting::enabled(12)
        );

        let ic7610 = crate::protocol::icom_civ::profile_by_id("icom-ic7610").unwrap();
        assert_eq!(
            encode_attenuator(ic7610, LeveledSetting::enabled(15)).unwrap(),
            15
        );
        assert_eq!(
            encode_attenuator(ic7610, LeveledSetting::enabled(24)).unwrap(),
            24
        );
        assert!(encode_attenuator(ic7610, LeveledSetting::enabled(20)).is_err());
        assert!(encode_attenuator(ic7610, LeveledSetting::enabled(27)).is_err());
    }

    #[test]
    fn ic7610_and_ic7760_expose_filter_bandwidth_query_and_command() {
        let state = RadioState {
            main_rx: ReceiverState {
                mode: Some(Mode::Usb),
                ..ReceiverState::default()
            },
            ..RadioState::default()
        };

        for id in ["icom-ic7610", "icom-ic7760"] {
            let profile = crate::protocol::icom_civ::profile_by_id(id).unwrap();
            let options = IcomCivOptions::defaults(profile);

            let query = encode_query(profile, options, "filter-bandwidth")
                .unwrap()
                .unwrap();
            assert_eq!(
                query.frames[0].as_bytes(),
                &[0xfe, 0xfe, options.radio_address, options.controller_address, 0x1a, 0x03, 0xfd]
            );

            let command = encode(
                profile,
                options,
                &RadioCommand::SetReceiverFilterBandwidth {
                    receiver: ReceiverPath::Main,
                    bandwidth_hz: 2_400,
                },
                &state,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                command.frames[0].as_bytes(),
                &[0xfe, 0xfe, options.radio_address, options.controller_address, 0x1a, 0x03, 0x28, 0xfd]
            );
        }
    }

    #[test]
    fn ic7610_exposes_main_rf_queries() {
        let profile = crate::protocol::icom_civ::profile_by_id("icom-ic7610").unwrap();
        let options = IcomCivOptions::defaults(profile);

        let preamp = encode_query(profile, options, "preamp-main").unwrap().unwrap();
        assert_eq!(
            preamp.frames[0].as_bytes(),
            &[0xfe, 0xfe, options.radio_address, options.controller_address, 0x16, 0x02, 0xfd]
        );

        let attenuator = encode_query(profile, options, "attenuator-main")
            .unwrap()
            .unwrap();
        assert_eq!(
            attenuator.frames[0].as_bytes(),
            &[0xfe, 0xfe, options.radio_address, options.controller_address, 0x11, 0xfd]
        );
    }

    #[test]
    fn ic7760_receiver_queries_and_wrapped_responses_target_sub_receiver() {
        let profile = crate::protocol::icom_civ::profile_by_id("icom-ic7760").unwrap();
        let encoded = encode_query(profile, IcomCivOptions::defaults(profile), "preamp-sub")
            .unwrap()
            .unwrap();
        assert_eq!(encoded.response_receiver, Some(ReceiverPath::Sub));
        assert!(matches!(encoded.matcher, ResponseMatcher::OneOf(_)));

        let state = RadioState {
            sub_rx: Some(ReceiverState::default()),
            ..RadioState::default()
        };
        let wrapped = CivFrame::new(0xe0, 0xb2, [0x29, 0x01, 0x16, 0x02, 0x01]).unwrap();
        let decoded = decode(profile, &wrapped, &state, None).unwrap().unwrap();
        assert_eq!(
            decoded.patches,
            vec![StatePatch::SubRxPreamp(LeveledSetting::enabled(1))]
        );

        let unwrapped = CivFrame::new(0xe0, 0xb2, [0x16, 0x02, 0x01]).unwrap();
        let decoded = decode(profile, &unwrapped, &state, Some(ReceiverPath::Sub))
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded.patches,
            vec![StatePatch::SubRxPreamp(LeveledSetting::enabled(1))]
        );
    }

    #[test]
    fn tx_power_maps_over_ic705_ten_watt_scale() {
        assert_eq!(power_to_raw(profile(), Power::from_watts(10)), 255);
        assert_eq!(raw_to_power(profile(), 255), Power::from_watts(10));
        assert_eq!(raw_to_power(profile(), 128).unit(), PowerUnit::Milliwatts);

        let ic7760 = crate::protocol::icom_civ::profile_by_id("icom-ic7760").unwrap();
        assert_eq!(raw_to_power(ic7760, 255), Power::from_watts(200));
    }

    #[test]
    fn cw_text_is_chunked_to_thirty_byte_frames() {
        let encoded = send_cw(options(), "ABCDEFGHIJKLMNOPQRSTUVWXYZ12345").unwrap();
        assert_eq!(encoded.frames.len(), 2);
        assert_eq!(encoded.frames[0].payload().len(), 31);
        assert_eq!(encoded.frames[1].payload().len(), 2);
    }
}
