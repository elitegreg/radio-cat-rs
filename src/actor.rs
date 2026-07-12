use std::{collections::VecDeque, sync::Arc};

use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::{timeout, Duration, Instant},
};

use crate::{
    driver::{CommandCompletion, RadioSession, StateSink},
    error::{RadioError, Result},
    keyer_emulation,
    transport::BoxedCatTransport,
    update::{SharedRadioState, StateReducer, StateUpdate},
    Capability, ConnectionState, RadioCommand, RadioState, StatePatch, UpdateSource,
};

const COMMAND_LOOP_IDLE_TICK: Duration = Duration::from_millis(50);
const SESSION_STARTUP_DEADLINE: Duration = Duration::from_secs(15);
const SESSION_COMMAND_DEADLINE: Duration = Duration::from_secs(3);
const SESSION_POLL_DEADLINE: Duration = Duration::from_secs(1);
const MAX_URGENT_BURST: u8 = 4;

pub(crate) struct CommandEnvelope {
    pub command: RadioCommand,
    pub result_tx: oneshot::Sender<Result<()>>,
}

pub struct RadioTask {
    session: Option<Box<dyn RadioSession>>,
    reducer: StateReducer,
    command_rx: mpsc::Receiver<CommandEnvelope>,
    urgent_commands: VecDeque<CommandEnvelope>,
    normal_commands: VecDeque<CommandEnvelope>,
    shutdown_rx: watch::Receiver<bool>,
    state_tx: watch::Sender<SharedRadioState>,
    update_tx: broadcast::Sender<StateUpdate>,
    transport: Option<BoxedCatTransport>,
    next_poll_at: Option<Instant>,
    emulated_keyer_done_at: Option<Instant>,
    started: bool,
    urgent_burst: u8,
}

impl RadioTask {
    pub(crate) fn new(
        session: Box<dyn RadioSession>,
        initial_state: crate::RadioState,
        command_rx: mpsc::Receiver<CommandEnvelope>,
        shutdown_rx: watch::Receiver<bool>,
        state_tx: watch::Sender<SharedRadioState>,
        update_tx: broadcast::Sender<StateUpdate>,
        transport: Option<BoxedCatTransport>,
    ) -> Self {
        Self {
            session: Some(session),
            reducer: StateReducer::new(initial_state),
            command_rx,
            urgent_commands: VecDeque::new(),
            normal_commands: VecDeque::new(),
            shutdown_rx,
            state_tx,
            update_tx,
            transport,
            next_poll_at: None,
            emulated_keyer_done_at: None,
            started: false,
            urgent_burst: 0,
        }
    }

