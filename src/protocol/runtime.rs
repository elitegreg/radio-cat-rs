use async_trait::async_trait;
use tokio::time::{timeout, Duration, Instant};

use crate::{
    driver::{DriverDescriptor, RadioSession, StateSink},
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
    ConnectionState, KeyerState, RadioCapabilities, RadioCommand, RadioState, ReceiverPath,
    ReceiverState, RitXitState, StatePatch, TransmitterState, UpdateSource,
};

const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const SMARTSDR_STARTUP_TIMEOUT: Duration = Duration::from_millis(1_500);

pub(crate) fn kenwood_session(
    profile: &'static KenwoodAsciiProfile,
    options: &str,
) -> Result<Box<dyn RadioSession>> {
    Ok(Box::new(KenwoodAsciiRuntime::new(
        profile,
        KenwoodAsciiOptions::parse(options)?,
    )))
}

pub(crate) fn icom_session(
    profile: &'static IcomCivProfile,
    options: &str,
) -> Result<Box<dyn RadioSession>> {
    Ok(Box::new(IcomCivRuntime::new(
        profile,
        IcomCivOptions::parse(profile, options)?,
    )))
}

pub(crate) fn smartsdr_session(
    profile: &'static SmartSdrProfile,
    options: &str,
) -> Result<Box<dyn RadioSession>> {
    let options = smartsdr::SmartSdrOptions::parse(profile, options)?;
    let mut profile = *profile;
    profile.slice = options.slice;
    Ok(Box::new(SmartSdrRuntime::new(profile)))
}

struct KenwoodAsciiRuntime {
    profile: &'static KenwoodAsciiProfile,
    options: KenwoodAsciiOptions,
    frame_splitter: KenwoodFrameSplitter,
    vfo_routing: kenwood_ascii::VfoRouting,
}

impl KenwoodAsciiRuntime {
    fn new(profile: &'static KenwoodAsciiProfile, options: KenwoodAsciiOptions) -> Self {
        Self {
            profile,
            options,
            frame_splitter: KenwoodFrameSplitter::new(),
            vfo_routing: kenwood_ascii::VfoRouting::for_profile(profile),
        }
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let frame_count = encoded.frames.len();
        for (index, frame) in encoded.frames.into_iter().enumerate() {
            let is_last = index + 1 == frame_count;

            tracing::debug!(
                driver = %self.profile.id(),
                tx_frame = frame.as_str(),
                priority = ?encoded.priority,
                "sending CAT frame"
            );

            transport.write_all(frame.as_bytes()).await?;
            transport.flush().await?;

            if is_last && matcher_expects_response(&encoded.matcher) {
                let matched = self
                    .process_incoming_with_expected(
                        transport,
                        wait_timeout,
                        default_source,
                        Some(&encoded.matcher),
                        ctx,
                    )
                    .await?;
                if !matched {
                    return Err(RadioError::Timeout {
                        command: "command-response",
                    });
                }
            }
        }

        Ok(())
    }

    async fn process_incoming_with_expected(
        &mut self,
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        expected: Option<&ResponseMatcher>,
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
                tracing::trace!(driver = %self.profile.id(), "transport read yielded EOF/empty");
                return Ok(false);
            }

