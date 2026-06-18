use async_trait::async_trait;
use tokio::time::{timeout, Duration, Instant};

use crate::{
    error::{RadioError, Result},
    protocol::{
        icom_civ::{
            self, CivFrame, FrameSplitter as IcomFrameSplitter, IcomCivOptions, IcomCivProfile,
            ResponseMatcher as IcomResponseMatcher,
        },
        kenwood_ascii::{
            self, AsciiFrame, CommandPriority, EncodedCommand,
            FrameSplitter as KenwoodFrameSplitter, KenwoodAsciiProfile, ResponseMatcher,
            StartupStep,
        },
        smartsdr::{self, LineSplitter as SmartSdrLineSplitter, SmartSdrProfile},
    },
    transport::CatTransport,
    ConnectionState, RadioCommand, RadioState, ReceiverPath, StatePatch, UpdateSource,
};

const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const SMARTSDR_STARTUP_TIMEOUT: Duration = Duration::from_millis(1_500);

pub(crate) trait ProtocolContext: Send {
    fn state(&self) -> &RadioState;
    fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource);
}

#[async_trait]
pub(crate) trait NativeProtocol: Send {
    fn id(&self) -> &'static str;
    fn poll_interval(&self) -> Option<Duration>;

    async fn startup(
        &mut self,
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()>;

    async fn dispatch_command(
        &mut self,
        transport: &mut dyn CatTransport,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()>;

    async fn process_incoming(
        &mut self,
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<bool>;

    async fn poll(
        &mut self,
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()>;
}

pub(crate) fn native_protocol_for_driver(
    driver_id: &str,
    options: &str,
) -> Result<Option<Box<dyn NativeProtocol>>> {
    if let Some(profile) = kenwood_ascii::profile_by_id(driver_id) {
        return Ok(Some(Box::new(KenwoodAsciiRuntime::new(profile))));
    }

    if let Some(profile) = icom_civ::profile_by_id(driver_id) {
        let options = IcomCivOptions::parse(profile, options)?;
        return Ok(Some(Box::new(IcomCivRuntime::new(profile, options))));
    }

    if let Some(profile) = smartsdr::profile_by_id(driver_id) {
        return Ok(Some(Box::new(SmartSdrRuntime::new(profile))));
    }

    Ok(None)
}

struct KenwoodAsciiRuntime {
    profile: &'static KenwoodAsciiProfile,
    frame_splitter: KenwoodFrameSplitter,
}

impl KenwoodAsciiRuntime {
    fn new(profile: &'static KenwoodAsciiProfile) -> Self {
        Self {
            profile,
            frame_splitter: KenwoodFrameSplitter::new(),
        }
    }

    async fn send_encoded(
        &mut self,
        transport: &mut dyn CatTransport,
        encoded: EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn ProtocolContext,
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
        ctx: &mut dyn ProtocolContext,
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

                match decode_kenwood_frame(self.profile, &frame, ctx.state()) {
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
        ctx: &mut dyn ProtocolContext,
    ) {
        for semantic in kenwood_timeout_recovery_queries(self.profile, command, state_before) {
            let Some(encoded) = (match encode_kenwood_query(self.profile, semantic) {
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
            }) else {
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
impl NativeProtocol for KenwoodAsciiRuntime {
    fn id(&self) -> &'static str {
        self.profile.id()
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.profile.poll.map(|plan| plan.interval)
    }

    async fn startup(
        &mut self,
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
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
                    let Some(encoded) = encode_kenwood_query(self.profile, semantic)? else {
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

    async fn dispatch_command(
        &mut self,
        transport: &mut dyn CatTransport,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
        if command_matches_state(&command, state_before) {
            for semantic in kenwood_validation_queries(self.profile, &command, state_before) {
                let Some(encoded) = (match encode_kenwood_query(self.profile, semantic) {
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
                }) else {
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

        let Some(encoded) = encode_kenwood_command(self.profile, &command, state_before)? else {
            tracing::trace!(driver = %self.profile.id(), ?command, "command has no native transport encoding");
            return Ok(());
        };

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
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<bool> {
        self.process_incoming_with_expected(transport, wait_timeout, default_source, None, ctx)
            .await
    }

    async fn poll(
        &mut self,
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
        if let Some(plan) = self.profile.poll {
            tracing::debug!(driver = %self.profile.id(), query_count = plan.queries.len(), "running poll plan");
            for semantic in plan.queries {
                let Some(encoded) = encode_kenwood_query(self.profile, semantic)? else {
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
        ctx: &mut dyn ProtocolContext,
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
        ctx: &mut dyn ProtocolContext,
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
        ctx: &mut dyn ProtocolContext,
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
impl NativeProtocol for IcomCivRuntime {
    fn id(&self) -> &'static str {
        self.profile.id()
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.profile.poll.map(|_| self.options.poll_interval)
    }

    async fn startup(
        &mut self,
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
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

    async fn dispatch_command(
        &mut self,
        transport: &mut dyn CatTransport,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
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

        let Some(encoded) = icom_civ::encode(self.profile, self.options, &command, state_before)?
        else {
            tracing::trace!(driver = %self.profile.id(), ?command, "command has no ICOM native transport encoding");
            return Ok(());
        };

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
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<bool> {
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
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
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
    profile: &'static SmartSdrProfile,
    line_splitter: SmartSdrLineSplitter,
    next_sequence: u32,
    version: Option<String>,
    handle: Option<String>,
    saw_slice_status: bool,
}

impl SmartSdrRuntime {
    fn new(profile: &'static SmartSdrProfile) -> Self {
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
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
        for command in encoded.commands {
            self.send_command_body(transport, &command, default_source, wait_timeout, ctx)
                .await?;
        }

        Ok(())
    }

    async fn send_command_body(
        &mut self,
        transport: &mut dyn CatTransport,
        command: &str,
        default_source: UpdateSource,
        wait_timeout: Duration,
        ctx: &mut dyn ProtocolContext,
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
        ctx: &mut dyn ProtocolContext,
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
                        match smartsdr::decode_status(self.profile, &message, ctx.state()) {
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
                            response_outcome = Some(if response.code == 0 {
                                SmartSdrWaitOutcome::Matched
                            } else {
                                SmartSdrWaitOutcome::Rejected {
                                    code: response.code,
                                    message: response.message,
                                }
                            });
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
impl NativeProtocol for SmartSdrRuntime {
    fn id(&self) -> &'static str {
        self.profile.id()
    }

    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    async fn startup(
        &mut self,
        transport: &mut dyn CatTransport,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
        let _ = self
            .process_incoming_with_expected(
                transport,
                STARTUP_RESPONSE_TIMEOUT,
                UpdateSource::Native,
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

        let _ = self
            .process_incoming_with_expected(
                transport,
                STARTUP_RESPONSE_TIMEOUT,
                UpdateSource::Native,
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

    async fn dispatch_command(
        &mut self,
        transport: &mut dyn CatTransport,
        command: RadioCommand,
        state_before: &RadioState,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
        if command_matches_state(&command, state_before) {
            tracing::debug!(
                driver = %self.profile.id(),
                ?command,
                "validated current state; skipping SmartSDR setter"
            );
            return Ok(());
        }

        let Some(encoded) = smartsdr::encode(self.profile, &command, state_before)? else {
            return Ok(());
        };

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
        transport: &mut dyn CatTransport,
        wait_timeout: Duration,
        default_source: UpdateSource,
        ctx: &mut dyn ProtocolContext,
    ) -> Result<bool> {
        Ok(matches!(
            self.process_incoming_with_expected(
                transport,
                wait_timeout,
                default_source,
                None,
                ctx,
            )
            .await?,
            SmartSdrWaitOutcome::Matched
        ))
    }

    async fn poll(
        &mut self,
        _transport: &mut dyn CatTransport,
        _ctx: &mut dyn ProtocolContext,
    ) -> Result<()> {
        Ok(())
    }
}

fn encode_kenwood_command(
    profile: &'static KenwoodAsciiProfile,
    command: &RadioCommand,
    current_state: &RadioState,
) -> Result<Option<EncodedCommand>> {
    if let Some(encoded) = kenwood_ascii::frequency::encode(profile, command, current_state)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::mode::encode(profile, command, current_state)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::split::encode(profile, command, current_state)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::rit_xit::encode(profile, command, current_state)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::filter::encode(profile, command, current_state)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::rf::encode(profile, command)? {
        return Ok(Some(encoded));
    }
    if let Some(encoded) = kenwood_ascii::tx::encode(profile, command)? {
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
) -> Result<Option<EncodedCommand>> {
    if let Some(encoded) = kenwood_ascii::frequency::encode_query(profile, semantic)? {
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
) -> Result<Option<kenwood_ascii::DecodedFrame>> {
    if let Some(decoded) = kenwood_ascii::info::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::frequency::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::mode::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::split::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::rit_xit::decode(profile, frame)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::filter::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = kenwood_ascii::rf::decode(profile, frame)? {
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
        RadioCommand::SetPtt(_) => vec!["IF"],
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
        RadioCommand::SetPtt(transmitting) => state
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
        RadioCommand::SetPtt(_) => vec!["ptt"],
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
