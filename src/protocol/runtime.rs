use async_trait::async_trait;
use tokio::time::{Duration, Instant, timeout};

use crate::{
    ConnectionState, KeyerState, RadioCapabilities, RadioCommand, RadioState, ReceiverPath,
    ReceiverState, RitXitState, StatePatch, TransmitterState, UpdateSource,
    driver::{CommandCompletion, DriverDescriptor, RadioSession, StateSink},
    error::{RadioError, Result},
    protocol::{
        icom_civ::{
            self, CivFrame, FrameSplitter as IcomFrameSplitter, IcomCivOptions, IcomCivProfile,
            ResponseMatcher as IcomResponseMatcher,
        },
        kenwood_ascii::{
            self, AsciiFrame, CommandPriority, EncodedCommand,
            FrameSplitter as KenwoodFrameSplitter, KenwoodAsciiOptions, KenwoodAsciiProfile,
            ResponseMatcher, StartupStep,
        },
        smartsdr::{self, LineSplitter as SmartSdrLineSplitter, SmartSdrProfile},
    },
    transport::CatTransport,
};

const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const SMARTSDR_STARTUP_TIMEOUT: Duration = Duration::from_millis(1_500);
const TRANSPORT_IO_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) fn kenwood_session(
    profile: &'static KenwoodAsciiProfile,
    options: KenwoodAsciiOptions,
) -> Result<Box<dyn RadioSession>> {
    Ok(Box::new(KenwoodAsciiRuntime::new(profile, options)))
}

pub(crate) fn icom_session(
    profile: &'static IcomCivProfile,
    options: IcomCivOptions,
) -> Result<Box<dyn RadioSession>> {
    Ok(Box::new(IcomCivRuntime::new(profile, options)))
}

pub(crate) fn smartsdr_session(
    profile: &'static SmartSdrProfile,
    options: smartsdr::SmartSdrOptions,
) -> Result<Box<dyn RadioSession>> {
    let mut profile = *profile;
    profile.slice = options.slice;
    Ok(Box::new(SmartSdrRuntime::new(profile)))
}