            let mut matched_expected = false;
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
                        return Ok(false);
                    }
                    continue;
                }

                if let Some(expected) = expected {
                    if matcher_matches_frame(expected, &frame) {
                        tracing::debug!(
                            driver = %self.profile.id(),
                            rx_frame = frame.as_str(),
                            expected = ?expected,
                            "received expected CAT response"
                        );
                        matched_expected = true;
                    }
                }

                match decode_kenwood_frame(self.profile, &frame, ctx.state(), &mut self.vfo_routing)
                {
                    Ok(Some(decoded)) => {
                        let source = decoded.source_hint.unwrap_or(default_source);
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
                        tracing::trace!(
                            driver = %self.profile.id(),
                            rx_frame = frame.as_str(),
                            "unhandled CAT frame"
                        );
                    }
                    Err(error) => {
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

            if matched_expected {
                return Ok(true);
            }
        }
    }

    async fn recover_timeout(
        &mut self,
        transport: &mut dyn CatTransport,
        command: &RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn StateSink,
    ) {
        for semantic in kenwood_timeout_recovery_queries(self.profile, command, state_before) {
            let Some(encoded) =
                (match encode_kenwood_query(self.profile, semantic, self.vfo_routing) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        tracing::warn!(
                            driver = %self.profile.id(),
                            ?command,
                            semantic,
                            ?error,
                            "failed to encode Kenwood timeout recovery query"
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
                    "Kenwood timeout recovery query failed; continuing"
                );
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
            vec![
                StatePatch::Connection(ConnectionState::Identifying),
                StatePatch::Connection(ConnectionState::Ready),
            ],
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
                        tracing::warn!(
                            driver = %self.profile.id(),
                            step = step.label(),
                            ?error,
                            "startup auto-info step failed; continuing"
                        );
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
                        tracing::warn!(
                            driver = %self.profile.id(),
                            semantic,
                            ?error,
                            "startup query failed; continuing"
                        );
                    }
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
    ) -> Result<()> {
        if matches!(command, RadioCommand::Refresh) {
            return Ok(());
        }

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

        ctx.publish_patches(encoded.optimistic.clone(), UpdateSource::Optimistic);

        let Some(transport) = transport else {
            return Ok(());
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
                        "Kenwood validation query failed; continuing with setter"
                    );
                }
            }

            if command_matches_state(&command, ctx.state()) {
                tracing::debug!(
                    driver = %self.profile.id(),
                    ?command,
                    "validated current state; skipping Kenwood setter"
                );
                return Ok(());
            }
        }

        tracing::debug!(
            driver = %self.profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            "dispatching command over transport"
        );

        match self
            .send_encoded(
                transport,
                encoded,
                UpdateSource::CommandResponse,
                COMMAND_RESPONSE_TIMEOUT,
                ctx,
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(error @ RadioError::Timeout { .. }) => {
                tracing::warn!(
                    driver = %self.profile.id(),
                    ?command,
                    ?error,
                    "Kenwood-ASCII set command timed out; querying current state instead"
                );
                self.recover_timeout(transport, &command, state_before, ctx)
                    .await;
                Ok(())
            }
            Err(error) => Err(error),
        }
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
        self.process_incoming_with_expected(transport, wait_timeout, default_source, None, ctx)
            .await
    }

    async fn poll(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let Some(transport) = transport else {
            return Ok(());
        };
        if let Some(plan) = self.profile.poll {
            tracing::debug!(driver = %self.profile.id(), query_count = plan.queries.len(), "running poll plan");
            for semantic in plan.queries {
                let Some(encoded) = encode_kenwood_query(self.profile, semantic, self.vfo_routing)?
                else {
                    continue;
                };

                if let Err(error) = self
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
            }
        }

        Ok(())
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
}

