use std::sync::Arc;

use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::{timeout, Duration, Instant},
};

use crate::{
    driver::RadioDriver,
    error::{RadioError, Result},
    protocol::runtime::{native_protocol_for_driver, NativeProtocol, ProtocolContext},
    transport::BoxedCatTransport,
    update::{SharedRadioState, StateReducer, StateUpdate},
    RadioCommand, RadioState, StatePatch, UpdateSource,
};

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
    native_protocol: Option<Box<dyn NativeProtocol>>,
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
            native_protocol: None,
            driver_options,
            next_poll_at: None,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let driver = self.driver.descriptor();
        self.native_protocol = native_protocol_for_driver(driver.id, &self.driver_options)?;

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

        if self.native_protocol.is_some() && self.transport.is_some() {
            let protocol = self
                .native_protocol
                .as_ref()
                .map(|protocol| protocol.id())
                .unwrap();
            tracing::info!(driver = %driver.id, protocol, "starting native transport bootstrap");
            self.run_native_startup().await?;
            tracing::info!(driver = %driver.id, protocol, "native transport bootstrap complete");
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
                        .process_native_incoming(Duration::from_millis(1), UpdateSource::Native)
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

    async fn run_native_startup(&mut self) -> Result<()> {
        let Some(mut protocol) = self.native_protocol.take() else {
            return Ok(());
        };
        let Some(mut transport) = self.transport.take() else {
            self.native_protocol = Some(protocol);
            return Ok(());
        };

        let result = protocol.startup(&mut *transport, self).await;
        self.transport = Some(transport);
        self.native_protocol = Some(protocol);

        if result.is_ok() {
            self.schedule_next_poll();
        }

        result
    }

    async fn dispatch_native_command(
        &mut self,
        command: RadioCommand,
        state_before: &RadioState,
    ) -> Result<()> {
        let Some(mut protocol) = self.native_protocol.take() else {
            return Ok(());
        };
        let Some(mut transport) = self.transport.take() else {
            tracing::trace!(driver = %protocol.id(), "no transport configured; skipping native command dispatch");
            self.native_protocol = Some(protocol);
            return Ok(());
        };

        let result = protocol
            .dispatch_command(&mut *transport, command, state_before, self)
            .await;
        self.transport = Some(transport);
        self.native_protocol = Some(protocol);
        result
    }

    async fn process_native_incoming(
        &mut self,
        wait_timeout: Duration,
        default_source: UpdateSource,
    ) -> Result<bool> {
        let Some(mut protocol) = self.native_protocol.take() else {
            return Ok(false);
        };
        let Some(mut transport) = self.transport.take() else {
            self.native_protocol = Some(protocol);
            return Ok(false);
        };

        let result = protocol
            .process_incoming(&mut *transport, wait_timeout, default_source, self)
            .await;
        self.transport = Some(transport);
        self.native_protocol = Some(protocol);
        result
    }

    async fn run_poll_if_due(&mut self) -> Result<()> {
        if self.transport.is_none() || self.native_protocol.is_none() {
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

        let Some(mut protocol) = self.native_protocol.take() else {
            return Ok(());
        };
        let Some(mut transport) = self.transport.take() else {
            self.native_protocol = Some(protocol);
            return Ok(());
        };

        let result = protocol.poll(&mut *transport, self).await;
        self.transport = Some(transport);
        self.native_protocol = Some(protocol);
        self.schedule_next_poll();
        result
    }

    fn schedule_next_poll(&mut self) {
        self.next_poll_at = self
            .native_protocol
            .as_ref()
            .and_then(|protocol| protocol.poll_interval())
            .map(|interval| Instant::now() + interval);
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

impl ProtocolContext for RadioTask {
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
