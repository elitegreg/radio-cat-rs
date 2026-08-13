use std::{borrow::Cow, net::SocketAddr, time::Duration};

use dxr::{FaultResponse, MethodCall, MethodResponse, Value};
use radio_cat_rs::{Frequency, Radio, RadioConfig, xml_rpc::XmlRpcServerTask};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

async fn start_server() -> (
    Radio,
    SocketAddr,
    radio_cat_rs::xml_rpc::XmlRpcServerShutdown,
    tokio::task::JoinHandle<Result<(), radio_cat_rs::xml_rpc::XmlRpcServerError>>,
) {
    let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
    let task = XmlRpcServerTask::bind(radio.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let address = task.local_addr();
    let shutdown = task.shutdown_handle();
    let join = tokio::spawn(task.run());
    (radio, address, shutdown, join)
}

async fn call(address: SocketAddr, method: &str, params: Vec<Value>) -> String {
    let body = MethodCall {
        name: Cow::Owned(method.to_string()),
        params,
    }
    .to_xml()
    .unwrap();
    post(address, &body).await
}

async fn post(address: SocketAddr, body: &str) -> String {
    let request = format!(
        "POST /RPC2 HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    response.split_once("\r\n\r\n").unwrap().1.to_string()
}

#[tokio::test]
async fn http_calls_control_the_radio_and_return_flrig_types() {
    let (radio, address, shutdown, join) = start_server().await;

    let version = call(address, "main.get_version", vec![]).await;
    assert!(
        version.starts_with("<?xml version=\"1.0\"?>\n"),
        "{version}"
    );
    assert!(version.ends_with("</methodResponse>\n"), "{version}");
    assert_eq!(
        MethodResponse::from_xml(&version).unwrap().value,
        Value::String("1.0.0.0".to_string())
    );

    let active_vfo = call(address, "rig.get_AB", vec![]).await;
    assert_eq!(
        MethodResponse::from_xml(&active_vfo).unwrap().value,
        Value::String("A".to_string())
    );

    let response = call(
        address,
        "rig.set_verify_vfoB",
        vec![Value::Double(7_050_000.0)],
    )
    .await;
    assert_eq!(
        MethodResponse::from_xml(&response).unwrap().value,
        Value::Double(7_050_000.0)
    );
    assert_eq!(
        radio.latest_state().sub_rx().unwrap().frequency(),
        Some(Frequency::from_hz(7_050_000))
    );

    let response = call(address, "rig.get_vfoB", vec![]).await;
    assert_eq!(
        MethodResponse::from_xml(&response).unwrap().value,
        Value::String("7050000".to_string())
    );

    let response = call(address, "rig.set_bwA", vec![Value::Integer(2_800)]).await;
    assert_eq!(
        MethodResponse::from_xml(&response).unwrap().value,
        Value::Integer(2_800)
    );
    assert_eq!(
        radio.latest_state().main_rx().filter().bandwidth_hz(),
        Some(2_800)
    );

    let response = call(address, "rig.set_bwB", vec![Value::Integer(500)]).await;
    assert_eq!(
        MethodResponse::from_xml(&response).unwrap().value,
        Value::Integer(500)
    );
    assert_eq!(
        radio
            .latest_state()
            .sub_rx()
            .unwrap()
            .filter()
            .bandwidth_hz(),
        Some(500)
    );

    for (method, bandwidth) in [
        ("rig.get_bw", 2_800),
        ("rig.get_bwA", 2_800),
        ("rig.get_bwB", 500),
    ] {
        let response = call(address, method, vec![]).await;
        assert_eq!(
            MethodResponse::from_xml(&response).unwrap().value,
            Value::Integer(bandwidth),
            "{method}"
        );
    }

    shutdown.shutdown();
    tokio::time::timeout(Duration::from_secs(2), join)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    radio.shutdown();
}

#[tokio::test]
async fn multicall_and_unknown_methods_use_xml_rpc_faults() {
    let (radio, address, shutdown, join) = start_server().await;

    let calls = vec![
        ("main.get_version".to_string(), ()),
        ("rig.get_xcvr".to_string(), ()),
    ];
    let multicall = dxr::multicall::into_multicall_params(calls).unwrap();
    let response = call(address, "system.multicall", vec![multicall]).await;
    let Value::Array(results) = MethodResponse::from_xml(&response).unwrap().value else {
        panic!("multicall response should be an array")
    };
    assert_eq!(results.len(), 2);

    let response = call(
        address,
        "rig.fskio_text",
        vec![Value::String("TEST".into())],
    )
    .await;
    let fault = FaultResponse::from_xml(&response).unwrap().fault;
    assert_eq!(fault.code(), 404);
    assert_eq!(fault.string(), "Unknown method.");

    let response = call(address, "rig.set_ptt", vec![Value::Boolean(true)]).await;
    let fault = FaultResponse::from_xml(&response).unwrap().fault;
    assert_eq!(fault.code(), 400);

    let response = post(address, "<not-an-xml-rpc-call>").await;
    assert!(FaultResponse::from_xml(&response).is_ok());

    shutdown.shutdown();
    tokio::time::timeout(Duration::from_secs(2), join)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    radio.shutdown();
}

#[tokio::test]
async fn shutdown_requested_before_run_is_not_lost() {
    let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
    let task = XmlRpcServerTask::bind(radio.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    task.shutdown_handle().shutdown();
    tokio::time::timeout(Duration::from_secs(2), task.run())
        .await
        .unwrap()
        .unwrap();
    radio.shutdown();
}