impl IcomCivRuntime {
    fn new(profile: &'static IcomCivProfile, options: IcomCivOptions) -> Self {
        Self {
            profile,
            options,
            frame_splitter: IcomFrameSplitter::new(),
        }
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: icom_civ::EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let icom_civ::EncodedCommand {
            frames,
            matcher,
            response_receiver,
            ..
        } = encoded;

        for frame in frames {
            tracing::debug!(
                driver = %self.profile.id(),
                tx_bytes = ?frame.as_bytes(),
                "sending ICOM CI-V frame"
            );

            transport.write_all(frame.as_bytes()).await?;
            transport.flush().await?;

            if matcher.expects_response() {
                match self
                    .process_incoming_with_expected(
                        transport,
                        wait_timeout,
                        default_source,
                        Some(&matcher),
                        Some(frame.as_bytes()),
                        response_receiver,
                        ctx,
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
                        return Err(RadioError::protocol_syntax(Some("icom-civ")));
                    }
                    IcomWaitOutcome::Collision => {
                        return Err(RadioError::ProtocolCommunication);
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_incoming_with_expected(
        &mut self,
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        expected: Option<&IcomResponseMatcher>,
        echo: Option<&[u8]>,
        receiver_hint: Option<crate::ReceiverPath>,
        ctx: &mut dyn StateSink,
    ) -> Result<IcomWaitOutcome> {
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
                    })
                }
            };

            if count == 0 {
                tracing::trace!(driver = %self.profile.id(), "ICOM transport read yielded EOF/empty");
                return Ok(if expected.is_none() && saw_frames {
                    IcomWaitOutcome::Matched
                } else {
                    IcomWaitOutcome::Timeout
                });
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
                saw_frames = true;
                if echo.is_some_and(|sent| frame.is_echo_of(sent)) {
                    tracing::trace!(driver = %self.profile.id(), rx_bytes = ?frame.as_bytes(), "discarding ICOM self-echo frame");
                    continue;
                }

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
                if let Some(expected) = expected {
                    if expected.matches(&frame) {
                        tracing::debug!(
                            driver = %self.profile.id(),
                            rx_bytes = ?frame.as_bytes(),
                            expected = ?expected,
                            "received expected ICOM CI-V response"
                        );
                        matched_expected = true;
                    }
                }

                self.decode_and_publish_frame(&frame, default_source, receiver_hint, ctx);

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
                let source = decoded.source_hint.unwrap_or(default_source);
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
            vec![
                StatePatch::Connection(ConnectionState::Identifying),
                StatePatch::Connection(ConnectionState::Ready),
            ],
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
                        tracing::warn!(
                            driver = %self.profile.id(),
                            semantic,
                            ?error,
                            "ICOM startup query failed; continuing"
                        );
                    }
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
    ) -> Result<()> {
        if matches!(command, RadioCommand::Refresh) {
            return Ok(());
        }

        let Some(encoded) = icom_civ::encode(self.profile, self.options, &command, state_before)?
        else {
            return Err(RadioError::UnsupportedCapability {
                capability: "command",
            });
        };

        ctx.publish_patches(encoded.optimistic.clone(), UpdateSource::Optimistic);

        let Some(transport) = transport else {
            return Ok(());
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
                return Ok(());
            }
        }

        tracing::debug!(
            driver = %self.profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            "dispatching ICOM command over transport"
        );

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
                None,
                ctx,
            )
            .await?,
            IcomWaitOutcome::Matched
        ))
    }

    async fn poll(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let Some(transport) = transport else {
            return Ok(());
        };
        if let Some(plan) = self.profile.poll {
            tracing::debug!(driver = %self.profile.id(), query_count = plan.queries.len(), "running ICOM poll plan");
            for semantic in plan.queries {
                let Some(encoded) = icom_civ::encode_query(self.profile, self.options, semantic)?
                else {
                    continue;
                };

                if let Err(error) = self
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
            }
        }

        Ok(())
    }
}

struct SmartSdrRuntime {
    profile: SmartSdrProfile,
    line_splitter: SmartSdrLineSplitter,
    next_sequence: u32,
    version: Option<String>,
    handle: Option<String>,
    saw_slice_status: bool,
}

