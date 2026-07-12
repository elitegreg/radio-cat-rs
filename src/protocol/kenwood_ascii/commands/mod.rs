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

use super::{AsciiFrame, CommandPriority, ResponseMatcher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCommand {
    pub frames: Vec<AsciiFrame>,
    pub matcher: ResponseMatcher,
    pub optimistic: Vec<StatePatch>,
    pub priority: CommandPriority,
}

impl EncodedCommand {
    pub fn new(
        frames: Vec<AsciiFrame>,
        matcher: ResponseMatcher,
        optimistic: Vec<StatePatch>,
        priority: CommandPriority,
    ) -> Self {
        Self {
            frames,
            matcher,
            optimistic,
            priority,
        }
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
    pub(crate) switchable: bool,
    pub(crate) main_bandwidth_id: Option<u8>,
}

impl VfoRouting {
    pub fn for_profile(profile: &super::KenwoodAsciiProfile) -> Self {
        Self {
            main_vfo: PhysicalVfo::A,
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

    pub fn set_main_bandwidth_id(&mut self, id: u8) {
        self.main_bandwidth_id = Some(id);
    }
}

impl Default for VfoRouting {
    fn default() -> Self {
        Self {
            main_vfo: PhysicalVfo::A,
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
