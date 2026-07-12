use std::{collections::VecDeque, sync::Arc, time::Duration};

use async_trait::async_trait;
use radio_cat_rs::{
    CatTransport, Frequency, LeveledSetting, Mode, Power, Radio, RadioConfig, RadioError, Result,
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
    transport.push_read(b"V1.0.0.0\n".to_vec()).await;
    transport.push_read(b"HABC1234\n".to_vec()).await;
    transport.push_read(lines(["R1|0||"])).await;
    transport.push_read(lines(["R2|0||"])).await;
    transport.push_read(lines(["R3|0||"])).await;
    transport
        .push_read(lines([
            "S0|slice 0 RF_frequency=14.074 mode=DIGU filter_lo=300 filter_hi=2700 rit_on=1 rit_freq=50 xit_on=0 xit_freq=0 nr=on nr_level=25 nb=off anf=on",
            "S0|cwx wpm=25 break_in_delay=100",
            "S0|transmit freq=14.074000 rfpower=100 tunepower=10 vox_enable=0 speed=25",
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
                && state.keyer.as_ref().and_then(|keyer| keyer.speed_wpm) == Some(25)
                && state.tx.as_ref().and_then(|tx| tx.power)
                    == Some(radio_cat_rs::Power::from_watts(100))
        }
    })
    .await
    .unwrap();

    let baseline = transport.written_len().await;
    transport.push_read(lines(["R4|0||"])).await;
    radio.set_tx_power(Power::from_watts(50)).await.unwrap();

    transport.push_read(lines(["R5|0||"])).await;
    radio
        .set_main_frequency(Frequency::from_hz(7_030_000))
        .await
        .unwrap();

    transport.push_read(lines(["R6|0||"])).await;
    radio
        .set_main_noise_reduction(LeveledSetting::enabled(37))
        .await
        .unwrap();

    transport
        .push_read(lines(["R7|0||", "S0|cwx wpm=30 break_in_delay=100"]))
        .await;
    radio.set_keyer_speed(30).await.unwrap();

    transport.push_read(lines(["R8|0||"])).await;
    transport
        .push_read(lines([
            "R9|0||",
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
                && state.keyer.as_ref().and_then(|keyer| keyer.speed_wpm) == Some(30)
                && state.tx.as_ref().and_then(|tx| tx.power) == Some(Power::from_watts(50))
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
            b"C4|transmit set rfpower=50\n".to_vec(),
            b"C5|slice t 0 7.030000 autopan=1\n".to_vec(),
            b"C6|slice s 0 nr=on nr_level=37\n".to_vec(),
            b"C7|cwx wpm 30\n".to_vec(),
            b"C8|slice s 0 tx=1\n".to_vec(),
            b"C9|xmit 1\n".to_vec(),
        ]
    );
}

#[tokio::test]
async fn flexradio_startup_requires_the_configured_slice_status() {
    let transport = SharedMockTransport::default();
    transport.push_read(lines(["V1.0.0.0", "HABC1234"])).await;
    transport.push_read(lines(["R1|0||"])).await;
    transport.push_read(lines(["R2|0||"])).await;
    transport.push_read(lines(["R3|0||"])).await;
    transport
        .push_read(lines(["S0|slice 1 RF_frequency=14.074 mode=DIGU"]))
        .await;

    let result =
        Radio::connect_with_transport(RadioConfig::new("flexradio-smartsdr"), transport.clone())
            .await;

    let error = match result {
        Ok(_) => panic!("startup unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RadioError::Transport(_) | RadioError::Timeout { .. }
    ));
    assert_eq!(transport.written_len().await, 3);
}

#[tokio::test]
async fn flexradio_startup_rejects_a_nonexistent_slice() {
    let transport = SharedMockTransport::default();
    transport.push_read(lines(["V1.0.0.0", "HABC1234"])).await;
    transport
        .push_read(lines(["R1|00000015|slice does not exist"]))
        .await;

    let result = Radio::connect_with_transport(
        RadioConfig::new("flexradio-smartsdr").with_options("slice=7"),
        transport.clone(),
    )
    .await;

    assert!(matches!(result, Err(RadioError::Decode { .. })));
    assert_eq!(transport.written_len().await, 1);
}

#[tokio::test]
async fn flexradio_refresh_reissues_subscriptions_and_preserves_connection_state() {
    let transport = SharedMockTransport::default();
    transport.push_read(lines(["V1.0.0.0", "HABC1234"])).await;
    transport.push_read(lines(["R1|0||"])).await;
    transport.push_read(lines(["R2|0||"])).await;
    transport.push_read(lines(["R3|0||"])).await;
    transport
        .push_read(lines([
            "S0|slice 0 RF_frequency=14.074 mode=DIGU filter_lo=300 filter_hi=2700 rit_on=0 rit_freq=0 xit_on=0 xit_freq=0 nr=off nb=off anf=off",
            "S0|cwx wpm=25 break_in_delay=100",
            "S0|transmit freq=14.074000 rfpower=100 tunepower=10 vox_enable=0 speed=25",
        ]))
        .await;

    let radio =
        Radio::connect_with_transport(RadioConfig::new("flexradio-smartsdr"), transport.clone())
            .await
            .unwrap();
    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().connection == radio_cat_rs::ConnectionState::Ready }
    })
    .await
    .unwrap();

    let baseline = transport.written_len().await;
    transport
        .push_read(lines([
            "S0|slice 0 RF_frequency=7.030 mode=CW filter_lo=300 filter_hi=2700 rit_on=0 rit_freq=0 xit_on=0 xit_freq=0 nr=off nb=off anf=off",
            "R4|0||",
        ]))
        .await;
    transport.push_read(lines(["R5|0||"])).await;
    transport.push_read(lines(["R6|0||"])).await;

    radio.refresh().await.unwrap();

    assert_eq!(
        radio.latest_state().connection,
        radio_cat_rs::ConnectionState::Ready
    );
    assert_eq!(
        radio.latest_state().main_rx.frequency,
        Some(Frequency::from_hz(7_030_000))
    );

    let written = transport.written_frames().await;
    assert_eq!(
        &written[baseline..],
        &[
            b"C4|sub slice 0\n".to_vec(),
            b"C5|sub cwx all\n".to_vec(),
            b"C6|sub tx all\n".to_vec(),
        ]
    );
}