/// Validate declarative query plans when built-in profiles are registered.
/// A `None` query encoder result is acceptable for ad-hoc recovery work, but
/// never for a static startup or poll entry: that would silently omit a
/// profile typo from a real connection.
pub(crate) fn validate_kenwood_profile(profile: &'static KenwoodAsciiProfile) -> Result<()> {
    let routing = kenwood_ascii::VfoRouting::for_profile(profile);
    for step in profile.startup {
        if let StartupStep::Query(semantic) = *step
            && encode_kenwood_query(profile, semantic, routing)?.is_none()
        {
            return Err(RadioError::InvalidValue {
                field: "profile.startup",
                message: format!("{} has an unencodable query {semantic:?}", profile.id()),
            });
        }
    }
    if let Some(plan) = profile.poll {
        if plan.queries.is_empty() {
            return Err(RadioError::InvalidValue {
                field: "profile.poll",
                message: format!("{} has an empty poll plan", profile.id()),
            });
        }
        for semantic in plan.queries {
            if encode_kenwood_query(profile, semantic, routing)?.is_none() {
                return Err(RadioError::InvalidValue {
                    field: "profile.poll",
                    message: format!("{} has an unencodable query {semantic:?}", profile.id()),
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_icom_profile(profile: &'static IcomCivProfile) -> Result<()> {
    let options = IcomCivOptions::defaults(profile);
    for step in profile.startup {
        let icom_civ::StartupStep::Query(semantic) = *step;
        if icom_civ::encode_query(profile, options, semantic)?.is_none() {
            return Err(RadioError::InvalidValue {
                field: "profile.startup",
                message: format!("{} has an unencodable query {semantic:?}", profile.id()),
            });
        }
    }
    if let Some(plan) = profile.poll {
        if plan.queries.is_empty() {
            return Err(RadioError::InvalidValue {
                field: "profile.poll",
                message: format!("{} has an empty poll plan", profile.id()),
            });
        }
        for semantic in plan.queries {
            if icom_civ::encode_query(profile, options, semantic)?.is_none() {
                return Err(RadioError::InvalidValue {
                    field: "profile.poll",
                    message: format!("{} has an unencodable query {semantic:?}", profile.id()),
                });
            }
        }
    }
    Ok(())
}

struct KenwoodAsciiRuntime {
    profile: &'static KenwoodAsciiProfile,
    options: KenwoodAsciiOptions,
    frame_splitter: KenwoodFrameSplitter,
    vfo_routing: kenwood_ascii::VfoRouting,
    poll_index: usize,
}

impl KenwoodAsciiRuntime {
    fn new(profile: &'static KenwoodAsciiProfile, options: KenwoodAsciiOptions) -> Self {
        Self {
            profile,
            options,
            frame_splitter: KenwoodFrameSplitter::new(),
            vfo_routing: kenwood_ascii::VfoRouting::for_profile(profile),
            poll_index: 0,
        }
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: EncodedCommand,
        default_source: UpdateSource,
        _wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        let mut completion = CommandCompletion::Written;
        for step in encoded.steps {
            let mut busy_retries = step.busy_retries;
            loop {
                let frame = &step.frame;

                tracing::debug!(
                    driver = %self.profile.id(),
                    tx_frame = frame.as_str(),
                    priority = ?step.priority,
                    busy_retries,
                    "sending CAT transaction step"
                );

                timeout(TRANSPORT_IO_TIMEOUT, transport.write_all(frame.as_bytes()))
                    .await
                    .map_err(|_| RadioError::Timeout {
                        command: "transport-write",
                    })??;
                timeout(TRANSPORT_IO_TIMEOUT, transport.flush())
                    .await
                    .map_err(|_| RadioError::Timeout {
                        command: "transport-flush",
                    })??;

                if !step.expected.expects_response() {
                    break;
                }

                let result = self
                    .process_incoming_with_expected(
                        transport,
                        step.timeout,
                        default_source,
                        Some(&step.expected),
                        step.decode_required,
                        ctx,
                    )
                    .await;
                match result {
                    Ok(true) => {
                        completion = match step.completion {
                            kenwood_ascii::StepCompletion::Written => completion,
                            kenwood_ascii::StepCompletion::Matched => CommandCompletion::Accepted,
                            kenwood_ascii::StepCompletion::Decoded => CommandCompletion::Observed,
                        };
                        break;
                    }
                    Ok(false) => {
                        return Err(RadioError::Timeout {
                            command: "kenwood-step-response",
                        });
                    }
                    Err(RadioError::ProtocolBusy) if busy_retries > 0 => {
                        busy_retries -= 1;
                        tokio::time::sleep(step.busy_retry_delay).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        Ok(completion)
    }

    async fn process_incoming_with_expected(
        &mut self,
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        expected: Option<&ResponseMatcher>,
        decode_required: bool,
        ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        let deadline = Instant::now() + wait_timeout;
        let mut saw_frames = false;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }

            let remaining = deadline.saturating_duration_since(now);
            let mut buf = [0u8; 1024];
            let count = match timeout(remaining, transport.read_some(&mut buf)).await {
                Ok(read_result) => read_result?,
                Err(_) => return Ok(false),
            };

            if count == 0 {
                return Err(RadioError::Transport(
                    "connection closed by peer".to_string(),
                ));
            }

            let mut matched_expected_in_batch = false;
            let frames = match self.frame_splitter.push(&buf[..count]) {
                Ok(frames) => frames,
                Err(error) => {
                    tracing::warn!(
                        driver = %self.profile.id(),
                        ?error,
                        "failed to split incoming CAT bytes into frames; dropping chunk"
                    );
                    continue;
                }
            };

            for frame in frames {
                saw_frames = true;

                if let Some(protocol_error) = kenwood_ascii::ProtocolErrorFrame::parse(&frame) {
                    tracing::warn!(
                        driver = %self.profile.id(),
                        rx_frame = frame.as_str(),
                        ?protocol_error,
                        "received CAT protocol error frame"
                    );
                    if expected.is_some() {
                        return Err(protocol_error.to_error());
                    }
                    continue;
                }

                let matched_expected = if let Some(expected) = expected
                    && expected.matches(&frame)
                {
                    tracing::debug!(
                        driver = %self.profile.id(),
                        rx_frame = frame.as_str(),
                        expected = ?expected,
                        "received expected CAT response"
                    );
                    true
                } else {
                    false
                };
                matched_expected_in_batch |= matched_expected;

                match decode_kenwood_frame(self.profile, &frame, ctx.state(), &mut self.vfo_routing)
                {
                    Ok(Some(decoded)) => {
                        let source = if matched_expected {
                            default_source
                        } else {
                            UpdateSource::Native
                        };
                        tracing::debug!(
                            driver = %self.profile.id(),
                            rx_frame = frame.as_str(),
                            source = ?source,
                            patch_count = decoded.patches.len(),
                            "decoded CAT frame into state patches"
                        );
                        ctx.publish_patches(decoded.patches, source);
                    }
                    Ok(None) => {
                        if matched_expected && decode_required {
                            return Err(RadioError::Decode {
                                command: "kenwood-response",
                                message: "matched response was not decoded".to_string(),
                            });
                        }
                        tracing::trace!(
                            driver = %self.profile.id(),
                            rx_frame = frame.as_str(),
                            "unhandled CAT frame"
                        );
                    }
                    Err(error) => {
                        if matched_expected {
                            return Err(error);
                        }
                        tracing::warn!(
                            driver = %self.profile.id(),
                            rx_frame = frame.as_str(),
                            ?error,
                            "failed to decode CAT response frame"
                        );
                    }
                }
            }

            if expected.is_none() {
                return Ok(saw_frames);
            }

            if matched_expected_in_batch {
                return Ok(true);
            }
        }
    }
}

#[async_trait]
impl RadioSession for KenwoodAsciiRuntime {
    fn descriptor(&self) -> DriverDescriptor {
        self.profile.descriptor
    }

    fn capabilities(&self) -> RadioCapabilities {
        self.profile.capabilities
    }

    fn initial_state(&self) -> RadioState {
        RadioState {
            connection: ConnectionState::Connecting,
            main_rx: ReceiverState::default(),
            sub_rx: match self.profile.receiver_kind {
                kenwood_ascii::ReceiverKind::SingleVfo => None,
                kenwood_ascii::ReceiverKind::DualVfo | kenwood_ascii::ReceiverKind::DualRx => {
                    Some(ReceiverState::default())
                }
            },
            tx: self
                .profile
                .capabilities
                .tx
                .map(|_| TransmitterState::default()),
            rit_xit: RitXitState::default(),
            keyer: self
                .profile
                .capabilities
                .keyer
                .map(|_| KeyerState::default()),
        }
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.profile.poll.map(|plan| plan.interval)
    }

    async fn startup(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Identifying)],
            UpdateSource::Native,
        );
        let Some(transport) = transport else {
            return Ok(());
        };
        tracing::info!(
            driver = %self.profile.id(),
            startup_steps = self.profile.startup.len(),
            "running kenwood-ascii startup sequence"
        );
        let mut responsive = false;

        for step in self.profile.startup {
            match *step {
                StartupStep::AutoInfo(frame_text) => {
                    let encoded = EncodedCommand::new(
                        vec![AsciiFrame::new(frame_text)?],
                        ResponseMatcher::None,
                        Vec::new(),
                        CommandPriority::High,
                    );
                    tracing::debug!(driver = %self.profile.id(), step = step.label(), "startup auto-info step");
                    if let Err(error) = self
                        .send_encoded(
                            transport,
                            encoded,
                            UpdateSource::Native,
                            STARTUP_RESPONSE_TIMEOUT,
                            ctx,
                        )
                        .await
                    {
                        if error.is_transport() {
                            return Err(error);
                        }
                        tracing::warn!(driver = %self.profile.id(), step = step.label(), ?error, "startup auto-info step failed; continuing");
                    }
                }
                StartupStep::Query(semantic) => {
                    let Some(encoded) =
                        encode_kenwood_query(self.profile, semantic, self.vfo_routing)?
                    else {
                        tracing::trace!(driver = %self.profile.id(), semantic, "startup semantic skipped (no query frame)");
                        continue;
                    };
                    tracing::debug!(
                        driver = %self.profile.id(),
                        semantic,
                        frame_count = encoded.frames.len(),
                        expected = ?encoded.matcher,
                        "startup query step"
                    );
                    match self
                        .send_encoded(
                            transport,
                            encoded,
                            UpdateSource::Native,
                            STARTUP_RESPONSE_TIMEOUT,
                            ctx,
                        )
                        .await
                    {
                        Ok(_) => responsive = true,
                        Err(error) if error.is_transport() => return Err(error),
                        Err(error) => {
                            tracing::warn!(driver = %self.profile.id(), semantic, ?error, "startup query failed; continuing")
                        }
                    }
                }
            }
        }

        if !responsive {
            return Err(RadioError::Timeout {
                command: "startup-response",
            });
        }
        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Ready)],
            UpdateSource::Native,
        );
        Ok(())
    }

    async fn refresh(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let Some(transport) = transport else {
            return Err(RadioError::InvalidValue {
                field: "transport",
                message: "kenwood-ascii requires a transport".to_string(),
            });
        };

        tracing::info!(
            driver = %self.profile.id(),
            refresh_steps = self.profile.startup.len(),
            "running kenwood-ascii refresh sequence"
        );
        for step in self.profile.startup {
            match *step {
                StartupStep::AutoInfo(frame_text) => {
                    self.send_encoded(
                        transport,
                        EncodedCommand::new(
                            vec![AsciiFrame::new(frame_text)?],
                            ResponseMatcher::None,
                            Vec::new(),
                            CommandPriority::High,
                        ),
                        UpdateSource::ManualRefresh,
                        STARTUP_RESPONSE_TIMEOUT,
                        ctx,
                    )
                    .await?;
                }
                StartupStep::Query(semantic) => {
                    let Some(encoded) =
                        encode_kenwood_query(self.profile, semantic, self.vfo_routing)?
                    else {
                        continue;
                    };
                    self.send_encoded(
                        transport,
                        encoded,
                        UpdateSource::ManualRefresh,
                        STARTUP_RESPONSE_TIMEOUT,
                        ctx,
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    async fn execute(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        let Some(encoded) = encode_kenwood_command(
            self.profile,
            self.options,
            &command,
            state_before,
            self.vfo_routing,
        )?
        else {
            return Err(RadioError::UnsupportedCapability {
                capability: "command",
            });
        };

        let Some(transport) = transport else {
            ctx.publish_patches(encoded.completion_patches, UpdateSource::CommandResponse);
            apply_kenwood_routing_command(&mut self.vfo_routing, &command);
            return Ok(CommandCompletion::Accepted);
        };

        if command_matches_state(&command, state_before) {
            for semantic in kenwood_validation_queries(self.profile, &command, state_before) {
                let Some(encoded) =
                    (match encode_kenwood_query(self.profile, semantic, self.vfo_routing) {
                        Ok(encoded) => encoded,
                        Err(error) => {
                            tracing::warn!(
                                driver = %self.profile.id(),
                                ?command,
                                semantic,
                                ?error,
                                "failed to encode Kenwood validation query"
                            );
                            continue;
                        }
                    })
                else {
                    continue;
                };

                self.send_encoded(
                    transport,
                    encoded,
                    UpdateSource::CommandResponse,
                    COMMAND_RESPONSE_TIMEOUT,
                    ctx,
                )
                .await?;
            }

            if command_matches_state(&command, ctx.state()) {
                tracing::debug!(
                    driver = %self.profile.id(),
                    ?command,
                    "validated current state; skipping Kenwood setter"
                );
                return Ok(CommandCompletion::Observed);
            }
        }

        tracing::debug!(
            driver = %self.profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            "dispatching command over transport"
        );

        let completion_patches = encoded.completion_patches.clone();

        let completion = self
            .send_encoded(
                transport,
                encoded,
                UpdateSource::CommandResponse,
                COMMAND_RESPONSE_TIMEOUT,
                ctx,
            )
            .await?;
        if completion == CommandCompletion::Written {
            ctx.publish_patches(completion_patches, UpdateSource::CommandResponse);
            apply_kenwood_routing_command(&mut self.vfo_routing, &command);
        }
        Ok(completion)
    }

    async fn process_incoming(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        let Some(transport) = transport else {
            return Ok(false);
        };
        self.process_incoming_with_expected(
            transport,
            wait_timeout,
            default_source,
            None,
            false,
            ctx,
        )
        .await
    }

    async fn poll_one(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        let Some(transport) = transport else {
            return Ok(true);
        };
        if let Some(plan) = self.profile.poll {
            let semantic = plan.queries[self.poll_index];
            self.poll_index = (self.poll_index + 1) % plan.queries.len();
            let complete = self.poll_index == 0;
            if let Some(encoded) = encode_kenwood_query(self.profile, semantic, self.vfo_routing)?
                && let Err(error) = self
                    .send_encoded(
                        transport,
                        encoded,
                        UpdateSource::Poll,
                        COMMAND_RESPONSE_TIMEOUT,
                        ctx,
                    )
                    .await
            {
                tracing::debug!(driver = %self.profile.id(), semantic, ?error, "poll query failed");
            }
            return Ok(complete);
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IcomWaitOutcome {
    Matched,
    Timeout,
    Rejected,
    Collision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SmartSdrWaitOutcome {
    Matched,
    Timeout,
    Rejected { code: u32, message: String },
}

struct IcomCivRuntime {
    profile: &'static IcomCivProfile,
    options: IcomCivOptions,
    frame_splitter: IcomFrameSplitter,
    poll_index: usize,
}

struct IcomWaitRequest<'a> {
    default_source: UpdateSource,
    expected: Option<&'a IcomResponseMatcher>,
    echo: Option<&'a [u8]>,
    receiver_hint: Option<ReceiverPath>,
    ctx: &'a mut dyn StateSink,
}

impl IcomCivRuntime {
    fn new(profile: &'static IcomCivProfile, options: IcomCivOptions) -> Self {
        Self {
            profile,
            options,
            frame_splitter: IcomFrameSplitter::new(),
            poll_index: 0,
        }
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: icom_civ::EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        let icom_civ::EncodedCommand {
            frames,
            matcher,
            response_receiver,
            ..
        } = encoded;

        let completion = match &matcher {
            IcomResponseMatcher::None => CommandCompletion::Written,
            IcomResponseMatcher::Ack => CommandCompletion::Accepted,
            IcomResponseMatcher::PayloadPrefix(_) | IcomResponseMatcher::OneOf(_) => {
                CommandCompletion::Observed
            }
        };

        for frame in frames {
            tracing::debug!(
                driver = %self.profile.id(),
                tx_bytes = ?frame.as_bytes(),
                "sending ICOM CI-V frame"
            );

            timeout(TRANSPORT_IO_TIMEOUT, transport.write_all(frame.as_bytes()))
                .await
                .map_err(|_| RadioError::Timeout {
                    command: "transport-write",
                })??;
            timeout(TRANSPORT_IO_TIMEOUT, transport.flush())
                .await
                .map_err(|_| RadioError::Timeout {
                    command: "transport-flush",
                })??;

            if matcher.expects_response() {
                match self
                    .process_incoming_with_expected(
                        transport,
                        wait_timeout,
                        IcomWaitRequest {
                            default_source,
                            expected: Some(&matcher),
                            echo: Some(frame.as_bytes()),
                            receiver_hint: response_receiver,
                            ctx,
                        },
                    )
                    .await?
                {
                    IcomWaitOutcome::Matched => {}
                    IcomWaitOutcome::Timeout => {
                        return Err(RadioError::Timeout {
                            command: "icom-command-response",
                        });
                    }
                    IcomWaitOutcome::Rejected => {
                        return Err(RadioError::CommandRejected {
                            protocol: "icom-civ",
                            reason: "negative acknowledgement",
                        });
                    }
                    IcomWaitOutcome::Collision => {
                        return Err(RadioError::ProtocolCommunication);
                    }
                }
            }
        }

        Ok(completion)
    }

    async fn process_incoming_with_expected(
        &mut self,
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        request: IcomWaitRequest<'_>,
    ) -> Result<IcomWaitOutcome> {
        let IcomWaitRequest {
            default_source,
            expected,
            echo,
            receiver_hint,
            ctx,
        } = request;
        let deadline = Instant::now() + wait_timeout;
        let mut saw_frames = false;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(if expected.is_none() && saw_frames {
                    IcomWaitOutcome::Matched
                } else {
                    IcomWaitOutcome::Timeout
                });
            }

            let remaining = deadline.saturating_duration_since(now);
            let mut buf = [0u8; 1024];
            let count = match timeout(remaining, transport.read_some(&mut buf)).await {
                Ok(read_result) => read_result?,
                Err(_) => {
                    return Ok(if expected.is_none() && saw_frames {
                        IcomWaitOutcome::Matched
                    } else {
                        IcomWaitOutcome::Timeout
                    });
                }
            };

            if count == 0 {
                return Err(RadioError::Transport(
                    "connection closed by peer".to_string(),
                ));
            }

            let frames = match self.frame_splitter.push(&buf[..count]) {
                Ok(frames) => frames,
                Err(error) => {
                    tracing::warn!(
                        driver = %self.profile.id(),
                        ?error,
                        "failed to split incoming ICOM CI-V bytes into frames; dropping chunk"
                    );
                    continue;
                }
            };

            for frame in frames {
                if echo.is_some_and(|sent| frame.is_echo_of(sent)) {
                    tracing::trace!(driver = %self.profile.id(), rx_bytes = ?frame.as_bytes(), "discarding ICOM self-echo frame");
                    continue;
                }

                if frame.to() != self.options.controller_address
                    || frame.from() != self.options.radio_address
                    || frame.to() == icom_civ::BROADCAST_ADDRESS
                    || frame.from() == icom_civ::BROADCAST_ADDRESS
                {
                    tracing::trace!(
                        driver = %self.profile.id(),
                        rx_bytes = ?frame.as_bytes(),
                        expected_to = self.options.controller_address,
                        expected_from = self.options.radio_address,
                        "discarding ICOM frame with unexpected address"
                    );
                    continue;
                }
                saw_frames = true;

                if let Some(status) = icom_civ::ProtocolStatus::parse(&frame) {
                    tracing::debug!(
                        driver = %self.profile.id(),
                        rx_bytes = ?frame.as_bytes(),
                        ?status,
                        "received ICOM protocol status"
                    );
                    match status {
                        icom_civ::ProtocolStatus::Ok => {
                            if expected
                                .is_some_and(|matcher| matches!(matcher, IcomResponseMatcher::Ack))
                            {
                                return Ok(IcomWaitOutcome::Matched);
                            }
                        }
                        icom_civ::ProtocolStatus::Ng => {
                            if expected.is_some() {
                                return Ok(IcomWaitOutcome::Rejected);
                            }
                        }
                        icom_civ::ProtocolStatus::Collision => {
                            if expected.is_some() {
                                return Ok(IcomWaitOutcome::Collision);
                            }
                        }
                    }
                    continue;
                }

                let mut matched_expected = false;
                if let Some(expected) = expected
                    && expected.matches_from(
                        &frame,
                        self.options.controller_address,
                        self.options.radio_address,
                    )
                {
                    tracing::debug!(
                        driver = %self.profile.id(),
                        rx_bytes = ?frame.as_bytes(),
                        expected = ?expected,
                        "received expected ICOM CI-V response"
                    );
                    matched_expected = true;
                }

                let source = if matched_expected {
                    default_source
                } else {
                    UpdateSource::Native
                };
                self.decode_and_publish_frame(&frame, source, receiver_hint, ctx);

                if matched_expected {
                    return Ok(IcomWaitOutcome::Matched);
                }
            }

            if expected.is_none() {
                return Ok(if saw_frames {
                    IcomWaitOutcome::Matched
                } else {
                    IcomWaitOutcome::Timeout
                });
            }
        }
    }

    fn decode_and_publish_frame(
        &self,
        frame: &CivFrame,
        default_source: UpdateSource,
        receiver_hint: Option<crate::ReceiverPath>,
        ctx: &mut dyn StateSink,
    ) {
        match icom_civ::decode(self.profile, frame, ctx.state(), receiver_hint) {
            Ok(Some(decoded)) => {
                let source = default_source;
                tracing::debug!(
                    driver = %self.profile.id(),
                    rx_bytes = ?frame.as_bytes(),
                    source = ?source,
                    patch_count = decoded.patches.len(),
                    "decoded ICOM CI-V frame into state patches"
                );
                ctx.publish_patches(decoded.patches, source);
            }
            Ok(None) => {
                tracing::trace!(
                    driver = %self.profile.id(),
                    rx_bytes = ?frame.as_bytes(),
                    "unhandled ICOM CI-V frame"
                );
            }
            Err(error) => {
                tracing::warn!(
                    driver = %self.profile.id(),
                    rx_bytes = ?frame.as_bytes(),
                    ?error,
                    "failed to decode ICOM CI-V frame; continuing"
                );
            }
        }
    }
}

#[async_trait]
impl RadioSession for IcomCivRuntime {
    fn descriptor(&self) -> DriverDescriptor {
        self.profile.descriptor
    }

    fn capabilities(&self) -> RadioCapabilities {
        self.profile.capabilities
    }

    fn initial_state(&self) -> RadioState {
        RadioState {
            connection: ConnectionState::Connecting,
            main_rx: ReceiverState::default(),
            sub_rx: self
                .profile
                .capabilities
                .sub_rx
                .map(|_| ReceiverState::default()),
            tx: self
                .profile
                .capabilities
                .tx
                .map(|_| TransmitterState::default()),
            rit_xit: RitXitState::default(),
            keyer: self
                .profile
                .capabilities
                .keyer
                .map(|_| KeyerState::default()),
        }
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.profile.poll.map(|_| self.options.poll_interval)
    }

    async fn startup(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Identifying)],
            UpdateSource::Native,
        );
        let Some(transport) = transport else {
            return Ok(());
        };
        tracing::info!(
            driver = %self.profile.id(),
            startup_steps = self.profile.startup.len(),
            "running icom-civ startup sequence"
        );
        let mut responsive = false;

        for step in self.profile.startup {
            match *step {
                icom_civ::StartupStep::Query(semantic) => {
                    let Some(encoded) =
                        icom_civ::encode_query(self.profile, self.options, semantic)?
                    else {
                        tracing::trace!(driver = %self.profile.id(), semantic, "ICOM startup semantic skipped");
                        continue;
                    };
                    tracing::debug!(
                        driver = %self.profile.id(),
                        semantic,
                        frame_count = encoded.frames.len(),
                        expected = ?encoded.matcher,
                        "ICOM startup query step"
                    );
                    let state_before = ctx.state().clone();
                    match self
                        .send_encoded(
                            transport,
                            encoded,
                            UpdateSource::Native,
                            STARTUP_RESPONSE_TIMEOUT,
                            ctx,
                        )
                        .await
                    {
                        Ok(_) => responsive = true,
                        Err(error) if error.is_transport() => return Err(error),
                        Err(error) => {
                            tracing::warn!(driver = %self.profile.id(), semantic, ?error, "ICOM startup query failed; continuing")
                        }
                    }
                    responsive |= ctx.state() != &state_before;
                }
            }
        }

        if !responsive {
            return Err(RadioError::Timeout {
                command: "startup-response",
            });
        }
        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Ready)],
            UpdateSource::Native,
        );
        Ok(())
    }

    async fn refresh(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let Some(transport) = transport else {
            return Err(RadioError::InvalidValue {
                field: "transport",
                message: "icom-civ requires a transport".to_string(),
            });
        };

        tracing::info!(
            driver = %self.profile.id(),
            refresh_steps = self.profile.startup.len(),
            "running icom-civ refresh sequence"
        );
        for step in self.profile.startup {
            let icom_civ::StartupStep::Query(semantic) = *step;
            let Some(encoded) = icom_civ::encode_query(self.profile, self.options, semantic)?
            else {
                continue;
            };
            self.send_encoded(
                transport,
                encoded,
                UpdateSource::ManualRefresh,
                STARTUP_RESPONSE_TIMEOUT,
                ctx,
            )
            .await?;
        }

        Ok(())
    }

    async fn execute(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        let Some(encoded) = icom_civ::encode(self.profile, self.options, &command, state_before)?
        else {
            return Err(RadioError::UnsupportedCapability {
                capability: "command",
            });
        };

        let Some(transport) = transport else {
            ctx.publish_patches(encoded.completion_patches, UpdateSource::CommandResponse);
            return Ok(CommandCompletion::Accepted);
        };

        if command_matches_state(&command, state_before) {
            for semantic in icom_validation_queries(self.profile, &command, state_before) {
                let Some(encoded) =
                    (match icom_civ::encode_query(self.profile, self.options, semantic) {
                        Ok(encoded) => encoded,
                        Err(error) => {
                            tracing::warn!(
                                driver = %self.profile.id(),
                                ?command,
                                semantic,
                                ?error,
                                "failed to encode ICOM validation query"
                            );
                            continue;
                        }
                    })
                else {
                    continue;
                };

                if let Err(error) = self
                    .send_encoded(
                        transport,
                        encoded,
                        UpdateSource::CommandResponse,
                        COMMAND_RESPONSE_TIMEOUT,
                        ctx,
                    )
                    .await
                {
                    tracing::warn!(
                        driver = %self.profile.id(),
                        ?command,
                        semantic,
                        ?error,
                        "ICOM validation query failed; continuing with setter"
                    );
                }
            }

            if command_matches_state(&command, ctx.state()) {
                tracing::debug!(
                    driver = %self.profile.id(),
                    ?command,
                    "validated current state; skipping ICOM setter"
                );
                return Ok(CommandCompletion::Observed);
            }
        }

        tracing::debug!(
            driver = %self.profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            "dispatching ICOM command over transport"
        );

        let completion_patches = encoded.completion_patches.clone();

        let completion = self
            .send_encoded(
                transport,
                encoded,
                UpdateSource::CommandResponse,
                COMMAND_RESPONSE_TIMEOUT,
                ctx,
            )
            .await?;
        if matches!(
            completion,
            CommandCompletion::Written | CommandCompletion::Accepted
        ) {
            ctx.publish_patches(completion_patches, UpdateSource::CommandResponse);
        }
        Ok(completion)
    }

    async fn process_incoming(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        let Some(transport) = transport else {
            return Ok(false);
        };
        Ok(matches!(
            self.process_incoming_with_expected(
                transport,
                wait_timeout,
                IcomWaitRequest {
                    default_source,
                    expected: None,
                    echo: None,
                    receiver_hint: None,
                    ctx,
                },
            )
            .await?,
            IcomWaitOutcome::Matched
        ))
    }

    async fn poll_one(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        let Some(transport) = transport else {
            return Ok(true);
        };
        if let Some(plan) = self.profile.poll {
            let semantic = plan.queries[self.poll_index];
            self.poll_index = (self.poll_index + 1) % plan.queries.len();
            let complete = self.poll_index == 0;
            if let Some(encoded) = icom_civ::encode_query(self.profile, self.options, semantic)?
                && let Err(error) = self
                    .send_encoded(
                        transport,
                        encoded,
                        UpdateSource::Poll,
                        COMMAND_RESPONSE_TIMEOUT,
                        ctx,
                    )
                    .await
            {
                tracing::debug!(driver = %self.profile.id(), semantic, ?error, "ICOM poll query failed");
            }
            return Ok(complete);
        }
        Ok(true)
    }
}

struct SmartSdrRuntime {
    profile: SmartSdrProfile,
    line_splitter: SmartSdrLineSplitter,
    next_sequence: u32,
    startup_phase: SmartSdrStartupPhase,
    version: Option<String>,
    handle: Option<String>,
    saw_slice_status: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartSdrStartupPhase {
    Greeting,
    Subscriptions,
    Ready,
}

impl SmartSdrRuntime {
    fn new(profile: SmartSdrProfile) -> Self {
        Self {
            profile,
            line_splitter: SmartSdrLineSplitter::new(),
            next_sequence: 1,
            startup_phase: SmartSdrStartupPhase::Greeting,
            version: None,
            handle: None,
            saw_slice_status: false,
        }
    }

    fn greeting_complete(&self) -> bool {
        self.version
            .as_ref()
            .is_some_and(|version| !version.is_empty())
            && self
                .handle
                .as_ref()
                .is_some_and(|handle| !handle.is_empty())
    }

    fn selected_slice_status(&self, message: &str) -> bool {
        let Some(rest) = message.strip_prefix("slice ") else {
            return false;
        };
        let Some((slice, _)) = rest.split_once(' ') else {
            return false;
        };
        slice.parse::<u8>().ok() == Some(self.profile.slice)
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: smartsdr::EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        let smartsdr::EncodedCommand {
            commands,
            completion_patches,
        } = encoded;

        for command in commands {
            self.send_command_body(transport, &command, default_source, wait_timeout, ctx)
                .await?;
        }

        if !completion_patches.is_empty() {
            ctx.publish_patches(completion_patches, default_source);
        }

        Ok(CommandCompletion::Accepted)
    }

    async fn send_command_body(
        &mut self,
        transport: &mut dyn CatTransport,
        command: &str,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);

        let frame = smartsdr::command_frame(sequence, command)?;
        tracing::debug!(
            driver = %self.profile.id(),
            sequence,
            tx_frame = frame.trim_end(),
            "sending SmartSDR command"
        );
        timeout(TRANSPORT_IO_TIMEOUT, transport.write_all(frame.as_bytes()))
            .await
            .map_err(|_| RadioError::Timeout {
                command: "transport-write",
            })??;
        timeout(TRANSPORT_IO_TIMEOUT, transport.flush())
            .await
            .map_err(|_| RadioError::Timeout {
                command: "transport-flush",
            })??;

        match self
            .process_incoming_with_expected(
                transport,
                wait_timeout,
                default_source,
                Some(sequence),
                Some(command),
                ctx,
            )
            .await?
        {
            SmartSdrWaitOutcome::Matched => Ok(()),
            SmartSdrWaitOutcome::Timeout => Err(RadioError::Timeout {
                command: "smartsdr-command-response",
            }),
            SmartSdrWaitOutcome::Rejected { code, message } => Err(RadioError::Decode {
                command: "smartsdr-response",
                message: format!("radio rejected command with 0x{code:08X}: {message}"),
            }),
        }
    }

    async fn process_incoming_with_expected(
        &mut self,
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        expected_sequence: Option<u32>,
        expected_command: Option<&str>,
        ctx: &mut dyn StateSink,
    ) -> Result<SmartSdrWaitOutcome> {
        let deadline = Instant::now() + wait_timeout;
        let mut saw_lines = false;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(if expected_sequence.is_none() && saw_lines {
                    SmartSdrWaitOutcome::Matched
                } else {
                    SmartSdrWaitOutcome::Timeout
                });
            }

            let remaining = deadline.saturating_duration_since(now);
            let mut buf = [0u8; 1024];
            let count = match timeout(remaining, transport.read_some(&mut buf)).await {
                Ok(read_result) => read_result?,
                Err(_) => {
                    return Ok(if expected_sequence.is_none() && saw_lines {
                        SmartSdrWaitOutcome::Matched
                    } else {
                        SmartSdrWaitOutcome::Timeout
                    });
                }
            };

            if count == 0 {
                return Err(RadioError::Transport(
                    "connection closed by peer".to_string(),
                ));
            }

            let lines = self.line_splitter.push(&buf[..count])?;
            let mut response_outcome = None;
            for line in lines {
                saw_lines = true;
                match smartsdr::parse_line(&line)? {
                    smartsdr::IncomingLine::Version(version) => {
                        tracing::debug!(
                            driver = %self.profile.id(),
                            version = %version,
                            "received SmartSDR protocol version"
                        );
                        self.version = Some(version);
                    }
                    smartsdr::IncomingLine::Handle(handle) => {
                        tracing::debug!(
                            driver = %self.profile.id(),
                            handle = %handle,
                            "received SmartSDR client handle"
                        );
                        self.handle = Some(handle);
                    }
                    smartsdr::IncomingLine::Status(message) => {
                        if self.selected_slice_status(&message) {
                            self.saw_slice_status = true;
                        }
                        match smartsdr::decode_status(&self.profile, &message, ctx.state()) {
                            Ok(Some(decoded)) => {
                                let source = if default_source == UpdateSource::ManualRefresh {
                                    UpdateSource::ManualRefresh
                                } else {
                                    UpdateSource::Native
                                };
                                ctx.publish_patches(decoded.patches, source);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(
                                    driver = %self.profile.id(),
                                    line = %line,
                                    ?error,
                                    "failed to decode SmartSDR status line"
                                );
                            }
                        }
                    }
                    smartsdr::IncomingLine::Response(response) => {
                        tracing::debug!(
                            driver = %self.profile.id(),
                            sequence = response.sequence,
                            code = format_args!("{:08X}", response.code),
                            message = %response.message,
                            "received SmartSDR command response"
                        );
                        if expected_sequence == Some(response.sequence) {
                            if response.code == 0 {
                                if let Some(command) = expected_command {
                                    match smartsdr::decode_response(
                                        &self.profile,
                                        command,
                                        &response.message,
                                        ctx.state(),
                                    ) {
                                        Ok(Some(decoded)) => {
                                            ctx.publish_patches(decoded.patches, default_source);
                                        }
                                        Ok(None) => {}
                                        Err(error) => {
                                            tracing::warn!(
                                                driver = %self.profile.id(),
                                                command,
                                                message = %response.message,
                                                ?error,
                                                "failed to decode SmartSDR response payload"
                                            );
                                        }
                                    }
                                }

                                response_outcome = Some(SmartSdrWaitOutcome::Matched);
                            } else {
                                response_outcome = Some(SmartSdrWaitOutcome::Rejected {
                                    code: response.code,
                                    message: response.message,
                                });
                            }
                        }
                    }
                    smartsdr::IncomingLine::Message(message) => {
                        tracing::debug!(
                            driver = %self.profile.id(),
                            message = %message,
                            "received SmartSDR message line"
                        );
                    }
                    smartsdr::IncomingLine::Unknown(text) => {
                        tracing::trace!(
                            driver = %self.profile.id(),
                            line = %text,
                            "ignoring unknown SmartSDR line"
                        );
                    }
                }
            }

            if let Some(outcome) = response_outcome {
                return Ok(outcome);
            }

            if expected_sequence.is_none()
                && saw_lines
                && (self.startup_phase != SmartSdrStartupPhase::Greeting
                    || self.greeting_complete())
            {
                return Ok(SmartSdrWaitOutcome::Matched);
            }
        }
    }
}