    /// Complete mandatory driver startup before accepting commands.
    pub async fn start(&mut self) -> Result<()> {
        if self.started {
            return Ok(());
        }

        let driver = self.session().descriptor();
        match self.run_session_startup().await {
            Ok(()) => {
                tracing::info!(driver = %driver.id, "session startup complete");
                self.started = true;
                Ok(())
            }
            Err(error) => {
                tracing::error!(driver = %driver.id, ?error, "driver startup failed");
                self.publish_terminal_error(&error);
                Err(error)
            }
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let driver = self.session().descriptor();
        tracing::info!(driver = %driver.id, "radio task run loop starting");
        self.start().await?;

        tracing::info!(driver = %driver.id, "radio task command loop started");
        loop {
            self.finish_emulated_keyer_if_due();
            if *self.shutdown_rx.borrow() {
                tracing::info!(driver = %driver.id, "radio task shutdown requested");
                break;
            }
            self.drain_commands();
            if let Some(envelope) = self.next_command() {
                self.complete_command(envelope).await;
                continue;
            }
            if self.poll_due() {
                if let Err(error) = self.run_poll_if_due().await {
                    if matches!(error, RadioError::Transport(_)) {
                        self.publish_terminal_error(&error);
                        return Err(error);
                    }
                    tracing::warn!(driver = %driver.id, ?error, "native poll failed");
                }
                continue;
            }
            let command_wait_timeout = self.command_wait_timeout();
            tokio::select! {
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || *self.shutdown_rx.borrow() {
                        tracing::info!(driver = %driver.id, "radio task shutdown requested");
                        break;
                    }
                }
                received = timeout(command_wait_timeout, self.command_rx.recv()) => match received {
                Ok(Some(envelope)) => self.enqueue_command(envelope),
                Ok(None) => break,
                Err(_) => {
                    self.finish_emulated_keyer_if_due();

                    if let Err(error) = self
                        .process_session_incoming(Duration::from_millis(1), UpdateSource::Native)
                        .await
                    {
                        if matches!(error, RadioError::Transport(_)) {
                            tracing::error!(driver = %driver.id, ?error, "native incoming processing failed");
                            self.publish_terminal_error(&error);
                            return Err(error);
                        }
                        tracing::warn!(driver = %driver.id, ?error, "native incoming processing failed");
                    }

                }
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

    fn enqueue_command(&mut self, envelope: CommandEnvelope) {
        if is_urgent(&envelope.command) {
            self.urgent_commands.push_back(envelope);
        } else {
            self.normal_commands.push_back(envelope);
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(envelope) = self.command_rx.try_recv() {
            self.enqueue_command(envelope);
        }
    }

    fn next_command(&mut self) -> Option<CommandEnvelope> {
        // Give normal and background work a turn after a bounded urgent burst.
        // A queued safety command is delayed by at most one bounded poll item.
        if !self.urgent_commands.is_empty()
            && self.normal_commands.is_empty()
            && self.urgent_burst >= MAX_URGENT_BURST
            && self.poll_due()
        {
            self.urgent_burst = 0;
            return None;
        }
        if !self.urgent_commands.is_empty()
            && (self.normal_commands.is_empty() || self.urgent_burst < MAX_URGENT_BURST)
        {
            self.urgent_burst += 1;
            return self.urgent_commands.pop_front();
        }
        if let Some(envelope) = self.normal_commands.pop_front() {
            self.urgent_burst = 0;
            return Some(envelope);
        }
        self.urgent_burst = 0;
        self.urgent_commands.pop_front()
    }

    async fn complete_command(&mut self, envelope: CommandEnvelope) {
        tracing::debug!(?envelope.command, "radio task dispatching command");
        let result = self.handle_command(envelope.command).await;
        if let Err(error) = &result {
            tracing::debug!(?error, "radio task command failed");
        }
        let _ = envelope.result_tx.send(result);
    }

    fn publish_terminal_error(&mut self, error: &RadioError) {
        self.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Error {
                message: error.to_string(),
            })],
            UpdateSource::Native,
        );
        self.publish_patches(
            vec![StatePatch::Connection(ConnectionState::Disconnected)],
            UpdateSource::Native,
        );
    }

    async fn handle_command(&mut self, command: RadioCommand) -> Result<()> {
        if matches!(command, RadioCommand::Refresh) {
            self.run_session_refresh().await?;
            return Ok(());
        }

        let command_for_emulation = command.clone();
        let state_before = self.reducer.state().clone();

        let completion = self.execute_session_command(command, &state_before).await?;
        tracing::debug!(?completion, "radio task command completed");
        self.apply_emulated_keyer_command(&command_for_emulation);

        Ok(())
    }

    fn command_wait_timeout(&self) -> Duration {
        let mut wait_timeout = COMMAND_LOOP_IDLE_TICK;

        if let Some(deadline) = self.emulated_keyer_done_at {
            wait_timeout = wait_timeout.min(deadline.saturating_duration_since(Instant::now()));
        }

        wait_timeout
    }

    fn poll_due(&self) -> bool {
        self.next_poll_at
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn apply_emulated_keyer_command(&mut self, command: &RadioCommand) {
        if self
            .session()
            .capabilities()
            .keyer
            .map(|keyer| keyer.sending)
            != Some(Capability::Emulated)
        {
            return;
        }

        match command {
            RadioCommand::SendCw(text) => {
                let Some(wpm) = self
                    .reducer
                    .state()
                    .keyer
                    .as_ref()
                    .and_then(|keyer| keyer.speed_wpm)
                else {
                    tracing::debug!(
                        ?command,
                        "skipping emulated keyer sending update because WPM is unknown"
                    );
                    return;
                };

                let Some(duration) = keyer_emulation::estimate_send_time(text, wpm) else {
                    tracing::debug!(?command, wpm, "skipping emulated keyer sending update because send time could not be estimated");
                    return;
                };

                let now = Instant::now();
                let start_at = self
                    .emulated_keyer_done_at
                    .filter(|deadline| *deadline > now)
                    .unwrap_or(now);
                self.emulated_keyer_done_at = Some(start_at + duration);
                self.publish_patches(vec![StatePatch::KeyerSending(true)], UpdateSource::Emulated);
            }
            RadioCommand::StopCw => {
                if self.emulated_keyer_done_at.take().is_some() {
                    self.publish_patches(
                        vec![StatePatch::KeyerSending(false)],
                        UpdateSource::Emulated,
                    );
                }
            }
            _ => {}
        }
    }

    fn finish_emulated_keyer_if_due(&mut self) {
        let Some(deadline) = self.emulated_keyer_done_at else {
            return;
        };

        if Instant::now() < deadline {
            return;
        }

        self.emulated_keyer_done_at = None;
        self.publish_patches(
            vec![StatePatch::KeyerSending(false)],
            UpdateSource::Emulated,
        );
    }

    fn session(&self) -> &dyn RadioSession {
        self.session
            .as_deref()
            .expect("session is present outside calls")
    }

    async fn run_session_startup(&mut self) -> Result<()> {
        let mut session = self.session.take().expect("session is present");
        let mut transport = self.transport.take();
        let result = timeout(SESSION_STARTUP_DEADLINE, async {
            match transport.as_mut() {
                Some(transport) => session.startup(Some(transport.as_mut()), self).await,
                None => session.startup(None, self).await,
            }
        })
        .await
        .unwrap_or(Err(RadioError::Timeout {
            command: "session-startup",
        }));
        self.transport = transport;
        self.session = Some(session);

        if result.is_ok() {
            self.schedule_next_poll();
        }

        result
    }

    async fn run_session_refresh(&mut self) -> Result<()> {
        let mut session = self.session.take().expect("session is present");
        let mut transport = self.transport.take();
        let result = timeout(SESSION_COMMAND_DEADLINE, async {
            match transport.as_mut() {
                Some(transport) => session.refresh(Some(transport.as_mut()), self).await,
                None => session.refresh(None, self).await,
            }
        })
        .await
        .unwrap_or(Err(RadioError::Timeout {
            command: "session-refresh",
        }));
        self.transport = transport;
        self.session = Some(session);
        result
    }

    async fn execute_session_command(
        &mut self,
        command: RadioCommand,
        state_before: &RadioState,
    ) -> Result<CommandCompletion> {
        let mut session = self.session.take().expect("session is present");
        let mut transport = self.transport.take();
        let result = timeout(SESSION_COMMAND_DEADLINE, async {
            match transport.as_mut() {
                Some(transport) => {
                    session
                        .execute(Some(transport.as_mut()), command, state_before, self)
                        .await
                }
                None => session.execute(None, command, state_before, self).await,
            }
        })
        .await
        .unwrap_or(Err(RadioError::Timeout {
            command: "session-command",
        }));
        self.transport = transport;
        self.session = Some(session);
        result
    }

    async fn process_session_incoming(
        &mut self,
        wait_timeout: Duration,
        default_source: UpdateSource,
    ) -> Result<bool> {
        let mut session = self.session.take().expect("session is present");
        let mut transport = self.transport.take();
        let result = timeout(SESSION_POLL_DEADLINE, async {
            match transport.as_mut() {
                Some(transport) => {
                    session
                        .process_incoming(
                            Some(transport.as_mut()),
                            wait_timeout,
                            default_source,
                            self,
                        )
                        .await
                }
                None => {
                    session
                        .process_incoming(None, wait_timeout, default_source, self)
                        .await
                }
            }
        })
        .await
        .unwrap_or(Err(RadioError::Timeout {
            command: "session-read",
        }));
        self.transport = transport;
        self.session = Some(session);
        result
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

        let mut session = self.session.take().expect("session is present");
        let mut transport = self.transport.take();
        let result = timeout(SESSION_POLL_DEADLINE, async {
            match transport.as_mut() {
                Some(transport) => session.poll_one(Some(transport.as_mut()), self).await,
                None => session.poll_one(None, self).await,
            }
        })
        .await
        .unwrap_or(Err(RadioError::Timeout {
            command: "session-poll",
        }));
        self.transport = transport;
        self.session = Some(session);
        match result {
            Ok(true) => self.schedule_next_poll(),
            Ok(false) => self.next_poll_at = Some(Instant::now()),
            Err(_) => self.schedule_next_poll(),
        }
        result.map(|_| ())
    }

    fn schedule_next_poll(&mut self) {
        self.next_poll_at = self
            .session
            .as_ref()
            .and_then(|session| session.poll_interval())
            .map(|interval| Instant::now() + interval);
    }

    fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource) {
        tracing::trace!(patch_count = patches.len(), source = ?source, "publishing patches");

        if patches
            .iter()
            .any(|patch| matches!(patch, StatePatch::KeyerSending(false)))
            || patches.iter().any(|patch| {
                matches!(
                    patch,
                    StatePatch::Connection(ConnectionState::Disconnected)
                        | StatePatch::Connection(ConnectionState::Error { .. })
                )
            })
        {
            self.emulated_keyer_done_at = None;
        }

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

fn is_urgent(command: &RadioCommand) -> bool {
    matches!(
        command,
        RadioCommand::SetPtt(false) | RadioCommand::SetDataPtt(false) | RadioCommand::StopCw
    )
}

impl StateSink for RadioTask {
    fn state(&self) -> &RadioState {
        self.reducer.state()
    }

    fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource) {
        RadioTask::publish_patches(self, patches, source);
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::{sync::Notify, task::JoinHandle};

    use crate::{
        Capability, DriverDescriptor, KeyerCapabilities, KeyerState, RadioCapabilities,
        TransportRequirement,
    };

    #[derive(Clone)]
    struct TestSession {
        initial_state: RadioState,
        capabilities: RadioCapabilities,
    }

    struct PausingPollSession {
        poll_started: Arc<Notify>,
        urgent_dispatched: Arc<AtomicBool>,
    }

    #[async_trait]
    impl RadioSession for PausingPollSession {
        fn descriptor(&self) -> DriverDescriptor {
            TestSession::new(None).descriptor()
        }
        fn capabilities(&self) -> RadioCapabilities {
            RadioCapabilities::dummy_all()
        }
        fn initial_state(&self) -> RadioState {
            RadioState::default()
        }
        fn poll_interval(&self) -> Option<Duration> {
            Some(Duration::from_millis(1))
        }
        async fn startup(
            &mut self,
            _: Option<&mut dyn crate::CatTransport>,
            _: &mut dyn StateSink,
        ) -> Result<()> {
            Ok(())
        }
        async fn refresh(
            &mut self,
            _: Option<&mut dyn crate::CatTransport>,
            _: &mut dyn StateSink,
        ) -> Result<()> {
            Ok(())
        }
        async fn execute(
            &mut self,
            _: Option<&mut dyn crate::CatTransport>,
            command: RadioCommand,
            _: &RadioState,
            _: &mut dyn StateSink,
        ) -> Result<CommandCompletion> {
            if command == RadioCommand::SetPtt(false) {
                self.urgent_dispatched.store(true, Ordering::SeqCst);
            }
            Ok(CommandCompletion::Accepted)
        }
        async fn process_incoming(
            &mut self,
            _: Option<&mut dyn crate::CatTransport>,
            _: Duration,
            _: UpdateSource,
            _: &mut dyn StateSink,
        ) -> Result<bool> {
            Ok(false)
        }
        async fn poll_one(
            &mut self,
            _: Option<&mut dyn crate::CatTransport>,
            _: &mut dyn StateSink,
        ) -> Result<bool> {
            self.poll_started.notify_waiters();
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(false)
        }
    }

    struct NoopTransport;

    #[async_trait]
    impl crate::CatTransport for NoopTransport {
        async fn write_all(&mut self, _: &[u8]) -> Result<()> {
            Ok(())
        }
        async fn read_some(&mut self, _: &mut [u8]) -> Result<usize> {
            Ok(0)
        }
        async fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl TestSession {
        fn new(speed_wpm: Option<u8>) -> Self {
            let mut capabilities = RadioCapabilities::dummy_all();
            capabilities.keyer = Some(KeyerCapabilities::new(
                Capability::ReadWrite,
                Capability::Emulated,
                Capability::WriteOnly,
                Capability::WriteOnly,
            ));

            let initial_state = RadioState {
                connection: ConnectionState::Connecting,
                keyer: Some(KeyerState {
                    speed_wpm,
                    sending: None,
                }),
                ..RadioState::default()
            };

            Self {
                initial_state,
                capabilities,
            }
        }
    }

    #[async_trait]
    impl RadioSession for TestSession {
        fn descriptor(&self) -> DriverDescriptor {
            DriverDescriptor {
                id: "test-emulated-keyer",
                display_name: "Test Emulated Keyer",
                description: "test",
                transport_requirement: TransportRequirement::None,
            }
        }

        fn capabilities(&self) -> RadioCapabilities {
            self.capabilities
        }

        fn initial_state(&self) -> RadioState {
            self.initial_state.clone()
        }

        fn poll_interval(&self) -> Option<Duration> {
            None
        }

        async fn startup(
            &mut self,
            _transport: Option<&mut dyn crate::CatTransport>,
            sink: &mut dyn StateSink,
        ) -> Result<()> {
            sink.publish_patches(
                vec![StatePatch::Connection(ConnectionState::Ready)],
                UpdateSource::Native,
            );
            Ok(())
        }

        async fn refresh(
            &mut self,
            _transport: Option<&mut dyn crate::CatTransport>,
            _sink: &mut dyn StateSink,
        ) -> Result<()> {
            Ok(())
        }

        async fn execute(
            &mut self,
            _transport: Option<&mut dyn crate::CatTransport>,
            _command: RadioCommand,
            _current_state: &RadioState,
            _sink: &mut dyn StateSink,
        ) -> Result<CommandCompletion> {
            Ok(CommandCompletion::Accepted)
        }

        async fn process_incoming(
            &mut self,
            _transport: Option<&mut dyn crate::CatTransport>,
            _wait_timeout: Duration,
            _default_source: UpdateSource,
            _sink: &mut dyn StateSink,
        ) -> Result<bool> {
            Ok(false)
        }

        async fn poll_one(
            &mut self,
            _transport: Option<&mut dyn crate::CatTransport>,
            _sink: &mut dyn StateSink,
        ) -> Result<bool> {
            Ok(true)
        }
    }

    async fn spawn_test_radio(
        speed_wpm: Option<u8>,
    ) -> (
        mpsc::Sender<CommandEnvelope>,
        watch::Receiver<SharedRadioState>,
        JoinHandle<Result<()>>,
    ) {
        let session = TestSession::new(speed_wpm);
        let initial_state = session.initial_state();
        let (command_tx, command_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (state_tx, state_rx) = watch::channel(Arc::new(initial_state.clone()));
        let (update_tx, _) = broadcast::channel(8);
        let task = RadioTask::new(
            Box::new(session),
            initial_state,
            command_rx,
            shutdown_rx,
            state_tx,
            update_tx,
            None,
        );

        let handle = tokio::spawn(async move {
            let _shutdown_tx = shutdown_tx;
            task.run().await
        });
        (command_tx, state_rx, handle)
    }

    #[tokio::test(start_paused = true)]
    async fn urgent_ptt_release_dispatches_within_one_nonresponsive_poll_item() {
        let poll_started = Arc::new(Notify::new());
        let urgent_dispatched = Arc::new(AtomicBool::new(false));
        let session = PausingPollSession {
            poll_started: poll_started.clone(),
            urgent_dispatched: urgent_dispatched.clone(),
        };
        let initial_state = session.initial_state();
        let (command_tx, command_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (state_tx, _) = watch::channel(Arc::new(initial_state.clone()));
        let (update_tx, _) = broadcast::channel(8);
        let task = RadioTask::new(
            Box::new(session),
            initial_state,
            command_rx,
            shutdown_rx,
            state_tx,
            update_tx,
            Some(Box::new(NoopTransport)),
        );
        let handle = tokio::spawn(task.run());

        let waiting_for_poll = poll_started.notified();
        tokio::time::advance(Duration::from_millis(50)).await;
        waiting_for_poll.await;

        let (result_tx, result_rx) = oneshot::channel();
        command_tx
            .send(CommandEnvelope {
                command: RadioCommand::SetPtt(false),
                result_tx,
            })
            .await
            .unwrap();
        tokio::time::advance(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;
        assert!(result_rx.await.unwrap().is_ok());
        assert!(urgent_dispatched.load(Ordering::SeqCst));

        shutdown_tx.send(true).unwrap();
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn emulates_keyer_sending_when_wpm_is_known() {
        let (command_tx, mut state_rx, handle) = spawn_test_radio(Some(20)).await;
        state_rx.changed().await.unwrap();

        send_command(&command_tx, RadioCommand::SendCw("E".to_string()))
            .await
            .unwrap();
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(true)
        );

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(false)
        );

        drop(command_tx);
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn does_not_emulate_keyer_sending_when_wpm_is_unknown() {
        let (command_tx, mut state_rx, handle) = spawn_test_radio(None).await;
        state_rx.changed().await.unwrap();

        send_command(&command_tx, RadioCommand::SendCw("E".to_string()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;

        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            None
        );

        drop(command_tx);
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn does_not_emulate_keyer_sending_when_estimate_is_unavailable() {
        let (command_tx, mut state_rx, handle) = spawn_test_radio(Some(4)).await;
        state_rx.changed().await.unwrap();

        send_command(&command_tx, RadioCommand::SendCw("E".to_string()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;

        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            None
        );

        drop(command_tx);
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stop_cw_cancels_emulated_sending() {
        let (command_tx, mut state_rx, handle) = spawn_test_radio(Some(20)).await;
        state_rx.changed().await.unwrap();

        send_command(&command_tx, RadioCommand::SendCw("TEST TEST".to_string()))
            .await
            .unwrap();
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(true)
        );

        send_command(&command_tx, RadioCommand::StopCw)
            .await
            .unwrap();
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(false)
        );

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(false)
        );

        drop(command_tx);
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn additional_send_extends_emulated_deadline() {
        let (command_tx, mut state_rx, handle) = spawn_test_radio(Some(20)).await;
        state_rx.changed().await.unwrap();

        send_command(&command_tx, RadioCommand::SendCw("E".to_string()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
        send_command(&command_tx, RadioCommand::SendCw("E".to_string()))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(true)
        );

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            state_rx
                .borrow()
                .keyer
                .as_ref()
                .and_then(|keyer| keyer.sending),
            Some(false)
        );

        drop(command_tx);
        handle.await.unwrap().unwrap();
    }
}
