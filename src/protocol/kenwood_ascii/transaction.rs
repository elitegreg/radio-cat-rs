use std::time::Duration;

use super::AsciiFrame;

pub const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_millis(500);
pub const DEFAULT_BUSY_RETRY_DELAY: Duration = Duration::from_millis(250);

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
    pub(crate) fn matches(&self, frame: &AsciiFrame) -> bool {
        match self {
            Self::None => false,
            Self::Exact(expected) => frame.as_str() == *expected,
            Self::Prefix(prefix) => frame.as_str().starts_with(prefix),
            Self::OneOf(prefixes) => prefixes
                .iter()
                .any(|prefix| frame.as_str().starts_with(prefix)),
        }
    }

    pub(crate) fn expects_response(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepCompletion {
    Written,
    Matched,
    Decoded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingStep {
    pub frame: AsciiFrame,
    pub expected: ResponseMatcher,
    pub priority: CommandPriority,
    pub timeout: Duration,
    pub busy_retries: u8,
    pub busy_retry_delay: Duration,
    pub decode_required: bool,
    pub completion: StepCompletion,
}

impl OutgoingStep {
    pub fn written(frame: AsciiFrame, priority: CommandPriority) -> Self {
        Self {
            frame,
            expected: ResponseMatcher::None,
            priority,
            timeout: DEFAULT_STEP_TIMEOUT,
            busy_retries: 0,
            busy_retry_delay: DEFAULT_BUSY_RETRY_DELAY,
            decode_required: false,
            completion: StepCompletion::Written,
        }
    }

    pub fn decoded(
        frame: AsciiFrame,
        expected: ResponseMatcher,
        priority: CommandPriority,
    ) -> Self {
        Self {
            frame,
            expected,
            priority,
            timeout: DEFAULT_STEP_TIMEOUT,
            busy_retries: 1,
            busy_retry_delay: DEFAULT_BUSY_RETRY_DELAY,
            decode_required: true,
            completion: StepCompletion::Decoded,
        }
    }

    pub fn matched(
        frame: AsciiFrame,
        expected: ResponseMatcher,
        priority: CommandPriority,
    ) -> Self {
        Self {
            frame,
            expected,
            priority,
            timeout: DEFAULT_STEP_TIMEOUT,
            busy_retries: 1,
            busy_retry_delay: DEFAULT_BUSY_RETRY_DELAY,
            decode_required: false,
            completion: StepCompletion::Matched,
        }
    }
}