#[async_trait]
impl RadioSession for SmartSdrRuntime {
    fn descriptor(&self) -> DriverDescriptor {
        self.profile.descriptor
    }

    fn capabilities(&self) -> RadioCapabilities {
        self.profile.capabilities
    }

    fn initial_state(&self) -> RadioState {
        RadioState {
            connection: ConnectionState::Connecting,
            main_rx: ReceiverState::default(),
            sub_rx: None,
            tx: Some(TransmitterState::default()),
            rit_xit: RitXitState::default(),
            keyer: Some(KeyerState::default()),
        }
    }

    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    async fn startup(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Identifying)],
            UpdateSource::Native,
        );
        let Some(transport) = transport else {
            return Err(RadioError::InvalidValue {
                field: "transport",
                message: "flexradio-smartsdr requires a transport".to_string(),
            });
        };
        let _ = self
            .process_incoming_with_expected(
                transport,
                STARTUP_RESPONSE_TIMEOUT,
                UpdateSource::Native,
                None,
                None,
                ctx,
            )
            .await?;

        if !self.greeting_complete() {
            return Err(RadioError::Timeout {
                command: "smartsdr-greeting",
            });
        }

        self.saw_slice_status = false;
        self.startup_phase = SmartSdrStartupPhase::Subscriptions;

        self.send_command_body(
            transport,
            &format!("sub slice {}", self.profile.slice),
            UpdateSource::Native,
            SMARTSDR_STARTUP_TIMEOUT,
            ctx,
        )
        .await?;

        self.send_command_body(
            transport,
            "sub cwx all",
            UpdateSource::Native,
            SMARTSDR_STARTUP_TIMEOUT,
            ctx,
        )
        .await?;

        self.send_command_body(
            transport,
            "sub tx all",
            UpdateSource::Native,
            SMARTSDR_STARTUP_TIMEOUT,
            ctx,
        )
        .await?;

        let _ = self
            .process_incoming_with_expected(
                transport,
                STARTUP_RESPONSE_TIMEOUT,
                UpdateSource::Native,
                None,
                None,
                ctx,
            )
            .await?;

        if !self.saw_slice_status {
            return Err(RadioError::Timeout {
                command: "smartsdr-slice-status",
            });
        }

        self.startup_phase = SmartSdrStartupPhase::Ready;

        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Ready)],
            UpdateSource::Native,
        );

        Ok(())
    }

    async fn refresh(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let Some(transport) = transport else {
            return Err(RadioError::InvalidValue {
                field: "transport",
                message: "flexradio-smartsdr requires a transport".to_string(),
            });
        };

        self.send_command_body(
            transport,
            &format!("sub slice {}", self.profile.slice),
            UpdateSource::ManualRefresh,
            SMARTSDR_STARTUP_TIMEOUT,
            ctx,
        )
        .await?;
        self.send_command_body(
            transport,
            "sub cwx all",
            UpdateSource::ManualRefresh,
            SMARTSDR_STARTUP_TIMEOUT,
            ctx,
        )
        .await?;
        self.send_command_body(
            transport,
            "sub tx all",
            UpdateSource::ManualRefresh,
            SMARTSDR_STARTUP_TIMEOUT,
            ctx,
        )
        .await?;

        Ok(())
    }

    async fn execute(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn StateSink,
    ) -> Result<CommandCompletion> {
        let Some(encoded) = smartsdr::encode(&self.profile, &command, state_before)? else {
            return Err(RadioError::UnsupportedCapability {
                capability: "command",
            });
        };

        let Some(transport) = transport else {
            return Err(RadioError::InvalidValue {
                field: "transport",
                message: "flexradio-smartsdr requires a transport".to_string(),
            });
        };

        if command_matches_state(&command, state_before) {
            tracing::debug!(
                driver = %self.profile.id(),
                ?command,
                "validated current state; skipping SmartSDR setter"
            );
            return Ok(CommandCompletion::Observed);
        }

        self.send_encoded(
            transport,
            encoded,
            UpdateSource::CommandResponse,
            COMMAND_RESPONSE_TIMEOUT,
            ctx,
        )
        .await
    }

    async fn process_incoming(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        let Some(transport) = transport else {
            return Ok(false);
        };
        Ok(matches!(
            self.process_incoming_with_expected(
                transport,
                wait_timeout,
                default_source,
                None,
                None,
                ctx,
            )
            .await?,
            SmartSdrWaitOutcome::Matched
        ))
    }

    async fn poll_one(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        _ctx: &mut dyn StateSink,
    ) -> Result<bool> {
        Ok(true)
    }
}

