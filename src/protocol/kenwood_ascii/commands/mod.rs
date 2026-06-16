pub mod filter;
pub mod frequency;
pub mod info;
pub mod keyer;
pub mod mode;
pub mod rf;
pub mod rit_xit;
pub mod split;
pub mod tx;

use crate::{update::StatePatch, UpdateSource};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerCommandEncoding {
    StandardWatts { watts: u16 },
    K4High { watts: u16 },
    K4Low { deci_watts: u16 },
    K4Milli { deci_milliwatts: u16 },
}
