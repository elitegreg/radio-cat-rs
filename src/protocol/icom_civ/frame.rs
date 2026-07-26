use crate::{Result, error::RadioError};

const MAX_FRAME_BYTES: usize = 512;

/// CI-V's reserved broadcast address. Broadcast frames are never transaction
/// responses because they do not identify this controller/radio pair.
pub const BROADCAST_ADDRESS: u8 = 0x00;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CivFrame {
    bytes: Vec<u8>,
    to: u8,
    from: u8,
    payload: Vec<u8>,
}

impl CivFrame {
    pub fn new(to: u8, from: u8, payload: impl Into<Vec<u8>>) -> Result<Self> {
        let payload = payload.into();
        if payload.is_empty() {
            return Err(RadioError::Decode {
                command: "frame",
                message: "CI-V frame payload is empty".to_string(),
            });
        }

        if payload.len() + 5 > MAX_FRAME_BYTES {
            return Err(RadioError::Decode {
                command: "frame",
                message: format!("CI-V frame exceeds maximum size of {MAX_FRAME_BYTES} bytes"),
            });
        }
        if payload.contains(&0xfd) {
            return Err(RadioError::Decode {
                command: "frame",
                message: "CI-V frame payload contains an embedded terminator".to_string(),
            });
        }

        let mut bytes = Vec::with_capacity(payload.len() + 5);
        bytes.extend_from_slice(&[0xfe, 0xfe, to, from]);
        bytes.extend_from_slice(&payload);
        bytes.push(0xfd);

        Ok(Self {
            bytes,
            to,
            from,
            payload,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn to(&self) -> u8 {
        self.to
    }

    pub fn from(&self) -> u8 {
        self.from
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn command(&self) -> Option<u8> {
        self.payload.first().copied()
    }

    pub fn is_echo_of(&self, sent: &[u8]) -> bool {
        self.bytes == sent
    }

    fn parse(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < 6 {
            return Err(RadioError::Decode {
                command: "frame",
                message: format!("CI-V frame is too short: {} bytes", bytes.len()),
            });
        }
        if !bytes.starts_with(&[0xfe, 0xfe]) || bytes.last() != Some(&0xfd) {
            return Err(RadioError::Decode {
                command: "frame",
                message: "invalid CI-V frame delimiters".to_string(),
            });
        }

        let to = bytes[2];
        let from = bytes[3];
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(RadioError::Decode {
                command: "frame",
                message: format!("CI-V frame exceeds maximum size of {MAX_FRAME_BYTES} bytes"),
            });
        }
        let payload = bytes[4..bytes.len() - 1].to_vec();
        if payload.is_empty() {
            return Err(RadioError::Decode {
                command: "frame",
                message: "CI-V frame payload is empty".to_string(),
            });
        }

        Ok(Self {
            bytes,
            to,
            from,
            payload,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolStatus {
    Ok,
    Ng,
    Collision,
}

impl ProtocolStatus {
    pub fn parse(frame: &CivFrame) -> Option<Self> {
        match frame.payload() {
            [0xfb] => Some(Self::Ok),
            [0xfa] => Some(Self::Ng),
            [0xfc] => Some(Self::Collision),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseMatcher {
    None,
    Ack,
    PayloadPrefix(Vec<u8>),
    OneOf(Vec<Vec<u8>>),
}

impl ResponseMatcher {
    pub fn matches(&self, frame: &CivFrame) -> bool {
        match self {
            Self::None => false,
            Self::Ack => ProtocolStatus::parse(frame) == Some(ProtocolStatus::Ok),
            Self::PayloadPrefix(prefix) => frame.payload().starts_with(prefix),
            Self::OneOf(prefixes) => prefixes
                .iter()
                .any(|prefix| frame.payload().starts_with(prefix)),
        }
    }

    /// Match a response payload and its CI-V endpoint pair.
    pub fn matches_from(&self, frame: &CivFrame, controller: u8, radio: u8) -> bool {
        frame.to() == controller
            && frame.from() == radio
            && frame.to() != BROADCAST_ADDRESS
            && frame.from() != BROADCAST_ADDRESS
            && self.matches(frame)
    }

    pub fn expects_response(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Default, Clone)]
pub struct FrameSplitter {
    buffer: Vec<u8>,
}

impl FrameSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<CivFrame>> {
        let mut frames = Vec::new();
        for byte in bytes {
            self.buffer.push(*byte);
            loop {
                let Some(start) = find_preamble(&self.buffer) else {
                    keep_possible_partial_preamble(&mut self.buffer);
                    break;
                };
                if start > 0 {
                    self.buffer.drain(..start);
                }

                let Some(end_offset) = self.buffer.iter().skip(2).position(|byte| *byte == 0xfd)
                else {
                    if self.buffer.len() > MAX_FRAME_BYTES {
                        resynchronize_oversized(&mut self.buffer);
                    }
                    break;
                };
                let end = end_offset + 2;
                let frame_bytes: Vec<u8> = self.buffer.drain(..=end).collect();
                if frame_bytes.len() <= MAX_FRAME_BYTES
                    && let Ok(frame) = CivFrame::parse(frame_bytes)
                {
                    frames.push(frame);
                }
            }
        }

        Ok(frames)
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

fn find_preamble(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|window| window == [0xfe, 0xfe])
}

fn keep_possible_partial_preamble(buffer: &mut Vec<u8>) {
    if buffer.last() == Some(&0xfe) {
        buffer.drain(..buffer.len().saturating_sub(1));
    } else {
        buffer.clear();
    }
}

fn resynchronize_oversized(buffer: &mut Vec<u8>) {
    if let Some(start) = buffer[2..]
        .windows(2)
        .position(|window| window == [0xfe, 0xfe])
    {
        buffer.drain(..start + 2);
    } else {
        keep_possible_partial_preamble(buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips_bytes() {
        let frame = CivFrame::new(0xa4, 0xe0, [0x25, 0x00]).unwrap();
        assert_eq!(
            frame.as_bytes(),
            &[0xfe, 0xfe, 0xa4, 0xe0, 0x25, 0x00, 0xfd]
        );
        assert_eq!(frame.to(), 0xa4);
        assert_eq!(frame.from(), 0xe0);
        assert_eq!(frame.payload(), &[0x25, 0x00]);
    }

    #[test]
    fn splitter_handles_noise_partials_and_multiple_frames() {
        let mut splitter = FrameSplitter::new();
        assert!(splitter.push(&[0x00, 0xfe]).unwrap().is_empty());
        assert_eq!(splitter.buffered_len(), 1);

        let frames = splitter
            .push(&[
                0xfe, 0xe0, 0xa4, 0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00, 0xfd, 0xfe, 0xfe, 0xe0,
                0xa4, 0xfb, 0xfd,
            ])
            .unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(
            frames[0].payload(),
            &[0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00]
        );
        assert_eq!(ProtocolStatus::parse(&frames[1]), Some(ProtocolStatus::Ok));
    }

    #[test]
    fn matcher_recognizes_ack_and_payload_prefix() {
        let ack = CivFrame::new(0xe0, 0xa4, [0xfb]).unwrap();
        assert!(ResponseMatcher::Ack.matches(&ack));

        let response = CivFrame::new(0xe0, 0xa4, [0x26, 0x01, 0x01, 0x00, 0x03]).unwrap();
        assert!(ResponseMatcher::PayloadPrefix(vec![0x26, 0x01]).matches(&response));
        assert!(ResponseMatcher::Ack.matches_from(&ack, 0xe0, 0xa4));
        assert!(!ResponseMatcher::Ack.matches_from(
            &CivFrame::new(0xe0, 0xb2, [0xfb]).unwrap(),
            0xe0,
            0xa4
        ));
        assert!(!ResponseMatcher::Ack.matches_from(
            &CivFrame::new(0x00, 0xa4, [0xfb]).unwrap(),
            0xe0,
            0xa4
        ));
    }

    #[test]
    fn constructor_rejects_embedded_terminator_and_oversized_payload() {
        assert!(CivFrame::new(0xe0, 0xa4, [0x25, 0xfd]).is_err());
        assert!(CivFrame::new(0xe0, 0xa4, vec![0; 508]).is_err());
    }

    #[test]
    fn splitter_recovers_after_an_oversized_frame() {
        let mut splitter = FrameSplitter::new();
        let mut bytes = vec![0xfe, 0xfe, 0xe0, 0xa4];
        bytes.extend(std::iter::repeat_n(0x01, 512));
        bytes.extend_from_slice(&[0xfd, 0xfe, 0xfe, 0xe0, 0xa4, 0xfb, 0xfd]);

        let frames = splitter.push(&bytes).unwrap();

        assert_eq!(frames.len(), 1);
        assert_eq!(ProtocolStatus::parse(&frames[0]), Some(ProtocolStatus::Ok));
        assert_eq!(splitter.buffered_len(), 0);
    }
}