fn encode_kenwood_command(
    profile: &'static KenwoodAsciiProfile,
    options: KenwoodAsciiOptions,
    command: &RadioCommand,
    current_state: &RadioState,
    vfo_routing: kenwood_ascii::VfoRouting,
) -> Result<Option<EncodedCommand>> {
    if let Some(encoded) =
        kenwood_ascii::frequency::encode_with_routing(profile, command, current_state, vfo_routing)?
    {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::mode::encode_with_routing_and_options(
        profile,
        options,
        command,
        current_state,
        vfo_routing,
    )? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) =
        kenwood_ascii::split::encode_with_routing(profile, command, current_state, vfo_routing)?
    {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::rit_xit::encode(profile, command, current_state)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) =
        kenwood_ascii::filter::encode_with_routing(profile, command, current_state, vfo_routing)?
    {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::rf::encode_with_routing(profile, command, vfo_routing)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::tx::encode(profile, options, command)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::keyer::encode(profile, command)? {
        return Ok(Some(encoded));
    }

    Ok(None)
}

fn apply_kenwood_routing_command(routing: &mut kenwood_ascii::VfoRouting, command: &RadioCommand) {
    if let RadioCommand::SetSplit(split) = command {
        routing.set_split(*split);
    }
}

fn encode_kenwood_query(
    profile: &'static KenwoodAsciiProfile,
    semantic: &'static str,
    vfo_routing: kenwood_ascii::VfoRouting,
) -> Result<Option<EncodedCommand>> {
    if let Some(encoded) =
        kenwood_ascii::frequency::encode_query_with_routing(profile, semantic, vfo_routing)?
    {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::info::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::mode::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::split::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::rit_xit::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::filter::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::rf::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::tx::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::keyer::encode_query(profile, semantic)? {
        return Ok(Some(encoded));
    }

    Ok(None)
}

fn decode_kenwood_frame(
    profile: &'static KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
    vfo_routing: &mut kenwood_ascii::VfoRouting,
) -> Result<Option<kenwood_ascii::DecodedFrame>> {
    if let Some(decoded) = kenwood_ascii::info::decode(profile, frame, state, vfo_routing)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) =
        kenwood_ascii::frequency::decode_with_routing(profile, frame, state, *vfo_routing)?
    {
        return Ok(Some(decoded));
    }
    if let Some(decoded) =
        kenwood_ascii::mode::decode_with_routing(profile, frame, state, *vfo_routing)?
    {
        return Ok(Some(decoded));
    }
    if let Some(decoded) =
        kenwood_ascii::split::decode_with_routing(profile, frame, state, vfo_routing)?
    {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::rit_xit::decode(profile, frame)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) =
        kenwood_ascii::filter::decode_with_routing(profile, frame, state, vfo_routing)?
    {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::rf::decode_with_routing(profile, frame, vfo_routing)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::tx::decode(profile, frame)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::keyer::decode(profile, frame)? {
        return Ok(Some(decoded));
    }

    Ok(None)
}

fn kenwood_validation_queries(
    profile: &'static KenwoodAsciiProfile,
    command: &RadioCommand,
    _state_before: &RadioState,
) -> Vec<&'static str> {
    match command {
        RadioCommand::SetReceiverFrequency { receiver, .. } => {
            vec![frequency_query_for_receiver(*receiver)]
        }
        RadioCommand::SetTxFrequency(_) => vec!["FA", "FB"],
        RadioCommand::SetReceiverMode { receiver, .. } => {
            mode_queries_for_receiver(profile, *receiver)
        }
        RadioCommand::SetTxMode(_) => all_mode_queries(profile),
        RadioCommand::SetReceiverFilterBandwidth { receiver, .. }
        | RadioCommand::SetReceiverFilterShift { receiver, .. } => {
            filter_queries_for_receiver(profile, *receiver)
        }
        RadioCommand::SetReceiverPreamp { receiver, .. } => {
            rf_query_for_receiver(profile, *receiver, RfRecoveryFeature::Preamp)
        }
        RadioCommand::SetReceiverAttenuator { receiver, .. } => {
            rf_query_for_receiver(profile, *receiver, RfRecoveryFeature::Attenuator)
        }
        RadioCommand::SetReceiverNoiseBlanker { receiver, .. } => {
            rf_query_for_receiver(profile, *receiver, RfRecoveryFeature::NoiseBlanker)
        }
        RadioCommand::SetReceiverNoiseReduction { receiver, .. } => {
            rf_query_for_receiver(profile, *receiver, RfRecoveryFeature::NoiseReduction)
        }
        RadioCommand::SetReceiverAutoNotch { receiver, .. } => {
            rf_query_for_receiver(profile, *receiver, RfRecoveryFeature::AutoNotch)
        }
        RadioCommand::SetTxPower(_) => vec!["PC"],
        RadioCommand::SetPtt(_) | RadioCommand::SetDataPtt(_) => vec!["IF"],
        RadioCommand::SetSplit(_) => split_queries(profile),
        RadioCommand::SetRitEnabled { receiver, .. } => vec![rit_enabled_query(profile, *receiver)],
        RadioCommand::SetXitEnabled { receiver, .. } => vec![match receiver {
            ReceiverPath::Main => "XT",
            ReceiverPath::Sub => "XT$",
        }],
        RadioCommand::SetRitOffset { receiver, .. } => rit_offset_queries(profile, *receiver),
        RadioCommand::SetXitOffset { receiver, .. }
        | RadioCommand::SetRitXitOffset { receiver, .. } => rit_offset_queries(profile, *receiver),
        RadioCommand::SetKeyerSpeed(_) => vec!["KS"],
        RadioCommand::SendCw(_) | RadioCommand::StopCw => vec!["KY"],
        RadioCommand::Refresh => Vec::new(),
    }
}

fn frequency_query_for_receiver(receiver: ReceiverPath) -> &'static str {
    match receiver {
        ReceiverPath::Main => "FA",
        ReceiverPath::Sub => "FB",
    }
}

