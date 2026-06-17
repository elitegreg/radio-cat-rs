use std::sync::Arc;

use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::{timeout, Duration, Instant},
};

use crate::{
    driver::RadioDriver,
    error::{RadioError, Result},
    protocol::{
        icom_civ::{
            self, CivFrame, FrameSplitter as IcomFrameSplitter, IcomCivOptions, IcomCivProfile,
            ResponseMatcher as IcomResponseMatcher,
        },
        kenwood_ascii::{
            self as kenwood_ascii, AsciiFrame, CommandPriority, EncodedCommand,
            FrameSplitter as KenwoodFrameSplitter, KenwoodAsciiProfile, ResponseMatcher,
            StartupStep,
        },
    },
    transport::BoxedCatTransport,
    update::{SharedRadioState, StateReducer, StateUpdate},
    RadioCommand, RadioState, ReceiverPath, StatePatch, UpdateSource,
};

const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_millis(900);
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_millis(1200);
const COMMAND_LOOP_IDLE_TICK: Duration = Duration::from_millis(50);

pub(crate) struct CommandEnvelope {
    pub command: RadioCommand,
    pub result_tx: oneshot::Sender<Result<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IcomWaitOutcome {
    Matched,
    Timeout,
    Rejected,
    Collision,
}

pub struct RadioTask {
    driver: Box<dyn RadioDriver>,
    reducer: StateReducer,
    command_rx: mpsc::Receiver<CommandEnvelope>,
    state_tx: watch::Sender<SharedRadioState>,
    update_tx: broadcast::Sender<StateUpdate>,
    transport: Option<BoxedCatTransport>,
    kenwood_frame_splitter: KenwoodFrameSplitter,
    icom_frame_splitter: IcomFrameSplitter,
    kenwood_profile: Option<&'static KenwoodAsciiProfile>,
    icom_profile: Option<&'static IcomCivProfile>,
    icom_options: Option<IcomCivOptions>,
    driver_options: String,
    next_poll_at: Option<Instant>,
}

impl RadioTask {
    pub(crate) fn new(
        driver: Box<dyn RadioDriver>,
        initial_state: crate::RadioState,
        command_rx: mpsc::Receiver<CommandEnvelope>,
        state_tx: watch::Sender<SharedRadioState>,
        update_tx: broadcast::Sender<StateUpdate>,
        transport: Option<BoxedCatTransport>,
        driver_options: String,
    ) -> Self {
        Self {
            driver,
            reducer: StateReducer::new(initial_state),
            command_rx,
            state_tx,
            update_tx,
            transport,
            kenwood_frame_splitter: KenwoodFrameSplitter::new(),
            icom_frame_splitter: IcomFrameSplitter::new(),
            kenwood_profile: None,
            icom_profile: None,
            icom_options: None,
            driver_options,
            next_poll_at: None,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let driver = self.driver.descriptor();
        self.kenwood_profile = kenwood_ascii::profile_by_id(driver.id);
        self.icom_profile = icom_civ::profile_by_id(driver.id);
        if let Some(profile) = self.icom_profile {
            self.icom_options = Some(IcomCivOptions::parse(profile, &self.driver_options)?);
        }

        tracing::info!(driver = %driver.id, "radio task run loop starting");

        match self.driver.start().await {
            Ok(patches) => {
                tracing::info!(driver = %driver.id, patch_count = patches.len(), "driver startup complete");
                self.publish_patches(patches, UpdateSource::Native)
            }
            Err(error) => {
                tracing::error!(driver = %driver.id, ?error, "driver startup failed");
                let message = error.to_string();
                self.publish_patches(
                    vec![StatePatch::Connection(crate::ConnectionState::Error {
                        message,
                    })],
                    UpdateSource::Native,
                );
                return Err(error);
            }
        }

        if self.kenwood_profile.is_some() && self.transport.is_some() {
            tracing::info!(driver = %driver.id, "starting kenwood-ascii transport bootstrap");
            self.run_kenwood_startup().await?;
            self.schedule_next_poll();
            tracing::info!(driver = %driver.id, "kenwood-ascii transport bootstrap complete");
        }
        if self.icom_profile.is_some() && self.transport.is_some() {
            tracing::info!(driver = %driver.id, "starting icom-civ transport bootstrap");
            self.run_icom_startup().await?;
            self.schedule_next_poll();
            tracing::info!(driver = %driver.id, "icom-civ transport bootstrap complete");
        }

        tracing::info!(driver = %driver.id, "radio task command loop started");
        loop {
            match timeout(COMMAND_LOOP_IDLE_TICK, self.command_rx.recv()).await {
                Ok(Some(envelope)) => {
                    tracing::debug!(driver = %driver.id, ?envelope.command, "radio task received command");
                    let result = self.handle_command(envelope.command).await;
                    if let Err(error) = &result {
                        tracing::debug!(driver = %driver.id, ?error, "radio task command failed");
                    }
                    let _ = envelope.result_tx.send(result);
                }
                Ok(None) => break,
                Err(_) => {
                    let _ = self
                        .process_kenwood_incoming(
                            Duration::from_millis(1),
                            UpdateSource::Native,
                            None,
                        )
                        .await?;
                    let _ = self
                        .process_icom_incoming(
                            Duration::from_millis(1),
                            UpdateSource::Native,
                            None,
                            None,
                        )
                        .await?;
                    self.run_poll_if_due().await?;
                }
            }
        }

        tracing::info!(driver = %driver.id, "radio task command channel closed");
        self.publish_patches(
            vec![StatePatch::Connection(crate::ConnectionState::Disconnected)],
            UpdateSource::Native,
        );

        tracing::info!(driver = %driver.id, "radio task run loop exiting");
        Ok(())
    }

    async fn run_icom_startup(&mut self) -> Result<()> {
        let Some(profile) = self.icom_profile else {
            return Ok(());
        };
        let Some(options) = self.icom_options else {
            return Ok(());
        };
        if self.transport.is_none() {
            return Ok(());
        }

        tracing::info!(
            driver = %profile.id(),
            startup_steps = profile.startup.len(),
            "running icom-civ startup sequence"
        );

        for step in profile.startup {
            match *step {
                icom_civ::StartupStep::Query(semantic) => {
                    let Some(encoded) = icom_civ::encode_query(options, semantic)? else {
                        tracing::trace!(driver = %profile.id(), semantic, "ICOM startup semantic skipped");
                        continue;
                    };
                    tracing::debug!(
                        driver = %profile.id(),
                        semantic,
                        frame_count = encoded.frames.len(),
                        expected = ?encoded.matcher,
                        "ICOM startup query step"
                    );
                    if let Err(error) = self
                        .send_icom_encoded(
                            profile,
                            encoded,
                            UpdateSource::Native,
                            STARTUP_RESPONSE_TIMEOUT,
                        )
                        .await
                    {
                        tracing::warn!(
                            driver = %profile.id(),
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

    async fn handle_command(&mut self, command: RadioCommand) -> Result<()> {
        let command_for_native = command.clone();
        let state_before = self.reducer.state().clone();

        let outcome = self.driver.handle_command(command, &state_before).await?;
        tracing::debug!(
            patch_count = outcome.patches.len(),
            source = ?outcome.source,
            "driver produced state patches"
        );
        self.publish_patches(outcome.patches, outcome.source);

        self.dispatch_native_command(command_for_native.clone(), &state_before)
            .await?;
        self.dispatch_icom_command(command_for_native, &state_before)
            .await?;

        Ok(())
    }

    async fn run_kenwood_startup(&mut self) -> Result<()> {
        let Some(profile) = self.kenwood_profile else {
            return Ok(());
        };
        if self.transport.is_none() {
            return Ok(());
        }

        tracing::info!(
            driver = %profile.id(),
            startup_steps = profile.startup.len(),
            "running kenwood-ascii startup sequence"
        );

        for step in profile.startup {
            match *step {
                StartupStep::AutoInfo(frame_text) => {
                    let encoded = EncodedCommand::new(
                        vec![AsciiFrame::new(frame_text)?],
                        ResponseMatcher::None,
                        Vec::new(),
                        CommandPriority::High,
                    );
                    tracing::debug!(driver = %profile.id(), step = step.label(), "startup auto-info step");
                    if let Err(error) = self
                        .send_kenwood_encoded(
                            profile,
                            encoded,
                            UpdateSource::Native,
                            STARTUP_RESPONSE_TIMEOUT,
                        )
                        .await
                    {
                        tracing::warn!(
                            driver = %profile.id(),
                            step = step.label(),
                            ?error,
                            "startup auto-info step failed; continuing"
                        );
                    }
                }
                StartupStep::Query(semantic) => {
                    let Some(encoded) = encode_kenwood_query(profile, semantic)? else {
                        tracing::trace!(driver = %profile.id(), semantic, "startup semantic skipped (no query frame)");
                        continue;
                    };
                    tracing::debug!(
                        driver = %profile.id(),
                        semantic,
                        frame_count = encoded.frames.len(),
                        expected = ?encoded.matcher,
                        "startup query step"
                    );
                    if let Err(error) = self
                        .send_kenwood_encoded(
                            profile,
                            encoded,
                            UpdateSource::Native,
                            STARTUP_RESPONSE_TIMEOUT,
                        )
                        .await
                    {
                        tracing::warn!(
                            driver = %profile.id(),
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

    async fn dispatch_native_command(
        &mut self,
        command: RadioCommand,
        state_before: &RadioState,
    ) -> Result<()> {
        let Some(profile) = self.kenwood_profile else {
            return Ok(());
        };
        if self.transport.is_none() {
            tracing::trace!(driver = %profile.id(), "no transport configured; skipping native command dispatch");
            return Ok(());
        }

        let Some(encoded) = encode_kenwood_command(profile, &command, state_before)? else {
            tracing::trace!(driver = %profile.id(), ?command, "command has no native transport encoding");
            return Ok(());
        };

        tracing::debug!(
            driver = %profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            "dispatching command over transport"
        );

        match self
            .send_kenwood_encoded(
                profile,
                encoded,
                UpdateSource::CommandResponse,
                COMMAND_RESPONSE_TIMEOUT,
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(error @ RadioError::Timeout { .. }) => {
                tracing::warn!(
                    driver = %profile.id(),
                    ?command,
                    ?error,
                    "Kenwood-ASCII set command timed out; querying current state instead"
                );
                self.recover_kenwood_timeout(profile, &command, state_before)
                    .await;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn recover_kenwood_timeout(
        &mut self,
        profile: &'static KenwoodAsciiProfile,
        command: &RadioCommand,
        state_before: &RadioState,
    ) {
        for semantic in kenwood_timeout_recovery_queries(profile, command, state_before) {
            let Some(encoded) = (match encode_kenwood_query(profile, semantic) {
                Ok(encoded) => encoded,
                Err(error) => {
                    tracing::warn!(
                        driver = %profile.id(),
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
                .send_kenwood_encoded(
                    profile,
                    encoded,
                    UpdateSource::CommandResponse,
                    COMMAND_RESPONSE_TIMEOUT,
                )
                .await
            {
                tracing::warn!(
                    driver = %profile.id(),
                    ?command,
                    semantic,
                    ?error,
                    "Kenwood timeout recovery query failed; continuing"
                );
            }
        }
    }

    async fn dispatch_icom_command(
        &mut self,
        command: RadioCommand,
        state_before: &RadioState,
    ) -> Result<()> {
        let Some(profile) = self.icom_profile else {
            return Ok(());
        };
        let Some(options) = self.icom_options else {
            return Ok(());
        };
        if self.transport.is_none() {
            tracing::trace!(driver = %profile.id(), "no transport configured; skipping ICOM native command dispatch");
            return Ok(());
        }

        let Some(encoded) = icom_civ::encode(profile, options, &command, state_before)? else {
            tracing::trace!(driver = %profile.id(), ?command, "command has no ICOM native transport encoding");
            return Ok(());
        };

        tracing::debug!(
            driver = %profile.id(),
            ?command,
            frame_count = encoded.frames.len(),
            expected = ?encoded.matcher,
            "dispatching ICOM command over transport"
        );

        self.send_icom_encoded(
            profile,
            encoded,
            UpdateSource::CommandResponse,
            COMMAND_RESPONSE_TIMEOUT,
        )
        .await
    }

    async fn send_kenwood_encoded(
        &mut self,
        profile: &'static KenwoodAsciiProfile,
        encoded: EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
    ) -> Result<()> {
        let frame_count = encoded.frames.len();
        for (index, frame) in encoded.frames.into_iter().enumerate() {
            let is_last = index + 1 == frame_count;

            tracing::debug!(
                driver = %profile.id(),
                tx_frame = frame.as_str(),
                priority = ?encoded.priority,
                "sending CAT frame"
            );

            {
                let transport = self.transport.as_mut().ok_or_else(|| {
                    RadioError::Transport(
                        "missing transport for native command dispatch".to_string(),
                    )
                })?;
                transport.write_all(frame.as_bytes()).await?;
                transport.flush().await?;
            }

            if is_last && matcher_expects_response(&encoded.matcher) {
                let matched = self
                    .process_kenwood_incoming(wait_timeout, default_source, Some(&encoded.matcher))
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

    async fn process_kenwood_incoming(
        &mut self,
        wait_timeout: Duration,
        default_source: UpdateSource,
        expected: Option<&ResponseMatcher>,
    ) -> Result<bool> {
        let Some(profile) = self.kenwood_profile else {
            return Ok(false);
        };
        if self.transport.is_none() {
            return Ok(false);
        }

        let deadline = Instant::now() + wait_timeout;
        let mut saw_frames = false;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }

            let remaining = deadline.saturating_duration_since(now);
            let mut buf = [0u8; 1024];
            let count = {
                let transport = self
                    .transport
                    .as_mut()
                    .ok_or_else(|| RadioError::Transport("transport disappeared".to_string()))?;
                match timeout(remaining, transport.read_some(&mut buf)).await {
                    Ok(read_result) => read_result?,
                    Err(_) => return Ok(false),
                }
            };

            if count == 0 {
                tracing::trace!(driver = %profile.id(), "transport read yielded EOF/empty");
                return Ok(false);
            }

            let mut matched_expected = false;
            let frames = match self.kenwood_frame_splitter.push(&buf[..count]) {
                Ok(frames) => frames,
                Err(error) => {
                    tracing::warn!(
                        driver = %profile.id(),
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
                        driver = %profile.id(),
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
                            driver = %profile.id(),
                            rx_frame = frame.as_str(),
                            expected = ?expected,
                            "received expected CAT response"
                        );
                        matched_expected = true;
                    }
                }

                match decode_kenwood_frame(profile, &frame, self.reducer.state()) {
                    Ok(Some(decoded)) => {
                        let source = decoded.source_hint.unwrap_or(default_source);
                        tracing::debug!(
                            driver = %profile.id(),
                            rx_frame = frame.as_str(),
                            source = ?source,
                            patch_count = decoded.patches.len(),
                            "decoded CAT frame into state patches"
                        );
                        self.publish_patches(decoded.patches, source);
                    }
                    Ok(None) => {
                        tracing::trace!(
                            driver = %profile.id(),
                            rx_frame = frame.as_str(),
                            "unhandled CAT frame"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            driver = %profile.id(),
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

    async fn send_icom_encoded(
        &mut self,
        profile: &'static IcomCivProfile,
        encoded: icom_civ::EncodedCommand,
        default_source: UpdateSource,
        wait_timeout: Duration,
    ) -> Result<()> {
        for frame in encoded.frames {
            tracing::debug!(
                driver = %profile.id(),
                tx_bytes = ?frame.as_bytes(),
                "sending ICOM CI-V frame"
            );

            {
                let transport = self.transport.as_mut().ok_or_else(|| {
                    RadioError::Transport(
                        "missing transport for ICOM native command dispatch".to_string(),
                    )
                })?;
                transport.write_all(frame.as_bytes()).await?;
                transport.flush().await?;
            }

            if encoded.matcher.expects_response() {
                match self
                    .process_icom_incoming(
                        wait_timeout,
                        default_source,
                        Some(&encoded.matcher),
                        Some(frame.as_bytes()),
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

    async fn process_icom_incoming(
        &mut self,
        wait_timeout: Duration,
        default_source: UpdateSource,
        expected: Option<&IcomResponseMatcher>,
        echo: Option<&[u8]>,
    ) -> Result<IcomWaitOutcome> {
        let Some(profile) = self.icom_profile else {
            return Ok(IcomWaitOutcome::Timeout);
        };
        if self.transport.is_none() {
            return Ok(IcomWaitOutcome::Timeout);
        }

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
            let count = {
                let transport = self
                    .transport
                    .as_mut()
                    .ok_or_else(|| RadioError::Transport("transport disappeared".to_string()))?;
                match timeout(remaining, transport.read_some(&mut buf)).await {
                    Ok(read_result) => read_result?,
                    Err(_) => {
                        return Ok(if expected.is_none() && saw_frames {
                            IcomWaitOutcome::Matched
                        } else {
                            IcomWaitOutcome::Timeout
                        })
                    }
                }
            };

            if count == 0 {
                tracing::trace!(driver = %profile.id(), "ICOM transport read yielded EOF/empty");
                return Ok(if expected.is_none() && saw_frames {
                    IcomWaitOutcome::Matched
                } else {
                    IcomWaitOutcome::Timeout
                });
            }

            let frames = match self.icom_frame_splitter.push(&buf[..count]) {
                Ok(frames) => frames,
                Err(error) => {
                    tracing::warn!(
                        driver = %profile.id(),
                        ?error,
                        "failed to split incoming ICOM CI-V bytes into frames; dropping chunk"
                    );
                    continue;
                }
            };

            for frame in frames {
                saw_frames = true;
                if echo.is_some_and(|sent| frame.is_echo_of(sent)) {
                    tracing::trace!(driver = %profile.id(), rx_bytes = ?frame.as_bytes(), "discarding ICOM self-echo frame");
                    continue;
                }

                if let Some(status) = icom_civ::ProtocolStatus::parse(&frame) {
                    tracing::debug!(
                        driver = %profile.id(),
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
                            driver = %profile.id(),
                            rx_bytes = ?frame.as_bytes(),
                            expected = ?expected,
                            "received expected ICOM CI-V response"
                        );
                        matched_expected = true;
                    }
                }

                self.decode_and_publish_icom_frame(profile, &frame, default_source);

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

    fn decode_and_publish_icom_frame(
        &mut self,
        profile: &'static IcomCivProfile,
        frame: &CivFrame,
        default_source: UpdateSource,
    ) {
        match icom_civ::decode(profile, frame, self.reducer.state()) {
            Ok(Some(decoded)) => {
                let source = decoded.source_hint.unwrap_or(default_source);
                tracing::debug!(
                    driver = %profile.id(),
                    rx_bytes = ?frame.as_bytes(),
                    source = ?source,
                    patch_count = decoded.patches.len(),
                    "decoded ICOM CI-V frame into state patches"
                );
                self.publish_patches(decoded.patches, source);
            }
            Ok(None) => {
                tracing::trace!(
                    driver = %profile.id(),
                    rx_bytes = ?frame.as_bytes(),
                    "unhandled ICOM CI-V frame"
                );
            }
            Err(error) => {
                tracing::warn!(
                    driver = %profile.id(),
                    rx_bytes = ?frame.as_bytes(),
                    ?error,
                    "failed to decode ICOM CI-V frame; continuing"
                );
            }
        }
    }

    async fn run_poll_if_due(&mut self) -> Result<()> {
        if self.transport.is_none() {
            return Ok(());
        }

        let now = Instant::now();
        let Some(next_poll_at) = self.next_poll_at else {
            self.schedule_next_poll();
            return Ok(());
        };

        if now < next_poll_at {
            return Ok(());
        }

        if let Some(profile) = self.kenwood_profile {
            if let Some(plan) = profile.poll {
                tracing::debug!(driver = %profile.id(), query_count = plan.queries.len(), "running poll plan");
                for semantic in plan.queries {
                    let Some(encoded) = encode_kenwood_query(profile, semantic)? else {
                        continue;
                    };

                    if let Err(error) = self
                        .send_kenwood_encoded(
                            profile,
                            encoded,
                            UpdateSource::Poll,
                            COMMAND_RESPONSE_TIMEOUT,
                        )
                        .await
                    {
                        tracing::debug!(driver = %profile.id(), semantic, ?error, "poll query failed");
                    }
                }
            }
        }

        if let Some(profile) = self.icom_profile {
            if let (Some(plan), Some(options)) = (profile.poll, self.icom_options) {
                tracing::debug!(driver = %profile.id(), query_count = plan.queries.len(), "running ICOM poll plan");
                for semantic in plan.queries {
                    let Some(encoded) = icom_civ::encode_query(options, semantic)? else {
                        continue;
                    };

                    if let Err(error) = self
                        .send_icom_encoded(
                            profile,
                            encoded,
                            UpdateSource::Poll,
                            COMMAND_RESPONSE_TIMEOUT,
                        )
                        .await
                    {
                        tracing::debug!(driver = %profile.id(), semantic, ?error, "ICOM poll query failed");
                    }
                }
            }
        }

        self.schedule_next_poll();
        Ok(())
    }

    fn schedule_next_poll(&mut self) {
        if let Some(profile) = self.kenwood_profile {
            if let Some(plan) = profile.poll {
                self.next_poll_at = Some(Instant::now() + plan.interval);
                return;
            }
        }
        if let Some(profile) = self.icom_profile {
            if profile.poll.is_some() {
                let interval = self
                    .icom_options
                    .map(|options| options.poll_interval)
                    .unwrap_or(IcomCivOptions::DEFAULT_POLL_INTERVAL);
                self.next_poll_at = Some(Instant::now() + interval);
                return;
            }
        }

        self.next_poll_at = None;
    }

    fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource) {
        tracing::trace!(patch_count = patches.len(), source = ?source, "publishing patches");
        let change_set = self.reducer.apply_patches(patches);
        if change_set.is_empty() {
            tracing::trace!(source = ?source, "no observable state change from patches");
            return;
        }

        tracing::debug!(
            source = ?source,
            changes = ?change_set.flags,
            fields = ?change_set.fields,
            "published state update"
        );

        let state = Arc::new(self.reducer.state().clone());
        let _ = self.state_tx.send(state.clone());
        let _ = self.update_tx.send(StateUpdate {
            changes: change_set.flags,
            fields: change_set.fields,
            source,
            state,
        });
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
        RadioCommand::SetReceiverMode { receiver, .. } => mode_queries_for_receiver(profile, *receiver),
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
        RadioCommand::SetRitXitOffset(_) => rit_offset_queries(profile, ReceiverPath::Main),
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

fn rit_enabled_query(profile: &'static KenwoodAsciiProfile, receiver: ReceiverPath) -> &'static str {
    match (profile.id(), receiver) {
        ("elecraft-k4", ReceiverPath::Sub) => "RT$",
        _ => "RT",
    }
}

fn rit_offset_queries(
    profile: &'static KenwoodAsciiProfile,
    _receiver: ReceiverPath,
) -> Vec<&'static str> {
    match profile.id() {
        "kenwood-ts890" | "kenwood-ts990" => vec!["RF"],
        "elecraft-k4" | "elecraft-k3" => match _receiver {
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

pub(crate) async fn send_command(
    command_tx: &mpsc::Sender<CommandEnvelope>,
    command: RadioCommand,
) -> Result<()> {
    let (result_tx, result_rx) = oneshot::channel();
    tracing::trace!(?command, "sending command envelope to radio task");
    command_tx
        .send(CommandEnvelope { command, result_tx })
        .await
        .map_err(|_| RadioError::TaskStopped)?;

    result_rx.await.map_err(|_| RadioError::CommandCanceled)?
}