#[tokio::test]
async fn flexradio_uses_configured_slice_option() {
    let transport = SharedMockTransport::default();
    transport.push_read(lines(["V1.0.0.0", "HABC1234"])).await;
    transport.push_read(lines(["R1|0||"])).await;
    transport.push_read(lines(["R2|0||"])).await;
    transport.push_read(lines(["R3|0||"])).await;
    transport
        .push_read(lines([
            "S0|slice 2 RF_frequency=14.074 mode=DIGU filter_lo=300 filter_hi=2700 rit_on=0 rit_freq=0 xit_on=0 xit_freq=0 nr=off nb=off anf=off",
            "S0|cwx wpm=25 break_in_delay=100",
            "S0|transmit freq=14.074000 rfpower=100 tunepower=10 vox_enable=0 speed=25",
            "S0|interlock state=READY",
        ]))
        .await;

    let radio = Radio::connect_with_transport(
        RadioConfig::new("flexradio-smartsdr").with_options("slice=2"),
        transport.clone(),
    )
    .await
    .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().connection == radio_cat_rs::ConnectionState::Ready }
    })
    .await
    .unwrap();

    let written = transport.written_frames().await;
    assert_eq!(
        &written[..3],
        &[
            b"C1|sub slice 2\n".to_vec(),
            b"C2|sub cwx all\n".to_vec(),
            b"C3|sub tx all\n".to_vec(),
        ]
    );

    transport.push_read(lines(["R4|0||"])).await;
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

    let written = transport.written_frames().await;
    assert!(written.contains(&b"C4|slice t 2 7.030000 autopan=1\n".to_vec()));
}

#[tokio::test]
async fn flexradio_command_error_does_not_stop_connection() {
    let transport = SharedMockTransport::default();
    transport.push_read(lines(["V1.0.0.0", "HABC1234"])).await;
    transport.push_read(lines(["R1|0||"])).await;
    transport.push_read(lines(["R2|0||"])).await;
    transport.push_read(lines(["R3|0||"])).await;
    transport
        .push_read(lines([
            "S0|slice 0 RF_frequency=14.074 mode=DIGU filter_lo=300 filter_hi=2700 rit_on=0 rit_freq=0 xit_on=0 xit_freq=0 nr=off nb=off anf=off",
            "S0|cwx wpm=25 break_in_delay=100",
            "S0|transmit freq=14.074000 rfpower=100 tunepower=10 vox_enable=0 speed=25",
            "S0|interlock state=READY",
        ]))
        .await;

    let radio =
        Radio::connect_with_transport(RadioConfig::new("flexradio-smartsdr"), transport.clone())
            .await
            .unwrap();

    wait_for(Duration::from_secs(2), || {
        let radio = radio.clone();
        async move { radio.latest_state().connection == radio_cat_rs::ConnectionState::Ready }
    })
    .await
    .unwrap();

    transport
        .push_read(lines(["R4|00000015|invalid command"]))
        .await;
    assert!(radio.set_tx_power(Power::from_watts(50)).await.is_err());
    let state = radio.latest_state();
    assert_eq!(state.connection, radio_cat_rs::ConnectionState::Ready);
    assert_eq!(
        state.tx.as_ref().and_then(|tx| tx.power),
        Some(Power::from_watts(100))
    );

    transport
        .push_read(lines(["R5|0||", "S0|slice 0 RF_frequency=7.03"]))
        .await;
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
