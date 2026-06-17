use std::collections::VecDeque;
use std::time::Duration;

use crate::{error::RadioError, Result};

use super::{AsciiFrame, ProtocolErrorFrame};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommandPriority {
    Background,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseMatcher {
    None,
    Exact(&'static str),
    Prefix(&'static str),
    OneOf(&'static [&'static str]),
}

impl ResponseMatcher {
    fn matches(&self, frame: &AsciiFrame) -> bool {
        match self {
            Self::None => false,
            Self::Exact(expected) => frame.as_str() == *expected,
            Self::Prefix(prefix) => frame.as_str().starts_with(prefix),
            Self::OneOf(prefixes) => prefixes
                .iter()
                .any(|prefix| frame.as_str().starts_with(prefix)),
        }
    }

    fn expects_response(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingStep {
    pub frame: AsciiFrame,
    pub expected: ResponseMatcher,
    pub priority: CommandPriority,
    pub timeout: Duration,
    pub retries: u8,
}

impl OutgoingStep {
    pub fn new(
        frame: AsciiFrame,
        expected: ResponseMatcher,
        priority: CommandPriority,
        timeout: Duration,
        retries: u8,
    ) -> Self {
        Self {
            frame,
            expected,
            priority,
            timeout,
            retries,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingTransaction {
    pub command: &'static str,
    pub steps: VecDeque<OutgoingStep>,
}

impl OutgoingTransaction {
    pub fn new(command: &'static str, steps: impl IntoIterator<Item = OutgoingStep>) -> Self {
        Self {
            command,
            steps: steps.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionDispatch {
    pub command: &'static str,
    pub frame: AsciiFrame,
    pub priority: CommandPriority,
    pub timeout: Duration,
    pub waits_for_response: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionEvent {
    Idle,
    Dispatched(TransactionDispatch),
    Completed {
        command: &'static str,
        response: AsciiFrame,
    },
    CompletedWithoutResponse {
        command: &'static str,
    },
    Retrying(TransactionDispatch),
    Unsolicited(AsciiFrame),
}

#[derive(Debug, Clone)]
struct ActiveStep {
    command: &'static str,
    frame: AsciiFrame,
    expected: ResponseMatcher,
    priority: CommandPriority,
    timeout: Duration,
    retries_remaining: u8,
}

impl ActiveStep {
    fn dispatch(&self) -> TransactionDispatch {
        TransactionDispatch {
            command: self.command,
            frame: self.frame.clone(),
            priority: self.priority,
            timeout: self.timeout,
            waits_for_response: self.expected.expects_response(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct TransactionEngine {
    queue: VecDeque<OutgoingTransaction>,
    active_transaction: Option<OutgoingTransaction>,
    waiting_on: Option<ActiveStep>,
    pending_completion: Option<&'static str>,
}

impl TransactionEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, transaction: OutgoingTransaction) {
        tracing::debug!(
            command = transaction.command,
            step_count = transaction.steps.len(),
            "queued CAT transaction"
        );
        self.queue.push_back(transaction);
    }

    pub fn next_dispatch(&mut self) -> Option<TransactionEvent> {
        if self.waiting_on.is_some() {
            return None;
        }

        if let Some(command) = self.pending_completion.take() {
            tracing::debug!(command, "transaction completed without response");
            return Some(TransactionEvent::CompletedWithoutResponse { command });
        }

        loop {
            if self.active_transaction.is_none() {
                self.active_transaction = self.queue.pop_front();
            }

            let active = self.active_transaction.as_mut()?;
            let step = match active.steps.pop_front() {
                Some(step) => step,
                None => {
                    let command = active.command;
                    self.active_transaction = None;
                    tracing::debug!(command, "transaction completed without response");
                    return Some(TransactionEvent::CompletedWithoutResponse { command });
                }
            };

            tracing::debug!(
                command = active.command,
                tx_frame = step.frame.as_str(),
                expected = ?step.expected,
                priority = ?step.priority,
                timeout_ms = step.timeout.as_millis(),
                retries = step.retries,
                "dispatching CAT frame"
            );

            let dispatch = TransactionDispatch {
                command: active.command,
                frame: step.frame.clone(),
                priority: step.priority,
                timeout: step.timeout,
                waits_for_response: step.expected.expects_response(),
            };

            if step.expected.expects_response() {
                self.waiting_on = Some(ActiveStep {
                    command: active.command,
                    frame: step.frame,
                    expected: step.expected,
                    priority: step.priority,
                    timeout: step.timeout,
                    retries_remaining: step.retries,
                });
            } else if active.steps.is_empty() {
                self.pending_completion = Some(active.command);
                self.active_transaction = None;
            }

            return Some(TransactionEvent::Dispatched(dispatch));
        }
    }

    pub fn receive_frame(&mut self, frame: AsciiFrame) -> Result<TransactionEvent> {
        if let Some(protocol_error) = ProtocolErrorFrame::parse(&frame) {
            tracing::debug!(
                frame = frame.as_str(),
                ?protocol_error,
                "received protocol error frame"
            );
            return self.handle_protocol_error(protocol_error);
        }

        let waiting_on = match self.waiting_on.take() {
            Some(waiting_on) => waiting_on,
            None => {
                tracing::trace!(rx_frame = frame.as_str(), "received unsolicited CAT frame");
                return Ok(TransactionEvent::Unsolicited(frame));
            }
        };

        if waiting_on.expected.matches(&frame) {
            let command = waiting_on.command;
            tracing::debug!(
                command,
                rx_frame = frame.as_str(),
                expected = ?waiting_on.expected,
                "received expected CAT response"
            );
            if self
                .active_transaction
                .as_ref()
                .is_some_and(|transaction| transaction.steps.is_empty())
            {
                self.active_transaction = None;
            }

            Ok(TransactionEvent::Completed {
                command,
                response: frame,
            })
        } else {
            tracing::trace!(
                command = waiting_on.command,
                rx_frame = frame.as_str(),
                expected = ?waiting_on.expected,
                "received non-matching CAT frame while waiting for response"
            );
            self.waiting_on = Some(waiting_on);
            Ok(TransactionEvent::Unsolicited(frame))
        }
    }

    pub fn on_timeout(&mut self) -> Result<TransactionEvent> {
        let waiting_on = self.waiting_on.take().ok_or(RadioError::Timeout {
            command: "transaction",
        })?;

        if waiting_on.retries_remaining == 0 {
            tracing::debug!(
                command = waiting_on.command,
                "transaction timed out without retries remaining"
            );
            return Err(RadioError::Timeout {
                command: waiting_on.command,
            });
        }

        let mut retry = waiting_on;
        retry.retries_remaining -= 1;
        let dispatch = retry.dispatch();
        tracing::debug!(
            command = dispatch.command,
            tx_frame = dispatch.frame.as_str(),
            retries_remaining = retry.retries_remaining,
            "retrying CAT frame after timeout"
        );
        self.waiting_on = Some(retry);

        Ok(TransactionEvent::Retrying(dispatch))
    }

    fn handle_protocol_error(
        &mut self,
        protocol_error: ProtocolErrorFrame<'_>,
    ) -> Result<TransactionEvent> {
        match protocol_error {
            ProtocolErrorFrame::Busy => {
                let waiting_on = self.waiting_on.take().ok_or(RadioError::ProtocolBusy)?;
                if waiting_on.retries_remaining == 0 {
                    tracing::debug!(
                        command = waiting_on.command,
                        "radio reported busy and retries exhausted"
                    );
                    return Err(RadioError::ProtocolBusy);
                }

                let mut retry = waiting_on;
                retry.retries_remaining -= 1;
                let dispatch = retry.dispatch();
                tracing::debug!(
                    command = dispatch.command,
                    tx_frame = dispatch.frame.as_str(),
                    retries_remaining = retry.retries_remaining,
                    "radio busy; retrying CAT frame"
                );
                self.waiting_on = Some(retry);
                Ok(TransactionEvent::Retrying(dispatch))
            }
            ProtocolErrorFrame::Communication => {
                self.waiting_on = None;
                tracing::debug!("radio reported CAT communication error");
                Err(RadioError::ProtocolCommunication)
            }
            ProtocolErrorFrame::Syntax { command } => {
                self.waiting_on = None;
                tracing::debug!(?command, "radio reported CAT syntax error");
                Err(RadioError::protocol_syntax(command))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(frame: &str, expected: ResponseMatcher, retries: u8) -> OutgoingStep {
        OutgoingStep::new(
            AsciiFrame::new(frame).unwrap(),
            expected,
            CommandPriority::Normal,
            Duration::from_millis(250),
            retries,
        )
    }

    #[test]
    fn transaction_engine_allows_unsolicited_interleaving() {
        let mut engine = TransactionEngine::new();
        engine.enqueue(OutgoingTransaction::new(
            "frequency",
            [step("FA;", ResponseMatcher::Prefix("FA"), 0)],
        ));

        let dispatch = engine.next_dispatch().unwrap();
        assert!(matches!(dispatch, TransactionEvent::Dispatched(_)));

        let unsolicited = engine
            .receive_frame(AsciiFrame::new("IF00014074000     00000000000;").unwrap())
            .unwrap();
        assert!(matches!(unsolicited, TransactionEvent::Unsolicited(_)));

        let completion = engine
            .receive_frame(AsciiFrame::new("FA00014074000;").unwrap())
            .unwrap();
        assert!(matches!(
            completion,
            TransactionEvent::Completed {
                command: "frequency",
                ..
            }
        ));
    }

    #[test]
    fn busy_frame_retries_active_transaction() {
        let mut engine = TransactionEngine::new();
        engine.enqueue(OutgoingTransaction::new(
            "status",
            [step("IF;", ResponseMatcher::Prefix("IF"), 1)],
        ));

        let dispatch = engine.next_dispatch().unwrap();
        assert!(matches!(dispatch, TransactionEvent::Dispatched(_)));

        let retry = engine
            .receive_frame(AsciiFrame::new("O;").unwrap())
            .unwrap();
        assert!(matches!(retry, TransactionEvent::Retrying(_)));

        let completion = engine
            .receive_frame(AsciiFrame::new("IF000140740000000000000000000000000;").unwrap())
            .unwrap();
        assert!(matches!(completion, TransactionEvent::Completed { .. }));
    }

    #[test]
    fn timeout_retries_then_fails() {
        let mut engine = TransactionEngine::new();
        engine.enqueue(OutgoingTransaction::new(
            "mode",
            [step("MD;", ResponseMatcher::Prefix("MD"), 1)],
        ));

        let dispatch = engine.next_dispatch().unwrap();
        assert!(matches!(dispatch, TransactionEvent::Dispatched(_)));

        let retry = engine.on_timeout().unwrap();
        assert!(matches!(retry, TransactionEvent::Retrying(_)));

        let error = engine.on_timeout().unwrap_err();
        assert!(matches!(error, RadioError::Timeout { command: "mode" }));
    }

    #[test]
    fn no_response_steps_are_dispatched_in_order() {
        let mut engine = TransactionEngine::new();
        engine.enqueue(OutgoingTransaction::new(
            "startup",
            [
                step("AI2;", ResponseMatcher::None, 0),
                step("AID250;", ResponseMatcher::None, 0),
            ],
        ));

        let first = engine.next_dispatch().unwrap();
        let second = engine.next_dispatch().unwrap();
        let done = engine.next_dispatch().unwrap();

        match first {
            TransactionEvent::Dispatched(dispatch) => assert_eq!(dispatch.frame.as_str(), "AI2;"),
            other => panic!("unexpected event: {other:?}"),
        }
        match second {
            TransactionEvent::Dispatched(dispatch) => {
                assert_eq!(dispatch.frame.as_str(), "AID250;")
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(matches!(
            done,
            TransactionEvent::CompletedWithoutResponse { command: "startup" }
        ));
    }
}