impl SmartSdrRuntime {
    fn new(profile: SmartSdrProfile) -> Self {
        Self {
            profile,
            line_splitter: SmartSdrLineSplitter::new(),
            next_sequence: 1,
            version: None,
            handle: None,
            saw_slice_status: false,
        }
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: smartsdr::EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        let smartsdr::EncodedCommand {
            commands,
            optimistic,
        } = encoded;

        for command in commands {
            self.send_command_body(transport, &command, default_source, wait_timeout, ctx)
                .await?;
        }

        if !optimistic.is_empty() {
            ctx.publish_patches(optimistic, default_source);
        }

        Ok(())
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
        transport.write_all(frame.as_bytes()).await?;
        transport.flush().await?;

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
                    })
                }
            };

            if count == 0 {
                return Ok(if expected_sequence.is_none() && saw_lines {
                    SmartSdrWaitOutcome::Matched
                } else {
                    SmartSdrWaitOutcome::Timeout
                });
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
                        if message.starts_with("slice ") {
                            self.saw_slice_status = true;
                        }
                        match smartsdr::decode_status(&self.profile, &message, ctx.state()) {
                            Ok(Some(decoded)) => {
                                let source = decoded.source_hint.unwrap_or(default_source);
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
                                            let source =
                                                decoded.source_hint.unwrap_or(default_source);
                                            ctx.publish_patches(decoded.patches, source);
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

            if expected_sequence.is_none() && saw_lines {
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

        ctx.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Ready)],
            UpdateSource::Native,
        );

        Ok(())
    }

    async fn execute(
        &mut self,
        transport: Option<&mut dyn CatTransport>,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn StateSink,
    ) -> Result<()> {
        if matches!(command, RadioCommand::Refresh) {
            return Ok(());
        }

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
            return Ok(());
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

    async fn poll(
        &mut self,
        _transport: Option<&mut dyn CatTransport>,
        _ctx: &mut dyn StateSink,
    ) -> Result<()> {
        Ok(())
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
    if let Some(encoded) =
        kenwood_ascii::mode::encode_with_routing(profile, command, current_state, vfo_routing)?
    {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::split::encode(profile, command, current_state)? {
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
    if let Some(decoded) = kenwood_ascii::rf::decode_with_routing(profile, frame, *vfo_routing)? {
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

fn matcher_expects_response(matcher: &ResponseMatcher) -> bool {
    !matches!(matcher, ResponseMatcher::None)
}

fn kenwood_timeout_recovery_queries(
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
        RadioCommand::SetXitEnabled(_) => vec!["XT"],
        RadioCommand::SetRitOffset { receiver, .. } => rit_offset_queries(profile, *receiver),
        RadioCommand::SetXitOffset(_) | RadioCommand::SetRitXitOffset(_) => {
            rit_offset_queries(profile, ReceiverPath::Main)
        }
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

fn matcher_matches_frame(matcher: &ResponseMatcher, frame: &AsciiFrame) -> bool {
    match matcher {
        ResponseMatcher::None => false,
        ResponseMatcher::Exact(expected) => frame.as_str() == *expected,
        ResponseMatcher::Prefix(prefix) => frame.as_str().starts_with(prefix),
        ResponseMatcher::OneOf(prefixes) => prefixes
            .iter()
            .any(|prefix| frame.as_str().starts_with(prefix)),
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
        RadioCommand::SetXitEnabled(enabled) => state
            .rit_xit
            .xit_enabled
            .is_some_and(|current| current == *enabled),
        RadioCommand::SetRitOffset { receiver, offset } => match receiver {
            ReceiverPath::Main => state.rit_xit.offset_hz,
            ReceiverPath::Sub => state.rit_xit.sub_offset_hz,
        }
        .is_some_and(|current| current == *offset),
        RadioCommand::SetXitOffset(offset) => state
            .rit_xit
            .xit_offset_hz
            .is_some_and(|current| current == *offset),
        RadioCommand::SetRitXitOffset(offset) => {
            state
                .rit_xit
                .offset_hz
                .is_some_and(|current| current == *offset)
                && state
                    .rit_xit
                    .xit_offset_hz
                    .is_some_and(|current| current == *offset)
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

fn kenwood_validation_queries(
    profile: &'static KenwoodAsciiProfile,
    command: &RadioCommand,
    state_before: &RadioState,
) -> Vec<&'static str> {
    kenwood_timeout_recovery_queries(profile, command, state_before)
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
        RadioCommand::SetXitEnabled(_) => {
            if profile.capabilities.rit_xit.xit_enabled.can_read() {
                vec!["xit"]
            } else {
                Vec::new()
            }
        }
        RadioCommand::SetRitOffset { .. }
        | RadioCommand::SetXitOffset(_)
        | RadioCommand::SetRitXitOffset(_) => vec!["rit-offset"],
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
    use crate::{protocol::kenwood_ascii::profile_by_id, Mode, StateReducer};

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
}
