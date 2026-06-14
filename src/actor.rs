use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::{
    driver::RadioDriver,
    error::{RadioError, Result},
    transport::BoxedCatTransport,
    update::{SharedRadioState, StateReducer, StateUpdate},
    RadioCommand, StatePatch, UpdateSource,
};

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
    _transport: Option<BoxedCatTransport>,
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
            _transport: transport,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        match self.driver.start().await {
            Ok(patches) => self.publish_patches(patches, UpdateSource::Native),
            Err(error) => {
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

        while let Some(envelope) = self.command_rx.recv().await {
            let result = self.handle_command(envelope.command).await;
            let _ = envelope.result_tx.send(result);
        }

        self.publish_patches(
            vec![StatePatch::Connection(crate::ConnectionState::Disconnected)],
            UpdateSource::Native,
        );

        Ok(())
    }

    async fn handle_command(&mut self, command: RadioCommand) -> Result<()> {
        let outcome = self
            .driver
            .handle_command(command, self.reducer.state())
            .await?;
        self.publish_patches(outcome.patches, outcome.source);
        Ok(())
    }

    fn publish_patches(&mut self, patches: Vec<StatePatch>, source: UpdateSource) {
        let change_set = self.reducer.apply_patches(patches);
        if change_set.is_empty() {
            return;
        }

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

pub(crate) async fn send_command(
    command_tx: &mpsc::Sender<CommandEnvelope>,
    command: RadioCommand,
) -> Result<()> {
    let (result_tx, result_rx) = oneshot::channel();
    command_tx
        .send(CommandEnvelope { command, result_tx })
        .await
        .map_err(|_| RadioError::TaskStopped)?;

    result_rx.await.map_err(|_| RadioError::CommandCanceled)?
}
