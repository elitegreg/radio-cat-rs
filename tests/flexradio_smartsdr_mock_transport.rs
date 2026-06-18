use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use radio_cat_rs::{
    CatTransport, Frequency, LeveledSetting, Mode, Radio, RadioConfig, RadioError, Result,
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
}

#[async_trait]
impl CatTransport for SharedMockTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.lock().await.written_frames.push(bytes.to_vec());
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        let Some(mut chunk) = self.inner.lock().await.read_chunks.pop_front() else {
            return Ok(0);
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
async fn flexradio_actor_bootstraps_and_sends_expected_commands() {
    let transport = SharedMockTransport::default();
    transport.push_read(lines(["V1.0.0.0", "HABC1234"])).await;
    transport
        .push_read(lines([
            "R1|0||",
            "S0|slice 0 RF_frequency=14.074 mode=DIGU filter_lo=300 filter_hi=2700 rit_on=1 rit_freq=50 xit_on=0 xit_freq=0 nr=on nr_level=25 nb=off anf=on",
            "S0|interlock state=READY",
        ]))
        .await;

    let radio =
        Radio::connect_with_transport(RadioConfig::new("flexradio-smartsdr"), transport.clone())
            .await
            .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move {
            let state = radio.latest_state();
            state.connection == radio_cat_rs::ConnectionState::Ready
                && state.main_rx.frequency == Some(Frequency::from_hz(14_074_000))
                && state.main_rx.mode == Some(Mode::DataUsb)
                && state.main_rx.rf.noise_reduction == Some(LeveledSetting::enabled(25))
        }
    })
    .await
    .unwrap();

    let baseline = transport.written_len().await;
    transport
        .push_read(lines(["R2|0||", "S0|slice 0 RF_frequency=7.03"]))
        .await;
    radio
        .set_main_frequency(Frequency::from_hz(7_030_000))
        .await
        .unwrap();

    transport
        .push_read(lines(["R3|0||", "S0|slice 0 nr=on nr_level=37"]))
        .await;
    radio
        .set_main_noise_reduction(LeveledSetting::enabled(37))
        .await
        .unwrap();

    transport.push_read(lines(["R4|0||"])).await;
    transport
        .push_read(lines([
            "R5|0||",
            "S0|interlock state=TRANSMITTING source=SW",
        ]))
        .await;
    radio.set_ptt(true).await.unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move {
            let state = radio.latest_state();
            state.main_rx.frequency == Some(Frequency::from_hz(7_030_000))
                && state.main_rx.rf.noise_reduction == Some(LeveledSetting::enabled(37))
                && state.tx.as_ref().and_then(|tx| tx.transmitting) == Some(true)
        }
    })
    .await
    .unwrap();

    let written = transport.written_frames().await;
    let additional = &written[baseline..];
    assert_eq!(
        additional,
        &[
            b"C2|slice t 0 7.030000 autopan=1\n".to_vec(),
            b"C3|slice s 0 nr=on nr_level=37\n".to_vec(),
            b"C4|slice s 0 tx=1\n".to_vec(),
            b"C5|xmit 1\n".to_vec(),
        ]
    );
}

fn lines<const N: usize>(lines: [&str; N]) -> Vec<u8> {
    let mut joined = Vec::new();
    for line in lines {
        joined.extend_from_slice(line.as_bytes());
        joined.push(b'\n');
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