fn mode_queries_for_receiver(
    profile: &'static KenwoodAsciiProfile,
    receiver: ReceiverPath,
) -> Vec<&'static str> {
    match profile.id() {
        "kenwood-ts590" => vec!["MD", "DA"],
        "kenwood-ts890" => match receiver {
            ReceiverPath::Main => vec!["SF0"],
            ReceiverPath::Sub => vec!["SF1"],
        },
        "kenwood-ts990" => match receiver {
            ReceiverPath::Main => vec!["OM0"],
            ReceiverPath::Sub => vec!["OM1"],
        },
        "elecraft-k4" | "elecraft-k3" => match receiver {
            ReceiverPath::Main => vec!["MD", "DT"],
            ReceiverPath::Sub => vec!["MD$", "DT$"],
        },
        "kenwood-if232" | "elecraft-k2" => vec!["MD"],
        _ => vec!["MD"],
    }
}

fn all_mode_queries(profile: &'static KenwoodAsciiProfile) -> Vec<&'static str> {
    match profile.id() {
        "kenwood-ts590" => vec!["MD", "DA"],
        "kenwood-ts890" => vec!["SF0", "SF1"],
        "kenwood-ts990" => vec!["OM0", "OM1"],
        "elecraft-k4" | "elecraft-k3" => vec!["MD", "DT", "MD$", "DT$"],
        _ => vec!["MD"],
    }
}

fn filter_queries_for_receiver(
    profile: &'static KenwoodAsciiProfile,
    receiver: ReceiverPath,
) -> Vec<&'static str> {
    match profile.id() {
        "kenwood-ts890" | "kenwood-ts990" => match receiver {
            ReceiverPath::Main => vec!["filter-hi-lo-main"],
            ReceiverPath::Sub => vec!["filter-hi-lo-sub"],
        },
        "elecraft-k4" | "elecraft-k3" => match receiver {
            ReceiverPath::Main => vec!["BW", "IS"],
            ReceiverPath::Sub => vec!["BW$", "IS$"],
        },
        "kenwood-ts590" | "kenwood-ts2000" | "kenwood-ts480" => vec!["filter-state"],
        _ => vec!["IF"],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RfRecoveryFeature {
    Preamp,
    Attenuator,
    NoiseBlanker,
    NoiseReduction,
    AutoNotch,
}

fn rf_query_for_receiver(
    profile: &'static KenwoodAsciiProfile,
    receiver: ReceiverPath,
    feature: RfRecoveryFeature,
) -> Vec<&'static str> {
    match feature {
        RfRecoveryFeature::Preamp => match profile.id() {
            "kenwood-ts990" => match receiver {
                ReceiverPath::Main => vec!["PA0"],
                ReceiverPath::Sub => vec!["PA1"],
            },
            "elecraft-k4" | "elecraft-k3" => match receiver {
                ReceiverPath::Main => vec!["PA"],
                ReceiverPath::Sub => vec!["PA$"],
            },
            _ => vec!["PA"],
        },
        RfRecoveryFeature::Attenuator => match profile.id() {
            "kenwood-ts990" => match receiver {
                ReceiverPath::Main => vec!["RA0"],
                ReceiverPath::Sub => vec!["RA1"],
            },
            "elecraft-k4" | "elecraft-k3" => match receiver {
                ReceiverPath::Main => vec!["RA"],
                ReceiverPath::Sub => vec!["RA$"],
            },
            _ => vec!["RA"],
        },
        RfRecoveryFeature::NoiseBlanker => match profile.id() {
            "kenwood-ts890" => vec!["NB1", "NB2"],
            "kenwood-ts990" => match receiver {
                ReceiverPath::Main => vec!["NB10", "NB20"],
                ReceiverPath::Sub => vec!["NB11", "NB21"],
            },
            "elecraft-k4" | "elecraft-k3" => match receiver {
                ReceiverPath::Main => vec!["NB"],
                ReceiverPath::Sub => vec!["NB$"],
            },
            _ => vec!["NB"],
        },
        RfRecoveryFeature::NoiseReduction => match profile.id() {
            "kenwood-ts990" => match receiver {
                ReceiverPath::Main => vec!["NR0"],
                ReceiverPath::Sub => vec!["NR1"],
            },
            "elecraft-k4" | "elecraft-k3" => match receiver {
                ReceiverPath::Main => vec!["NR"],
                ReceiverPath::Sub => vec!["NR$"],
            },
            _ => vec!["NR"],
        },
        RfRecoveryFeature::AutoNotch => match profile.id() {
            "kenwood-ts990" => match receiver {
                ReceiverPath::Main => vec!["NT0"],
                ReceiverPath::Sub => vec!["NT1"],
            },
            "elecraft-k4" | "elecraft-k3" => match receiver {
                ReceiverPath::Main => vec!["NA"],
                ReceiverPath::Sub => vec!["NA$"],
            },
            _ => vec!["NT"],
        },
    }
}

fn split_queries(profile: &'static KenwoodAsciiProfile) -> Vec<&'static str> {
    match profile.id() {
        "kenwood-ts990" => vec!["SP"],
        "kenwood-if232" => vec!["ST"],
        "elecraft-k4" | "elecraft-k3" | "elecraft-k2" => vec!["FT"],
        _ => vec!["FR", "FT"],
    }
}

fn rit_enabled_query(
    profile: &'static KenwoodAsciiProfile,
    receiver: ReceiverPath,
) -> &'static str {
    match (profile.id(), receiver) {
        ("elecraft-k4", ReceiverPath::Sub) => "RT$",
        _ => "RT",
    }
}

fn rit_offset_queries(
    profile: &'static KenwoodAsciiProfile,
    receiver: ReceiverPath,
) -> Vec<&'static str> {
    match profile.id() {
        "kenwood-ts890" | "kenwood-ts990" => vec!["RF"],
        "elecraft-k4" | "elecraft-k3" => match receiver {
            ReceiverPath::Main => vec!["RO"],
            ReceiverPath::Sub => vec!["RO$"],
        },
        _ => vec!["IF"],
    }
}

