use std::sync::Arc;

use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::{timeout, Duration, Instant},
};

use crate::{
    driver::RadioDriver,
    error::{RadioError, Result},
    protocol::kenwood_ascii::{
        self as kenwood_ascii, AsciiFrame, CommandPriority, EncodedCommand, FrameSplitter,
        KenwoodAsciiProfile, ResponseMatcher, StartupStep,
    },
    transport::BoxedCatTransport,
    update::{SharedRadioState, StateReducer, StateUpdate},
    RadioCommand, RadioState, StatePatch, UpdateSource,
};

const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_millis(900);
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_millis(1200);
const COMMAND_LOOP_IDLE_TICK: Duration = Duration::from_millis(50);

pub(crate) struct CommandEnvelope {
    pub command: RadioCommand,
    pub result_tx: oneshot::Sender<Result<()>>,
}

pub struct RadioTask {
    driver: Box<dyn RadioDriver>,
    reducer: StateReducer,
    command_rx: mpsc::Receiver<CommandEnvelope>,
    state_tx: watch::Sender<SharedRadioState>,
    update_tx: broadcast::Sender<StateUpdate>,
    transport: Option<BoxedCatTransport>,
    frame_splitter: FrameSplitter,
    kenwood_profile: Option<&'static KenwoodAsciiProfile>,
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
    ) -> Self {
        Self {
            driver,
            reducer: StateReducer::new(initial_state),
            command_rx,
            state_tx,
            update_tx,
            transport,
            frame_splitter: FrameSplitter::new(),
            kenwood_profile: None,
            next_poll_at: None,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let driver = self.driver.descriptor();
        self.kenwood_profile = kenwood_ascii::profile_by_id(driver.id);

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

        self.dispatch_native_command(command_for_native, &state_before)
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

        self.send_kenwood_encoded(
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
            let frames = match self.frame_splitter.push(&buf[..count]) {
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

    async fn run_poll_if_due(&mut self) -> Result<()> {
        let Some(profile) = self.kenwood_profile else {
            return Ok(());
        };
        let Some(plan) = profile.poll else {
            return Ok(());
        };
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
