use std::collections::VecDeque;

use async_trait::async_trait;
use radio_cat_rs::{
    protocol::kenwood_ascii::{
        filter, frequency, info, keyer, mode, profile_by_id, rf, rit_xit, split, tx, AsciiFrame,
        FrameSplitter, StartupStep,
    },
    CatTransport, RadioError, RadioState, Result, StateReducer,
};

#[derive(Debug, Default)]
struct MockTransport {
    written_frames: Vec<String>,
    read_chunks: VecDeque<Vec<u8>>,
}

impl MockTransport {
    fn written_frames(&self) -> &[String] {
        &self.written_frames
    }
}

#[async_trait]
impl CatTransport for MockTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.written_frames
            .push(
                String::from_utf8(bytes.to_vec()).map_err(|error| RadioError::Decode {
                    command: "mock-transport",
                    message: error.to_string(),
                })?,
            );
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        let Some(mut chunk) = self.read_chunks.pop_front() else {
            return Ok(0);
        };

        let count = chunk.len().min(buf.len());
        buf[..count].copy_from_slice(&chunk[..count]);

        if count < chunk.len() {
            let remainder = chunk.split_off(count);
            self.read_chunks.push_front(remainder);
        }

        Ok(count)
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn startup_sends_auto_info_and_explicit_query_plan() {
    let profile = profile_by_id("kenwood-ts590").unwrap();
    let mut transport = MockTransport::default();
    let mut reducer = StateReducer::new(RadioState::default());

    run_startup(profile, &mut transport, &mut reducer)
        .await
        .unwrap();

    let expected = vec![
        "AI2;", "IF;", "FA;", "FB;", "FR;", "FT;", "MD;", "DA;", "RT;", "XT;", "SH;", "SL;", "NT;",
        "NB;", "NR;", "PA;", "RA;", "PC;", "KS;",
    ];
    assert_eq!(transport.written_frames(), expected.as_slice());
}

async fn run_startup(
    profile: &'static radio_cat_rs::protocol::kenwood_ascii::KenwoodAsciiProfile,
    transport: &mut MockTransport,
    reducer: &mut StateReducer,
) -> Result<()> {
    let mut splitter = FrameSplitter::new();

    for step in profile.startup {
        let frames = encode_startup_step(profile, *step)?;
        for frame in frames {
            transport.write_all(frame.as_bytes()).await?;
            transport.flush().await?;
            drain_transport_frames(profile, transport, &mut splitter, |frame| {
                if let Some(decoded) = decode_frame(profile, &frame, reducer.state()).unwrap() {
                    reducer.apply_patches(decoded.patches);
                }
            })
            .await?;
        }
    }

    Ok(())
}

async fn drain_transport_frames(
    profile: &'static radio_cat_rs::protocol::kenwood_ascii::KenwoodAsciiProfile,
    transport: &mut MockTransport,
    splitter: &mut FrameSplitter,
    mut on_frame: impl FnMut(AsciiFrame),
) -> Result<()> {
    let _ = profile;

    loop {
        let mut buf = [0u8; 64];
        let count = transport.read_some(&mut buf).await?;
        if count == 0 {
            break;
        }

        for frame in splitter.push(&buf[..count])? {
            on_frame(frame);
        }
    }

    Ok(())
}

fn encode_startup_step(
    profile: &'static radio_cat_rs::protocol::kenwood_ascii::KenwoodAsciiProfile,
    step: StartupStep,
) -> Result<Vec<AsciiFrame>> {
    match step {
        StartupStep::AutoInfo(frame) => Ok(vec![AsciiFrame::new(frame.to_string())?]),
        StartupStep::Query(semantic) => {
            if let Some(encoded) = frequency::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = info::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = mode::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = split::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = rit_xit::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = filter::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = rf::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = tx::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }
            if let Some(encoded) = keyer::encode_query(profile, semantic)? {
                return Ok(encoded.steps.into_iter().map(|step| step.frame).collect());
            }

            Err(RadioError::Decode {
                command: "startup",
                message: format!("unhandled startup semantic query {semantic:?}"),
            })
        }
    }
}

fn decode_frame(
    profile: &'static radio_cat_rs::protocol::kenwood_ascii::KenwoodAsciiProfile,
    frame: &AsciiFrame,
    state: &RadioState,
) -> Result<Option<radio_cat_rs::protocol::kenwood_ascii::DecodedFrame>> {
    let mut vfo_routing = radio_cat_rs::protocol::kenwood_ascii::VfoRouting::for_profile(profile);
    if let Some(decoded) = info::decode(profile, frame, state, &mut vfo_routing)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = frequency::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = mode::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = split::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = rit_xit::decode(profile, frame)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = filter::decode(profile, frame, state)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = rf::decode(profile, frame)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = tx::decode(profile, frame)? {
        return Ok(Some(decoded));
    }
    if let Some(decoded) = keyer::decode(profile, frame)? {
        return Ok(Some(decoded));
    }

    Ok(None)
}
