use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use radio_cat_rs::{
    protocol::icom_civ::{profile_by_id, CivFrame},
    CatTransport, Frequency, Mode, Radio, RadioConfig, RadioError, Result, StateUpdateCapability,
};
use tokio::sync::Mutex;

#[derive(Debug, Default, Clone)]
struct SharedMockTransport {
    inner: Arc<Mutex<MockInner>>,
}

#[derive(Debug, Default)]
struct MockInner {
    written_frames: Vec<Vec<u8>>,
    read_chunks: VecDeque<Vec<u8>>,
    fail_writes: bool,
}

impl SharedMockTransport {
    async fn push_read(&self, chunk: Vec<u8>) {
        self.inner.lock().await.read_chunks.push_back(chunk);
    }

    async fn written_frames(&self) -> Vec<Vec<u8>> {
        self.inner.lock().await.written_frames.clone()
    }

    async fn written_len(&self) -> usize {
        self.inner.lock().await.written_frames.len()
    }

    async fn set_fail_writes(&self, fail_writes: bool) {
        self.inner.lock().await.fail_writes = fail_writes;
    }
}

#[async_trait]
impl CatTransport for SharedMockTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.fail_writes {
            return Err(RadioError::Transport("injected write failure".to_string()));
        }
        inner.written_frames.push(bytes.to_vec());
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        let Some(mut chunk) = self.inner.lock().await.read_chunks.pop_front() else {
            return Err(RadioError::Timeout {
                command: "mock-transport-read",
            });
        };

        let count = chunk.len().min(buf.len());
        buf[..count].copy_from_slice(&chunk[..count]);

        if count < chunk.len() {
            let remainder = chunk.split_off(count);
            self.inner.lock().await.read_chunks.push_front(remainder);
        }

        Ok(count)
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn ic705_actor_skips_mode_set_when_validation_query_confirms_state() {
    let transport = SharedMockTransport::default();

    transport
        .push_read(response([0x26, 0x00, 0x03, 0x00, 0x03]))
        .await;

    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=0.2"),
        transport.clone(),
    )
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().main_rx.mode == Some(Mode::Cw) }
    })
    .await
    .unwrap();

    let baseline = transport.written_len().await;
    transport
        .push_read(response([0x26, 0x00, 0x03, 0x00, 0x03]))
        .await;

    radio.set_main_mode(Mode::Cw).await.unwrap();

    let written = transport.written_frames().await;
    let additional = &written[baseline..];
    assert_eq!(additional, &[command([0x26, 0x00])]);
}

#[tokio::test]
async fn ic705_actor_sends_mode_set_when_validation_query_disagrees() {
    let transport = SharedMockTransport::default();

    transport
        .push_read(response([0x26, 0x00, 0x03, 0x00, 0x03]))
        .await;

    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=0.2"),
        transport.clone(),
    )
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().main_rx.mode == Some(Mode::Cw) }
    })
    .await
    .unwrap();

    let baseline = transport.written_len().await;
    transport
        .push_read(response([0x26, 0x00, 0x01, 0x00, 0x03]))
        .await;
    transport.push_read(response([0xfb])).await;

    radio.set_main_mode(Mode::Cw).await.unwrap();

    let written = transport.written_frames().await;
    let additional = &written[baseline..];
    assert_eq!(
        additional,
        &[
            command([0x26, 0x00]),
            command([0x26, 0x00, 0x03, 0x00, 0x01])
        ]
    );
}

#[tokio::test]
async fn ic705_actor_handles_startup_async_errors_and_command_ack() {
    let profile = profile_by_id("icom-ic705").unwrap();
    let transport = SharedMockTransport::default();

    let startup_async_mode = response([0x26, 0x00, 0x17, 0x00, 0x03]);
    let startup_frequency = response([0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00]);
    transport
        .push_read(join_frames([
            startup_async_mode.as_slice(),
            startup_frequency.as_slice(),
        ]))
        .await;

    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=0.2"),
        transport.clone(),
    )
    .await
    .unwrap();

    assert_eq!(
        radio.capabilities().state_updates,
        StateUpdateCapability::Polling
    );

    wait_for(Duration::from_secs(2), || {
        let transport = transport.clone();
        async move { transport.written_len().await >= profile.startup.len() }
    })
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move {
            let state = radio.latest_state();
            state.main_rx.frequency == Some(Frequency::from_hz(14_074_000))
                && state.main_rx.mode == Some(Mode::DigitalVoice)
        }
    })
    .await
    .unwrap();

    let invalid_bcd_frequency = response([0x25, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff]);
    let async_wfm_mode = response([0x26, 0x00, 0x06, 0x00, 0x03]);
    transport
        .push_read(join_frames([
            invalid_bcd_frequency.as_slice(),
            async_wfm_mode.as_slice(),
        ]))
        .await;

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().main_rx.mode == Some(Mode::Wfm) }
    })
    .await
    .unwrap();

    let set_frequency = command([0x25, 0x00, 0x00, 0x00, 0x03, 0x07, 0x00]);
    transport.push_read(set_frequency.clone()).await;
    transport.push_read(response([0xfb])).await;

    radio
        .set_main_frequency(Frequency::from_hz(7_030_000))
        .await
        .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().main_rx.frequency == Some(Frequency::from_hz(7_030_000)) }
    })
    .await
    .unwrap();

    assert!(transport
        .written_frames()
        .await
        .iter()
        .any(|frame| frame == &set_frequency));
}