fn command_matches_state(command: &RadioCommand, state: &RadioState) -> bool {
    match command {
        RadioCommand::SetReceiverFrequency {
            receiver,
            frequency,
        } => receiver_state(state, *receiver)
            .and_then(|rx| rx.frequency)
            .is_some_and(|current| current == *frequency),
        RadioCommand::SetReceiverMode { receiver, mode } => receiver_state(state, *receiver)
            .and_then(|rx| rx.mode)
            .is_some_and(|current| current == *mode),
        RadioCommand::SetReceiverFilterBandwidth {
            receiver,
            bandwidth_hz,
        } => receiver_state(state, *receiver)
            .and_then(|rx| rx.filter.bandwidth_hz)
            .is_some_and(|current| current == *bandwidth_hz),
        RadioCommand::SetReceiverFilterShift { receiver, shift_hz } => {
            receiver_state(state, *receiver)
                .and_then(|rx| rx.filter.shift_hz)
                .is_some_and(|current| current == *shift_hz)
        }
        RadioCommand::SetReceiverPreamp { receiver, setting } => receiver_state(state, *receiver)
            .and_then(|rx| rx.rf.preamp)
            .is_some_and(|current| current == *setting),
        RadioCommand::SetReceiverAttenuator { receiver, setting } => {
            receiver_state(state, *receiver)
                .and_then(|rx| rx.rf.attenuator)
                .is_some_and(|current| current == *setting)
        }
        RadioCommand::SetReceiverNoiseBlanker { receiver, setting } => {
            receiver_state(state, *receiver)
                .and_then(|rx| rx.rf.noise_blanker)
                .is_some_and(|current| current == *setting)
        }
        RadioCommand::SetReceiverNoiseReduction { receiver, setting } => {
            receiver_state(state, *receiver)
                .and_then(|rx| rx.rf.noise_reduction)
                .is_some_and(|current| current == *setting)
        }
        RadioCommand::SetReceiverAutoNotch { receiver, enabled } => {
            receiver_state(state, *receiver)
                .and_then(|rx| rx.rf.auto_notch)
                .is_some_and(|current| current == *enabled)
        }
        RadioCommand::SetTxFrequency(frequency) => state
            .tx
            .as_ref()
            .and_then(|tx| tx.frequency)
            .is_some_and(|current| current == *frequency),
        RadioCommand::SetTxMode(mode) => state
            .tx
            .as_ref()
            .and_then(|tx| tx.mode)
            .is_some_and(|current| current == *mode),
        RadioCommand::SetTxPower(power) => state
            .tx
            .as_ref()
            .and_then(|tx| tx.power)
            .is_some_and(|current| current == *power),
        RadioCommand::SetPtt(transmitting) | RadioCommand::SetDataPtt(transmitting) => state
            .tx
            .as_ref()
            .and_then(|tx| tx.transmitting)
            .is_some_and(|current| current == *transmitting),
        RadioCommand::SetSplit(split) => state
            .tx
            .as_ref()
            .and_then(|tx| tx.split)
            .is_some_and(|current| current == *split),
        RadioCommand::SetRitEnabled { receiver, enabled } => match receiver {
            ReceiverPath::Main => state.rit_xit.main_rit_enabled,
            ReceiverPath::Sub => state.rit_xit.sub_rit_enabled,
        }
        .is_some_and(|current| current == *enabled),
        RadioCommand::SetXitEnabled { receiver, enabled } => match receiver {
            ReceiverPath::Main => state.rit_xit.xit_enabled,
            ReceiverPath::Sub => state.rit_xit.sub_xit_enabled,
        }
        .is_some_and(|current| current == *enabled),
        RadioCommand::SetRitOffset { receiver, offset } => match receiver {
            ReceiverPath::Main => state.rit_xit.offset_hz,
            ReceiverPath::Sub => state.rit_xit.sub_offset_hz,
        }
        .is_some_and(|current| current == *offset),
        RadioCommand::SetXitOffset { receiver, offset } => match receiver {
            ReceiverPath::Main => state.rit_xit.xit_offset_hz,
            ReceiverPath::Sub => state.rit_xit.sub_xit_offset_hz,
        }
        .is_some_and(|current| current == *offset),
        RadioCommand::SetRitXitOffset { receiver, offset } => {
            let (rit, xit) = match receiver {
                ReceiverPath::Main => (state.rit_xit.offset_hz, state.rit_xit.xit_offset_hz),
                ReceiverPath::Sub => (state.rit_xit.sub_offset_hz, state.rit_xit.sub_xit_offset_hz),
            };
            rit.is_some_and(|current| current == *offset)
                && xit.is_some_and(|current| current == *offset)
        }
        RadioCommand::SetKeyerSpeed(wpm) => state
            .keyer
            .as_ref()
            .and_then(|keyer| keyer.speed_wpm)
            .is_some_and(|current| current == *wpm),
        RadioCommand::SendCw(_) | RadioCommand::StopCw | RadioCommand::Refresh => false,
    }
}

