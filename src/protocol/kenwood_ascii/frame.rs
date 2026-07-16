use crate::{error::RadioError, Result};

const MAX_FRAME_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AsciiFrame {
    text: String,
}

impl AsciiFrame {
    pub fn new(text: impl Into<String>) -> Result<Self> {
        let text = text.into();
        validate_frame(&text)?;
        Ok(Self { text })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    pub fn command(&self) -> &str {
        let content = self.text.trim_end_matches(';');
        let split_at = content
            .find(|ch: char| !ch.is_ascii_alphabetic() && ch != '$')
            .unwrap_or(content.len());
        &content[..split_at]
    }

    pub fn payload(&self) -> &str {
        let content = self.text.trim_end_matches(';');
        &content[self.command().len()..]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolErrorFrame<'a> {
    Syntax { command: Option<&'a str> },
    Communication,
    Busy,
}

impl<'a> ProtocolErrorFrame<'a> {
    pub fn parse(frame: &'a AsciiFrame) -> Option<Self> {
        match frame.as_str() {
            "?;" => Some(Self::Syntax { command: None }),
            "E;" => Some(Self::Communication),
            "O;" => Some(Self::Busy),
            _ => {
                let body = frame.as_str().strip_suffix(";")?;
                let command = body.strip_suffix('?')?;
                if command.is_empty() {
                    None
                } else {
                    Some(Self::Syntax {
                        command: Some(command),
                    })
                }
            }
        }
    }

    pub fn to_error(self) -> RadioError {
        match self {
            Self::Syntax { command } => RadioError::protocol_syntax(command),
            Self::Communication => RadioError::ProtocolCommunication,
            Self::Busy => RadioError::ProtocolBusy,
        }
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

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<AsciiFrame>> {
        if !bytes.is_ascii() {
            return Err(RadioError::Decode {
                command: "frame",
                message: "non-ASCII bytes in Kenwood ASCII stream".to_string(),
            });
        }

        let mut frames = Vec::new();
        for byte in bytes {
            self.buffer.push(*byte);
            if *byte != b';' && self.buffer.len() > MAX_FRAME_BYTES {
                self.buffer.clear();
                continue;
            }
            if *byte == b';' {
                let frame_bytes = std::mem::take(&mut self.buffer);
                if frame_bytes.len() <= MAX_FRAME_BYTES {
                    let text = String::from_utf8(frame_bytes).expect("ASCII bytes are valid UTF-8");
                    if let Ok(frame) = AsciiFrame::new(text) {
                        frames.push(frame);
                    }
                }
            }
        }

        Ok(frames)
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

fn validate_frame(text: &str) -> Result<()> {
    if !text.is_ascii() {
        return Err(RadioError::Decode {
            command: "frame",
            message: "frame contains non-ASCII characters".to_string(),
        });
    }

    if !text.ends_with(';') {
        return Err(RadioError::Decode {
            command: "frame",
            message: "frame is missing semicolon terminator".to_string(),
        });
    }

    if text.len() > MAX_FRAME_BYTES {
        return Err(RadioError::Decode {
            command: "frame",
            message: format!("frame exceeds maximum size of {MAX_FRAME_BYTES} bytes"),
        });
    }

    if text.matches(';').count() != 1 || text[..text.len() - 1].contains(';') {
        return Err(RadioError::Decode {
            command: "frame",
            message: "frame contains an embedded semicolon".to_string(),
        });
    }

    if text.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(RadioError::Decode {
            command: "frame",
            message: "frame contains an ASCII control character".to_string(),
        });
    }

    if text == ";" {
        return Err(RadioError::Decode {
            command: "frame",
            message: "frame body is empty".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_handles_partial_and_multiple_frames() {
        let mut splitter = FrameSplitter::new();
        assert!(splitter.push(b"FA000140").unwrap().is_empty());
        assert_eq!(splitter.buffered_len(), 8);

        let frames = splitter.push(b"74000;MD2;").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_str(), "FA00014074000;");
        assert_eq!(frames[0].command(), "FA");
        assert_eq!(frames[0].payload(), "00014074000");
        assert_eq!(frames[1].as_str(), "MD2;");
        assert_eq!(splitter.buffered_len(), 0);
    }

    #[test]
    fn splitter_rejects_non_ascii_bytes() {
        let mut splitter = FrameSplitter::new();
        let error = splitter.push(&[0xff]).unwrap_err();
        assert!(matches!(
            error,
            RadioError::Decode {
                command: "frame",
                ..
            }
        ));
    }

    #[test]
    fn protocol_error_frames_are_recognized() {
        let plain = AsciiFrame::new("?;").unwrap();
        assert_eq!(
            ProtocolErrorFrame::parse(&plain),
            Some(ProtocolErrorFrame::Syntax { command: None })
        );

        let elecraft = AsciiFrame::new("FA?;").unwrap();
        assert_eq!(
            ProtocolErrorFrame::parse(&elecraft),
            Some(ProtocolErrorFrame::Syntax {
                command: Some("FA")
            })
        );

        let busy = AsciiFrame::new("O;").unwrap();
        assert_eq!(
            ProtocolErrorFrame::parse(&busy),
            Some(ProtocolErrorFrame::Busy)
        );
    }

    #[test]
    fn constructor_rejects_embedded_terminators_and_controls() {
        assert!(AsciiFrame::new("FA;MD2;").is_err());
        assert!(AsciiFrame::new("FA\n;").is_err());
    }

    #[test]
    fn splitter_discards_oversized_frame_and_recovers() {
        let mut splitter = FrameSplitter::new();
        let mut bytes = vec![b'X'; 1025];
        bytes.extend_from_slice(b";MD2;");

        let frames = splitter.push(&bytes).unwrap();

        assert_eq!(
            frames.iter().map(AsciiFrame::as_str).collect::<Vec<_>>(),
            ["MD2;"]
        );
        assert_eq!(splitter.buffered_len(), 0);
    }
}
