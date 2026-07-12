pub mod filter;
pub mod frequency;
pub mod info;
pub mod keyer;
pub mod mode;
pub mod rf;
pub mod rit_xit;
pub mod split;
pub mod tx;

use crate::{command::ReceiverPath, error::RadioError, update::StatePatch, Result, UpdateSource};

use super::{AsciiFrame, CommandPriority, OutgoingStep, ResponseMatcher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCommand {
    pub steps: Vec<OutgoingStep>,
    // Compatibility views for codec consumers. Production dispatch uses only
    // `steps`, which is the authoritative transaction representation.
    pub(crate) frames: Vec<AsciiFrame>,
    pub(crate) matcher: ResponseMatcher,
    pub completion_patches: Vec<StatePatch>,
    pub(crate) priority: CommandPriority,
}

impl EncodedCommand {
    pub fn new(
        frames: Vec<AsciiFrame>,
        matcher: ResponseMatcher,
        completion_patches: Vec<StatePatch>,
        priority: CommandPriority,
    ) -> Self {
        let steps = frames
            .iter()
            .cloned()
            .map(|frame| match matcher_for_frame(&frame, &matcher) {
                ResponseMatcher::None => OutgoingStep::written(frame, priority),
                expected => OutgoingStep::decoded(frame, expected, priority),
            })
            .collect();
        Self {
            steps,
            frames,
            matcher,
            completion_patches,
            priority,
        }
    }

    pub fn with_steps(steps: Vec<OutgoingStep>, completion_patches: Vec<StatePatch>) -> Self {
        let frames = steps.iter().map(|step| step.frame.clone()).collect();
        let matcher = steps
            .last()
            .map(|step| step.expected.clone())
            .unwrap_or(ResponseMatcher::None);
        let priority = steps
            .first()
            .map(|step| step.priority)
            .unwrap_or(CommandPriority::Normal);
        Self {
            steps,
            frames,
            matcher,
            completion_patches,
            priority,
        }
    }
}

fn matcher_for_frame(frame: &AsciiFrame, matcher: &ResponseMatcher) -> ResponseMatcher {
    match matcher {
        ResponseMatcher::OneOf(prefixes) => prefixes
            .iter()
            .copied()
            .find(|prefix| frame.command() == *prefix)
            .or_else(|| {
                prefixes
                    .iter()
                    .copied()
                    .find(|prefix| frame.command().starts_with(prefix))
            })
            .map(ResponseMatcher::Prefix)
            .unwrap_or_else(|| matcher.clone()),
        _ => matcher.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub patches: Vec<StatePatch>,
    pub source_hint: Option<UpdateSource>,
}

impl DecodedFrame {
    pub fn new(patches: Vec<StatePatch>) -> Self {
        Self {
            patches,
            source_hint: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrequencyCommandTarget {
    Main,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PhysicalVfo {
    #[default]
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfoRouting {
    pub(crate) main_vfo: PhysicalVfo,
    pub(crate) tx_vfo: PhysicalVfo,
    pub(crate) switchable: bool,
    pub(crate) main_bandwidth_id: Option<u8>,
}

impl VfoRouting {
    pub fn for_profile(profile: &super::KenwoodAsciiProfile) -> Self {
        Self {
            main_vfo: PhysicalVfo::A,
            tx_vfo: PhysicalVfo::A,
            switchable: matches!(
                profile.id(),
                "yaesu-ftdx10"
                    | "yaesu-ft710"
                    | "kenwood-ts590"
                    | "kenwood-ts890"
                    | "kenwood-ts2000"
                    | "kenwood-ts480"
                    | "kenwood-ts570"
                    | "kenwood-ts870"
                    | "elecraft-k2"
            ),
            main_bandwidth_id: None,
        }
    }

    pub fn receiver_for_vfo(self, vfo: PhysicalVfo) -> ReceiverPath {
        if vfo == self.main_vfo {
            ReceiverPath::Main
        } else {
            ReceiverPath::Sub
        }
    }

    pub fn vfo_for_receiver(self, receiver: ReceiverPath) -> PhysicalVfo {
        match (self.main_vfo, receiver) {
            (PhysicalVfo::A, ReceiverPath::Main) | (PhysicalVfo::B, ReceiverPath::Sub) => {
                PhysicalVfo::A
            }
            _ => PhysicalVfo::B,
        }
    }

    pub fn target_for_receiver(self, receiver: ReceiverPath) -> char {
        match self.vfo_for_receiver(receiver) {
            PhysicalVfo::A => '0',
            PhysicalVfo::B => '1',
        }
    }

    pub fn receiver_for_target(self, target: u8) -> Result<ReceiverPath> {
        match target {
            b'0' => Ok(self.receiver_for_vfo(PhysicalVfo::A)),
            b'1' => Ok(self.receiver_for_vfo(PhysicalVfo::B)),
            other => Err(RadioError::Decode {
                command: "VFO target",
                message: format!("expected VFO target 0/1, got {:?}", other as char),
            }),
        }
    }

    pub fn select(&mut self, vfo: PhysicalVfo) -> bool {
        if self.switchable && self.main_vfo != vfo {
            self.main_vfo = vfo;
            true
        } else {
            false
        }
    }

    pub fn tx_vfo(self) -> PhysicalVfo {
        self.tx_vfo
    }

    pub fn set_tx_vfo(&mut self, vfo: PhysicalVfo) {
        self.tx_vfo = vfo;
    }

    pub fn set_split(&mut self, split: bool) {
        self.tx_vfo = if split {
            match self.main_vfo {
                PhysicalVfo::A => PhysicalVfo::B,
                PhysicalVfo::B => PhysicalVfo::A,
            }
        } else {
            self.main_vfo
        };
    }

    pub fn split(self) -> bool {
        self.tx_vfo != self.main_vfo
    }

    pub fn set_main_bandwidth_id(&mut self, id: u8) {
        self.main_bandwidth_id = Some(id);
    }
}

impl Default for VfoRouting {
    fn default() -> Self {
        Self {
            main_vfo: PhysicalVfo::A,
            tx_vfo: PhysicalVfo::A,
            switchable: false,
            main_bandwidth_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerCommandEncoding {
    StandardWatts { watts: u16 },
    K4High { watts: u16 },
    K4Low { deci_watts: u16 },
    K4Milli { deci_milliwatts: u16 },
}