fn receiver_state(state: &RadioState, receiver: ReceiverPath) -> Option<&crate::ReceiverState> {
    match receiver {
        ReceiverPath::Main => Some(&state.main_rx),
        ReceiverPath::Sub => state.sub_rx.as_ref(),
    }
}

fn icom_validation_queries(
    profile: &'static IcomCivProfile,
    command: &RadioCommand,
    state_before: &RadioState,
) -> Vec<&'static str> {
    match command {
        RadioCommand::SetReceiverFrequency { receiver, .. } => match receiver {
            ReceiverPath::Main => vec!["freq-main"],
            ReceiverPath::Sub => vec!["freq-sub"],
        },
        RadioCommand::SetReceiverMode { receiver, .. } => match receiver {
            ReceiverPath::Main => vec!["mode-main"],
            ReceiverPath::Sub => vec!["mode-sub"],
        },
        RadioCommand::SetReceiverFilterBandwidth { .. } => vec!["filter-bandwidth"],
        RadioCommand::SetReceiverFilterShift { .. } => Vec::new(),
        RadioCommand::SetReceiverPreamp { receiver, .. } => {
            receiver_semantic(profile, *receiver, "preamp-main", "preamp-sub")
        }
        RadioCommand::SetReceiverAttenuator { receiver, .. } => {
            receiver_semantic(profile, *receiver, "attenuator-main", "attenuator-sub")
        }
        RadioCommand::SetReceiverNoiseBlanker { receiver, .. } => receiver_semantic(
            profile,
            *receiver,
            "noise-blanker-main",
            "noise-blanker-sub",
        ),
        RadioCommand::SetReceiverNoiseReduction { receiver, .. } => receiver_semantic(
            profile,
            *receiver,
            "noise-reduction-main",
            "noise-reduction-sub",
        ),
        RadioCommand::SetReceiverAutoNotch { receiver, .. } => {
            receiver_semantic(profile, *receiver, "auto-notch-main", "auto-notch-sub")
        }
        RadioCommand::SetTxFrequency(_) => vec!["tx-frequency"],
        RadioCommand::SetTxMode(_) => match tx_receiver_for_validation(state_before) {
            ReceiverPath::Main => vec!["mode-main"],
            ReceiverPath::Sub => vec!["mode-sub"],
        },
        RadioCommand::SetTxPower(_) => vec!["tx-power"],
        RadioCommand::SetPtt(_) | RadioCommand::SetDataPtt(_) => vec!["ptt"],
        RadioCommand::SetSplit(_) => vec!["split"],
        RadioCommand::SetRitEnabled { .. } => vec!["rit"],
        RadioCommand::SetXitEnabled { .. } => {
            if profile.capabilities.rit_xit.xit_enabled.can_read() {
                vec!["xit"]
            } else {
                Vec::new()
            }
        }
        RadioCommand::SetRitOffset { .. }
        | RadioCommand::SetXitOffset { .. }
        | RadioCommand::SetRitXitOffset { .. } => vec!["rit-offset"],
        RadioCommand::SetKeyerSpeed(_) => vec!["keyer-speed"],
        RadioCommand::SendCw(_) | RadioCommand::StopCw | RadioCommand::Refresh => Vec::new(),
    }
}

fn receiver_semantic(
    profile: &'static IcomCivProfile,
    receiver: ReceiverPath,
    main: &'static str,
    sub: &'static str,
) -> Vec<&'static str> {
    match receiver {
        ReceiverPath::Main => vec![main],
        ReceiverPath::Sub if profile.supports_command_29 => vec![sub],
        ReceiverPath::Sub => Vec::new(),
    }
}