#[tokio::test]
async fn civ_negative_ack_leaves_accepted_state_unchanged() {
    let transport = SharedMockTransport::default();
    transport
        .push_read(response([0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00]))
        .await;
    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=5"),
        transport.clone(),
    )
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let transport = transport.clone();
        async move { transport.written_len().await > 0 }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let before = radio.latest_state().main_rx.frequency;
    transport.push_read(response([0xfa])).await;

    let error = radio
        .set_main_frequency(Frequency::from_hz(7_030_000))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        RadioError::CommandRejected {
            protocol: "icom-civ",
            reason: "negative acknowledgement",
        }
    ));
    assert_eq!(radio.latest_state().main_rx.frequency, before);
}

#[tokio::test]
async fn civ_ignores_wrong_address_response_and_ack() {
    let transport = SharedMockTransport::default();
    transport
        .push_read(response([0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00]))
        .await;
    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=5"),
        transport.clone(),
    )
    .await
    .unwrap();
    wait_for(Duration::from_secs(2), || {
        let transport = transport.clone();
        async move { transport.written_len().await > 0 }
    })
    .await
    .unwrap();

    let before = radio.latest_state().main_rx.frequency;
    transport
        .push_read(
            CivFrame::new(0xe0, 0xb2, [0x25, 0x00, 0x00, 0x00, 0x03, 0x07, 0x00])
                .unwrap()
                .as_bytes()
                .to_vec(),
        )
        .await;
    transport
        .push_read(
            CivFrame::new(0xe0, 0xb2, [0xfb])
                .unwrap()
                .as_bytes()
                .to_vec(),
        )
        .await;
    transport.push_read(response([0xfb])).await;

    radio
        .set_main_frequency(Frequency::from_hz(7_030_000))
        .await
        .unwrap();

    assert_eq!(
        radio.latest_state().main_rx.frequency,
        Some(Frequency::from_hz(7_030_000))
    );
    assert_ne!(radio.latest_state().main_rx.frequency, before);
}

#[tokio::test]
async fn write_failure_leaves_accepted_state_unchanged() {
    let transport = SharedMockTransport::default();
    transport
        .push_read(response([0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00]))
        .await;
    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=5"),
        transport.clone(),
    )
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let transport = transport.clone();
        async move { transport.written_len().await > 0 }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let before = radio.latest_state().main_rx.frequency;
    transport.set_fail_writes(true).await;
    let error = radio
        .set_main_frequency(Frequency::from_hz(7_030_000))
        .await
        .unwrap_err();

    assert!(error.is_transport());
    assert_eq!(radio.latest_state().main_rx.frequency, before);
}

#[tokio::test]
async fn ic705_refresh_writes_startup_queries_and_propagates_failure() {
    let transport = SharedMockTransport::default();
    transport
        .push_read(response([0x25, 0x00, 0x00, 0x40, 0x07, 0x14, 0x00]))
        .await;
    let radio = Radio::connect_with_transport(
        RadioConfig::new("icom-ic705").with_options("poll_interval=5"),
        transport.clone(),
    )
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().main_rx.frequency.is_some() }
    })
    .await
    .unwrap();

    let baseline = transport.written_len().await;
    let error = radio.refresh().await.unwrap_err();

    assert!(matches!(error, RadioError::Timeout { .. }));
    let written = transport.written_frames().await;
    assert_eq!(&written[baseline..], &[command([0x25, 0x00])]);
}

fn command<const N: usize>(payload: [u8; N]) -> Vec<u8> {
    CivFrame::new(0xa4, 0xe0, payload)
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn response<const N: usize>(payload: [u8; N]) -> Vec<u8> {
    CivFrame::new(0xe0, 0xa4, payload)
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn join_frames<'a>(frames: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut joined = Vec::new();
    for frame in frames {
        joined.extend_from_slice(frame);
    }
    joined
}

async fn wait_for<F, Fut>(duration: Duration, mut condition: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        if condition().await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RadioError::Timeout {
                command: "test-condition",
            });
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