fn tx_receiver_for_validation(state: &RadioState) -> ReceiverPath {
    if state.tx.as_ref().and_then(|tx| tx.split) == Some(true) {
        ReceiverPath::Sub
    } else {
        ReceiverPath::Main
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::VecDeque;

    use crate::{
        CatTransport, Frequency, Mode, StateReducer, protocol::kenwood_ascii::profile_by_id,
    };

    struct TestSink {
        reducer: StateReducer,
        updates: Vec<(Vec<StatePatch>, UpdateSource)>,
    }

    impl TestSink {
        fn new(state: RadioState) -> Self {
            Self {
                reducer: StateReducer::new(state),
                updates: Vec::new(),
            }
        }
    }

    impl StateSink for TestSink {
        fn state(&self) -> &RadioState {
            self.reducer.state()
        }

        fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource) {
            self.reducer.apply_patches(patches.clone());
            self.updates.push((patches, source));
        }
    }

    #[test]
    fn kenwood_session_poll_intervals_match_declared_update_strategy() {
        for profile in crate::protocol::kenwood_ascii::SUPPORTED_PROFILES {
            let session = kenwood_session(profile, KenwoodAsciiOptions::defaults()).unwrap();
            if profile.id() == "kenwood-if232" {
                assert_eq!(session.poll_interval(), Some(Duration::from_secs(2)));
            } else {
                assert_eq!(
                    session.poll_interval(),
                    None,
                    "{} unexpectedly polls",
                    profile.id()
                );
            }
        }
    }

    #[derive(Default)]
    struct TestTransport {
        reads: VecDeque<Vec<u8>>,
        writes: Vec<Vec<u8>>,
        pending_when_empty: bool,
    }

    #[async_trait]
    impl CatTransport for TestTransport {
        async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
            let Some(chunk) = self.reads.pop_front() else {
                if self.pending_when_empty {
                    return std::future::pending().await;
                }
                return Ok(0);
            };
            buf[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }

        async fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn ftdx10_auto_info_sequence_routes_md_as_main_and_sub() {
        let profile = profile_by_id("yaesu-ftdx10").unwrap();
        let mut routing = kenwood_ascii::VfoRouting::for_profile(profile);
        let mut reducer = StateReducer::new(RadioState::default());
        let initial_vs = AsciiFrame::new("VS1;").unwrap();
        let decoded = decode_kenwood_frame(profile, &initial_vs, reducer.state(), &mut routing)
            .unwrap()
            .unwrap();
        reducer.apply_patches(decoded.patches);

        for (sequence, expected_main, expected_sub) in [
            (["MD03;", "MD11;", "SH0007;", "VS0;"], Mode::Cw, Mode::Lsb),
            (["MD01;", "MD13;", "SH0018;", "VS1;"], Mode::Lsb, Mode::Cw),
        ] {
            for text in sequence {
                let frame = AsciiFrame::new(text).unwrap();
                if let Some(decoded) =
                    decode_kenwood_frame(profile, &frame, reducer.state(), &mut routing).unwrap()
                {
                    reducer.apply_patches(decoded.patches);
                }
            }

            assert_eq!(reducer.state().main_rx.mode, Some(expected_main));
            assert_eq!(
                reducer.state().sub_rx.as_ref().and_then(|rx| rx.mode),
                Some(expected_sub)
            );
        }
    }

    #[tokio::test]
    async fn kenwood_session_keeps_tx_routing_when_vfos_have_equal_frequencies() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut state = runtime.initial_state();
        state.main_rx.frequency = Some(Frequency::from_hz(14_074_000));
        state.sub_rx.as_mut().unwrap().frequency = Some(Frequency::from_hz(14_074_000));
        let mut sink = TestSink::new(state);

        let mut route_transport = TestTransport {
            reads: VecDeque::from([b"FR1;".to_vec()]),
            ..Default::default()
        };
        runtime
            .process_incoming(
                Some(&mut route_transport),
                COMMAND_RESPONSE_TIMEOUT,
                UpdateSource::Native,
                &mut sink,
            )
            .await
            .unwrap();

        let mut info_transport = TestTransport {
            reads: VecDeque::from([concat!(
                "IF",
                "00014074000",
                "00000",
                "+0000",
                "0",
                "0",
                "000",
                "0",
                "2",
                "1",
                "1",
                "00000",
                ";"
            )
            .as_bytes()
            .to_vec()]),
            ..Default::default()
        };
        runtime
            .process_incoming(
                Some(&mut info_transport),
                COMMAND_RESPONSE_TIMEOUT,
                UpdateSource::Native,
                &mut sink,
            )
            .await
            .unwrap();

        let mut command_transport = TestTransport {
            reads: VecDeque::from([b"FA00014075000;".to_vec()]),
            ..Default::default()
        };
        let state_before = sink.state().clone();
        let completion = runtime
            .execute(
                Some(&mut command_transport),
                RadioCommand::SetTxFrequency(Frequency::from_hz(14_075_000)),
                &state_before,
                &mut sink,
            )
            .await
            .unwrap();

        assert_eq!(completion, CommandCompletion::Observed);
        assert_eq!(command_transport.writes, vec![b"FA00014075000;".to_vec()]);
        assert_eq!(
            sink.state()
                .sub_rx
                .as_ref()
                .and_then(|receiver| receiver.frequency),
            Some(Frequency::from_hz(14_075_000))
        );
        assert_eq!(
            sink.state().tx.as_ref().and_then(|tx| tx.frequency),
            Some(Frequency::from_hz(14_075_000))
        );
    }

    #[tokio::test]
    async fn kenwood_rejection_leaves_accepted_state_unchanged() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"?;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::ProtocolSyntax { .. }));
        assert_eq!(sink.state().main_rx.frequency, None);
        assert!(sink.updates.is_empty());
    }

    #[tokio::test]
    async fn kenwood_communication_error_remains_structured() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"E;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::ProtocolCommunication));
        assert!(sink.updates.is_empty());
    }

    #[tokio::test]
    async fn kenwood_validation_failure_aborts_before_setter() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"?;".to_vec()]),
            ..Default::default()
        };
        let mut state = RadioState::default();
        state.main_rx.frequency = Some(Frequency::from_hz(7_030_000));
        let mut sink = TestSink::new(state);
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::ProtocolSyntax { .. }));
        assert_eq!(transport.writes, vec![b"FA;".to_vec()]);
        assert!(sink.updates.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn kenwood_timeout_returns_without_hidden_recovery_queries() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            pending_when_empty: true,
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::Timeout { .. }));
        assert_eq!(transport.writes, vec![b"FA00007030000;".to_vec()]);
        assert!(sink.updates.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn kenwood_busy_retries_once_after_delay_then_succeeds() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"O;".to_vec(), b"FA00007030000;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();
        let started_at = Instant::now();

        let completion = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap();

        assert_eq!(completion, CommandCompletion::Observed);
        assert_eq!(transport.writes, vec![b"FA00007030000;".to_vec(); 2]);
        assert_eq!(Instant::now() - started_at, Duration::from_millis(250));
    }

    #[tokio::test(start_paused = true)]
    async fn kenwood_busy_retry_exhaustion_returns_structured_error() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"O;".to_vec(), b"O;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::ProtocolBusy));
        assert_eq!(transport.writes, vec![b"FA00007030000;".to_vec(); 2]);
        assert!(sink.updates.is_empty());
    }

    #[tokio::test]
    async fn kenwood_malformed_matching_response_fails_without_state_update() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"FAbad;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::Decode { .. }));
        assert!(sink.updates.is_empty());
    }

    #[tokio::test]
    async fn kenwood_failed_first_step_aborts_multiframe_command() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"?;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverMode {
            receiver: ReceiverPath::Main,
            mode: Mode::DataUsb,
        };
        let state_before = sink.state().clone();

        let error = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(error, RadioError::ProtocolSyntax { .. }));
        assert_eq!(transport.writes, vec![b"MD2;".to_vec()]);
        assert!(sink.updates.is_empty());
    }

    #[tokio::test]
    async fn kenwood_multiframe_command_requires_each_matching_response() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"MD2;".to_vec(), b"DA1;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverMode {
            receiver: ReceiverPath::Main,
            mode: Mode::DataUsb,
        };
        let state_before = sink.state().clone();

        let completion = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap();

        assert_eq!(completion, CommandCompletion::Observed);
        assert_eq!(transport.writes, vec![b"MD2;".to_vec(), b"DA1;".to_vec()]);
        assert_eq!(sink.state().main_rx.mode, Some(Mode::DataUsb));
    }

    #[tokio::test]
    async fn kenwood_ignores_unhandled_frame_after_matched_response_in_same_batch() {
        let profile = profile_by_id("elecraft-k4").unwrap();
        let mut runtime = KenwoodAsciiRuntime::new(
            profile,
            KenwoodAsciiOptions::parse("rtty_data_submode=fsk").unwrap(),
        );
        let mut transport = TestTransport {
            reads: VecDeque::from([b"MD6;".to_vec(), b"DT2;ZZ;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let state_before = sink.state().clone();

        let completion = runtime
            .execute(
                Some(&mut transport),
                RadioCommand::SetReceiverMode {
                    receiver: ReceiverPath::Main,
                    mode: Mode::Rtty,
                },
                &state_before,
                &mut sink,
            )
            .await
            .unwrap();

        assert_eq!(completion, CommandCompletion::Observed);
        assert_eq!(transport.writes, vec![b"MD6;".to_vec(), b"DT2;".to_vec()]);
        assert_eq!(sink.state().main_rx.mode, Some(Mode::Rtty));
    }

    #[tokio::test]
    async fn kenwood_refresh_reissues_startup_steps_and_propagates_failure() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport::default();
        let mut sink = TestSink::new(RadioState::default());

        let error = runtime
            .refresh(Some(&mut transport), &mut sink)
            .await
            .unwrap_err();

        assert!(error.is_transport());
        assert_eq!(transport.writes, vec![b"AI2;".to_vec(), b"IF;".to_vec()]);
        assert!(sink.updates.is_empty());
    }

    #[tokio::test]
    async fn kenwood_observed_response_wins_over_requested_value() {
        let profile = profile_by_id("kenwood-ts590").unwrap();
        let mut runtime =
            KenwoodAsciiRuntime::new(profile, KenwoodAsciiOptions::parse("").unwrap());
        let mut transport = TestTransport {
            reads: VecDeque::from([b"FA00007031000;".to_vec()]),
            ..Default::default()
        };
        let mut sink = TestSink::new(RadioState::default());
        let command = RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: Frequency::from_hz(7_030_000),
        };
        let state_before = sink.state().clone();

        let completion = runtime
            .execute(Some(&mut transport), command, &state_before, &mut sink)
            .await
            .unwrap();

        assert_eq!(completion, CommandCompletion::Observed);
        assert_eq!(
            sink.state().main_rx.frequency,
            Some(Frequency::from_hz(7_031_000))
        );
        assert_eq!(sink.updates.len(), 1);
        assert_eq!(sink.updates[0].1, UpdateSource::CommandResponse);
    }

    #[tokio::test]
    async fn smartsdr_refresh_reissues_subscriptions_with_manual_provenance() {
        let profile = smartsdr::profile_by_id("flexradio-smartsdr").unwrap();
        let mut runtime = SmartSdrRuntime::new(*profile);
        let mut transport = TestTransport {
            reads: VecDeque::from([
                b"S0|slice 0 RF_frequency=7.030 mode=CW filter_lo=300 filter_hi=2700 rit_on=0 rit_freq=0 xit_on=0 xit_freq=0 nr=off nb=off anf=off\nR1|0||\n".to_vec(),
                b"R2|0||\n".to_vec(),
                b"R3|0||\n".to_vec(),
            ]),
            ..Default::default()
        };
        let mut sink = TestSink::new(runtime.initial_state());

        runtime
            .refresh(Some(&mut transport), &mut sink)
            .await
            .unwrap();

        assert_eq!(
            transport.writes,
            vec![
                b"C1|sub slice 0\n".to_vec(),
                b"C2|sub cwx all\n".to_vec(),
                b"C3|sub tx all\n".to_vec(),
            ]
        );
        assert_eq!(
            sink.state().main_rx.frequency,
            Some(Frequency::from_hz(7_030_000))
        );
        assert!(
            sink.updates
                .iter()
                .any(|(_, source)| *source == UpdateSource::ManualRefresh)
        );
    }
}
