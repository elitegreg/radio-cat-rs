//! Optional flrig-compatible XML-RPC control server.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
};

use dxr::{Fault, MethodCall, Value};
use dxr_server::{
    Handler, HandlerMap, Server,
    axum::{Router, http::HeaderMap, routing::post},
};
use thiserror::Error;
use tokio::{net::TcpListener, sync::Notify};

use crate::{Frequency, Mode, Power, Radio, RadioCommand, ReceiverPath};

const INVALID_PARAMS: i32 = 400;
const UNAVAILABLE_STATE: i32 = 503;
const RADIO_FAILURE: i32 = 500;
const XML_RPC_ROUTE: &str = "/RPC2";

/// Errors returned while binding or running an XML-RPC server task.
#[derive(Debug, Error)]
pub enum XmlRpcServerError {
    /// The requested listen address could not be bound.
    #[error("failed to bind XML-RPC listener at {address}")]
    Bind {
        address: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    /// The HTTP server stopped with an error.
    #[error("XML-RPC server failed")]
    Serve {
        #[source]
        source: dxr_server::ServerError,
    },
}

/// A graceful-shutdown trigger for an [`XmlRpcServerTask`].
#[derive(Clone, Debug)]
pub struct XmlRpcServerShutdown {
    notify: Arc<Notify>,
}

impl XmlRpcServerShutdown {
    /// Requests graceful shutdown. Calling this more than once is harmless.
    pub fn shutdown(&self) {
        self.notify.notify_one();
    }
}

/// A bound, independently runnable XML-RPC server task.
///
/// Binding and running are separate so applications can surface bind failures
/// before spawning the long-lived task.
#[derive(Debug)]
pub struct XmlRpcServerTask {
    listener: TcpListener,
    server: Server,
    shutdown: XmlRpcServerShutdown,
    local_addr: SocketAddr,
}

impl XmlRpcServerTask {
    /// Binds an XML-RPC server for a connected radio.
    pub async fn bind(radio: Radio, address: SocketAddr) -> Result<Self, XmlRpcServerError> {
        tracing::debug!(%address, "binding XML-RPC listener");
        let listener = TcpListener::bind(address)
            .await
            .map_err(|source| XmlRpcServerError::Bind { address, source })?;
        let local_addr = listener
            .local_addr()
            .map_err(|source| XmlRpcServerError::Bind { address, source })?;

        let handlers = build_handlers(radio);
        let known_methods: Arc<HashSet<&'static str>> =
            Arc::new(handlers.keys().copied().collect());
        let server_handlers = handlers.clone();
        let route = Router::new().route(
            XML_RPC_ROUTE,
            post(move |headers: HeaderMap, body: String| {
                let handlers = server_handlers.clone();
                let known_methods = known_methods.clone();
                async move {
                    log_requested_methods(&body, &known_methods);
                    let (status, response_headers, mut response) =
                        dxr_server::server(handlers, &body, headers).await;
                    response.insert_str(0, "<?xml version=\"1.0\"?>\n");
                    // This is here because Hamlib can't parse valid XML without it.
                    if response.ends_with("</methodResponse>") {
                        response.push('\n');
                    }
                    (status, response_headers, response)
                }
            }),
        );

        let mut server = Server::from_route(route);
        let notify = server.shutdown_trigger();

        tracing::info!(%local_addr, "XML-RPC listener bound");
        Ok(Self {
            listener,
            server,
            shutdown: XmlRpcServerShutdown { notify },
            local_addr,
        })
    }

    /// Returns the actual bound address, including an OS-selected port.
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns a handle that can gracefully stop this task.
    pub fn shutdown_handle(&self) -> XmlRpcServerShutdown {
        self.shutdown.clone()
    }

    /// Runs the server until it is shut down or encounters an error.
    pub async fn run(self) -> Result<(), XmlRpcServerError> {
        tracing::info!(address = %self.local_addr, "XML-RPC server started");
        let result = self
            .server
            .serve_listener(self.listener)
            .await
            .map_err(|source| XmlRpcServerError::Serve { source });
        match &result {
            Ok(()) => tracing::info!(address = %self.local_addr, "XML-RPC server stopped"),
            Err(error) => {
                tracing::error!(address = %self.local_addr, ?error, "XML-RPC server stopped with error")
            }
        }
        result
    }
}

#[derive(Clone, Copy, Debug)]
enum Operation {
    SetFrequency(ReceiverPath),
    SetBandwidth(ReceiverPath),
    GetVersion,
    GetAb,
    GetBandwidth(ReceiverPath),
    GetMode(ReceiverPath),
    GetModes,
    GetPtt,
    GetPower,
    GetSplit,
    GetVfo(ReceiverPath),
    GetXcvr,
    SetMode(ReceiverPath),
    SetPower,
    SetPtt,
    SetSplit,
    Swap,
    SetCwWpm,
    CwText,
    CwSend,
    CopyVfoAToB,
    CopyFreqAToB,
    CopyModeAToB,
}

#[derive(Clone, Debug)]
struct RpcHandler {
    name: &'static str,
    operation: Operation,
    radio: Radio,
}

#[dxr_server::async_trait]
impl Handler for RpcHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> Result<Value, Fault> {
        tracing::debug!(method = self.name, ?params, "handling XML-RPC call");
        let result = self.execute(params).await;
        match &result {
            Ok(value) => tracing::debug!(method = self.name, ?value, "XML-RPC call completed"),
            Err(fault) => tracing::error!(
                method = self.name,
                fault_code = fault.code(),
                fault = fault.string(),
                "XML-RPC call failed"
            ),
        }
        result
    }
}

impl RpcHandler {
    async fn execute(&self, params: &[Value]) -> Result<Value, Fault> {
        match self.operation {
            Operation::SetFrequency(receiver) => {
                let requested = one_frequency(params)?;
                self.radio
                    .set_receiver_frequency(receiver, requested)
                    .await
                    .map_err(radio_fault)?;
                Ok(Value::Double(requested.hz() as f64))
            }
            Operation::SetBandwidth(receiver) => {
                let bandwidth_hz = one_bandwidth(params)?;
                self.radio
                    .set_receiver_filter_bandwidth(receiver, bandwidth_hz)
                    .await
                    .map_err(radio_fault)?;
                Ok(Value::Integer(i32::from(bandwidth_hz)))
            }
            Operation::GetVersion => {
                no_params(params)?;
                Ok(Value::String("1.0.0.0".to_string()))
                //Ok(Value::String(env!("CARGO_PKG_VERSION").to_string()))
            }
            Operation::GetAb => {
                no_params(params)?;
                Ok(Value::String("A".to_string()))
            }
            Operation::GetBandwidth(receiver) => {
                no_params(params)?;
                let state = self.radio.latest_state();
                Ok(Value::Integer(i32::from(receiver_bandwidth(
                    &state, receiver,
                )?)))
            }
            Operation::GetMode(receiver) => {
                no_params(params)?;
                let state = self.radio.latest_state();
                let mode = receiver_mode(&state, receiver)?;
                Ok(Value::String(mode.to_string()))
            }
            Operation::GetModes => {
                no_params(params)?;
                Ok(Value::Array(
                    self.radio
                        .capabilities()
                        .main_rx
                        .modes
                        .iter()
                        .map(|mode| Value::String(mode.to_string()))
                        .collect(),
                ))
            }
            Operation::GetPtt => {
                no_params(params)?;
                let state = self.radio.latest_state();
                let transmitting = state
                    .tx()
                    .and_then(|tx| tx.transmitting())
                    .ok_or_else(|| state_fault("PTT state is unavailable"))?;
                Ok(Value::Integer(i32::from(transmitting)))
            }
            Operation::GetPower => {
                no_params(params)?;
                let state = self.radio.latest_state();
                let power = state
                    .tx()
                    .and_then(|tx| tx.power())
                    .ok_or_else(|| state_fault("power state is unavailable"))?;
                let watts = power.as_watts().round();
                if !(0.0..=f64::from(i32::MAX)).contains(&watts) {
                    return Err(state_fault(
                        "power state cannot be represented as integer watts",
                    ));
                }
                Ok(Value::Integer(watts as i32))
            }
            Operation::GetSplit => {
                no_params(params)?;
                let state = self.radio.latest_state();
                let split = state
                    .tx()
                    .and_then(|tx| tx.split())
                    .ok_or_else(|| state_fault("split state is unavailable"))?;
                Ok(Value::Integer(i32::from(split)))
            }
            Operation::GetVfo(receiver) => {
                no_params(params)?;
                let state = self.radio.latest_state();
                Ok(Value::String(
                    receiver_frequency(&state, receiver)?.hz().to_string(),
                ))
            }
            Operation::GetXcvr => {
                no_params(params)?;
                Ok(Value::String(
                    self.radio.driver_descriptor().display_name.to_string(),
                ))
            }
            Operation::SetMode(receiver) => {
                let mode = one_mode(params)?;
                self.radio
                    .set_receiver_mode(receiver, mode)
                    .await
                    .map_err(radio_fault)?;
                Ok(Value::Integer(1))
            }
            Operation::SetPower => {
                let watts = one_nonnegative_integer(params, "power")?;
                let watts = u32::try_from(watts)
                    .map_err(|_| params_fault("power is outside the supported integer range"))?;
                self.radio
                    .set_tx_power(Power::from_watts(watts))
                    .await
                    .map_err(radio_fault)?;
                Ok(void_value())
            }
            Operation::SetPtt => {
                let transmitting = one_flag(params, "PTT")?;
                self.radio
                    .set_data_ptt(transmitting)
                    .await
                    .map_err(radio_fault)?;
                Ok(void_value())
            }
            Operation::SetSplit => {
                let split = one_flag(params, "split")?;
                self.radio.set_split(split).await.map_err(radio_fault)?;
                Ok(void_value())
            }
            Operation::Swap => {
                no_params(params)?;
                swap_receivers(&self.radio).await?;
                Ok(void_value())
            }
            Operation::SetCwWpm => {
                let wpm = one_nonnegative_integer(params, "CW speed")?;
                let wpm = u8::try_from(wpm)
                    .map_err(|_| params_fault("CW speed is outside the supported integer range"))?;
                self.radio.set_keyer_speed(wpm).await.map_err(radio_fault)?;
                Ok(void_value())
            }
            Operation::CwText => {
                let text = one_string(params, "CW text")?;
                self.radio.send_cw(text).await.map_err(radio_fault)?;
                Ok(Value::Integer(1))
            }
            Operation::CwSend => {
                let enabled = one_flag(params, "CW send")?;
                if !enabled {
                    self.radio.stop_cw().await.map_err(radio_fault)?;
                }
                Ok(void_value())
            }
            Operation::CopyVfoAToB => {
                no_params(params)?;
                copy_receiver(&self.radio, true, true).await?;
                Ok(void_value())
            }
            Operation::CopyFreqAToB => {
                no_params(params)?;
                copy_receiver(&self.radio, true, false).await?;
                Ok(void_value())
            }
            Operation::CopyModeAToB => {
                no_params(params)?;
                copy_receiver(&self.radio, false, true).await?;
                Ok(void_value())
            }
        }
    }
}

fn build_handlers(radio: Radio) -> HandlerMap {
    use Operation as O;

    let methods = [
        ("main.set_frequency", O::SetFrequency(ReceiverPath::Main)),
        ("main.get_version", O::GetVersion),
        ("rig.get_AB", O::GetAb),
        ("rig.get_bw", O::GetBandwidth(ReceiverPath::Main)),
        ("rig.get_bwA", O::GetBandwidth(ReceiverPath::Main)),
        ("rig.get_bwB", O::GetBandwidth(ReceiverPath::Sub)),
        ("rig.get_mode", O::GetMode(ReceiverPath::Main)),
        ("rig.get_modeA", O::GetMode(ReceiverPath::Main)),
        ("rig.get_modeB", O::GetMode(ReceiverPath::Sub)),
        ("rig.get_modes", O::GetModes),
        ("rig.get_ptt", O::GetPtt),
        ("rig.get_power", O::GetPower),
        ("rig.get_split", O::GetSplit),
        ("rig.get_vfo", O::GetVfo(ReceiverPath::Main)),
        ("rig.get_vfoA", O::GetVfo(ReceiverPath::Main)),
        ("rig.get_vfoB", O::GetVfo(ReceiverPath::Sub)),
        ("rig.get_xcvr", O::GetXcvr),
        ("rig.set_frequency", O::SetFrequency(ReceiverPath::Main)),
        ("rig.set_bw", O::SetBandwidth(ReceiverPath::Main)),
        ("rig.set_bwA", O::SetBandwidth(ReceiverPath::Main)),
        ("rig.set_bwB", O::SetBandwidth(ReceiverPath::Sub)),
        ("rig.set_mode", O::SetMode(ReceiverPath::Main)),
        ("rig.set_modeA", O::SetMode(ReceiverPath::Main)),
        ("rig.set_modeB", O::SetMode(ReceiverPath::Sub)),
        ("rig.set_power", O::SetPower),
        ("rig.set_ptt", O::SetPtt),
        ("rig.set_vfo", O::SetFrequency(ReceiverPath::Main)),
        ("rig.set_vfoA", O::SetFrequency(ReceiverPath::Main)),
        ("rig.set_vfoB", O::SetFrequency(ReceiverPath::Sub)),
        ("rig.set_split", O::SetSplit),
        (
            "rig.set_verify_frequency",
            O::SetFrequency(ReceiverPath::Main),
        ),
        ("rig.set_verify_mode", O::SetMode(ReceiverPath::Main)),
        ("rig.set_verify_modeA", O::SetMode(ReceiverPath::Main)),
        ("rig.set_verify_modeB", O::SetMode(ReceiverPath::Sub)),
        ("rig.set_verify_power", O::SetPower),
        ("rig.set_verify_ptt", O::SetPtt),
        ("rig.set_verify_vfoA", O::SetFrequency(ReceiverPath::Main)),
        ("rig.set_verify_vfoB", O::SetFrequency(ReceiverPath::Sub)),
        ("rig.set_verify_split", O::SetSplit),
        ("rig.swap", O::Swap),
        ("rig.cwio_set_wpm", O::SetCwWpm),
        ("rig.cwio_text", O::CwText),
        ("rig.cwio_send", O::CwSend),
        ("rig.vfoA2B", O::CopyVfoAToB),
        ("rig.freqA2B", O::CopyFreqAToB),
        ("rig.modeA2B", O::CopyModeAToB),
    ];

    let handlers: HashMap<&'static str, Box<dyn Handler>> = methods
        .into_iter()
        .map(|(name, operation)| {
            let handler: Box<dyn Handler> = Box::new(RpcHandler {
                name,
                operation,
                radio: radio.clone(),
            });
            (name, handler)
        })
        .collect();
    Arc::new(handlers)
}

fn log_requested_methods(body: &str, known_methods: &HashSet<&'static str>) {
    let Ok(call) = MethodCall::from_xml(body) else {
        return;
    };

    if call.name == "system.multicall" {
        if let Ok(calls) = dxr::multicall::from_multicall_params(call.params) {
            for (name, params) in calls.into_iter().flatten() {
                tracing::debug!(method = %name, ?params, "received XML-RPC multicall member");
                if !known_methods.contains(name.as_str()) {
                    tracing::error!(method = %name, "unknown XML-RPC method called");
                }
            }
        }
    } else if !known_methods.contains(call.name.as_ref()) {
        tracing::error!(method = %call.name, "unknown XML-RPC method called");
    }
}

fn no_params(params: &[Value]) -> Result<(), Fault> {
    if params.is_empty() {
        Ok(())
    } else {
        Err(params_fault(format!(
            "expected no parameters, received {}",
            params.len()
        )))
    }
}

fn one_value(params: &[Value]) -> Result<&Value, Fault> {
    if let [value] = params {
        Ok(value)
    } else {
        Err(params_fault(format!(
            "expected one parameter, received {}",
            params.len()
        )))
    }
}

fn one_frequency(params: &[Value]) -> Result<Frequency, Fault> {
    let value = match one_value(params)? {
        Value::Double(value) => *value,
        Value::Integer(value) => f64::from(*value),
        _ => return Err(params_fault("frequency must be an XML-RPC double")),
    };
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > u64::MAX as f64 {
        return Err(params_fault(
            "frequency must be a finite, non-negative whole number of Hz",
        ));
    }
    Ok(Frequency::from_hz(value as u64))
}

fn one_bandwidth(params: &[Value]) -> Result<u16, Fault> {
    let bandwidth_hz = one_nonnegative_integer(params, "bandwidth")?;
    u16::try_from(bandwidth_hz)
        .map_err(|_| params_fault("bandwidth is outside the supported integer range"))
}

fn one_mode(params: &[Value]) -> Result<Mode, Fault> {
    let value = one_string(params, "mode")?;
    Mode::from_str(value).map_err(|error| params_fault(error.to_string()))
}

fn one_string<'a>(params: &'a [Value], field: &str) -> Result<&'a str, Fault> {
    match one_value(params)? {
        Value::String(value) => Ok(value),
        _ => Err(params_fault(format!("{field} must be an XML-RPC string"))),
    }
}

fn one_nonnegative_integer(params: &[Value], field: &str) -> Result<i32, Fault> {
    match one_value(params)? {
        Value::Integer(value) if *value >= 0 => Ok(*value),
        Value::Integer(_) => Err(params_fault(format!("{field} must not be negative"))),
        _ => Err(params_fault(format!("{field} must be an XML-RPC integer"))),
    }
}

fn one_flag(params: &[Value], field: &str) -> Result<bool, Fault> {
    match one_value(params)? {
        Value::Integer(0) => Ok(false),
        Value::Integer(1) => Ok(true),
        _ => Err(params_fault(format!("{field} must be integer 0 or 1"))),
    }
}

fn receiver_frequency(
    state: &crate::RadioState,
    receiver: ReceiverPath,
) -> Result<Frequency, Fault> {
    match receiver {
        ReceiverPath::Main => state.main_rx().frequency(),
        ReceiverPath::Sub => state.sub_rx().and_then(|receiver| receiver.frequency()),
    }
    .ok_or_else(|| state_fault(format!("{receiver:?} receiver frequency is unavailable")))
}

fn receiver_mode(state: &crate::RadioState, receiver: ReceiverPath) -> Result<Mode, Fault> {
    match receiver {
        ReceiverPath::Main => state.main_rx().mode(),
        ReceiverPath::Sub => state.sub_rx().and_then(|receiver| receiver.mode()),
    }
    .ok_or_else(|| state_fault(format!("{receiver:?} receiver mode is unavailable")))
}

fn receiver_bandwidth(state: &crate::RadioState, receiver: ReceiverPath) -> Result<u16, Fault> {
    match receiver {
        ReceiverPath::Main => state.main_rx().filter().bandwidth_hz(),
        ReceiverPath::Sub => state
            .sub_rx()
            .and_then(|receiver| receiver.filter().bandwidth_hz()),
    }
    .ok_or_else(|| state_fault(format!("{receiver:?} receiver bandwidth is unavailable")))
}

async fn copy_receiver(radio: &Radio, frequency: bool, mode: bool) -> Result<(), Fault> {
    let state = radio.latest_state();
    let frequency_value = frequency
        .then(|| receiver_frequency(&state, ReceiverPath::Main))
        .transpose()?;
    let mode_value = mode
        .then(|| receiver_mode(&state, ReceiverPath::Main))
        .transpose()?;

    let mut commands = Vec::new();
    if let Some(value) = frequency_value {
        commands.push(RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Sub,
            frequency: value,
        });
    }
    if let Some(value) = mode_value {
        commands.push(RadioCommand::SetReceiverMode {
            receiver: ReceiverPath::Sub,
            mode: value,
        });
    }
    validate_commands(radio, &state, &commands)?;
    for command in commands {
        radio.command(command).await.map_err(radio_fault)?;
    }
    Ok(())
}

async fn swap_receivers(radio: &Radio) -> Result<(), Fault> {
    let state = radio.latest_state();
    let main_frequency = receiver_frequency(&state, ReceiverPath::Main)?;
    let main_mode = receiver_mode(&state, ReceiverPath::Main)?;
    let sub_frequency = receiver_frequency(&state, ReceiverPath::Sub)?;
    let sub_mode = receiver_mode(&state, ReceiverPath::Sub)?;
    let commands = [
        RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Main,
            frequency: sub_frequency,
        },
        RadioCommand::SetReceiverMode {
            receiver: ReceiverPath::Main,
            mode: sub_mode,
        },
        RadioCommand::SetReceiverFrequency {
            receiver: ReceiverPath::Sub,
            frequency: main_frequency,
        },
        RadioCommand::SetReceiverMode {
            receiver: ReceiverPath::Sub,
            mode: main_mode,
        },
    ];
    validate_commands(radio, &state, &commands)?;
    for command in commands {
        radio.command(command).await.map_err(radio_fault)?;
    }
    Ok(())
}

fn validate_commands(
    radio: &Radio,
    state: &crate::RadioState,
    commands: &[RadioCommand],
) -> Result<(), Fault> {
    for command in commands {
        radio
            .capabilities()
            .validate_command(command, state)
            .map_err(radio_fault)?;
    }
    Ok(())
}

fn void_value() -> Value {
    Value::String(String::new())
}

fn params_fault(message: impl Into<String>) -> Fault {
    Fault::new(INVALID_PARAMS, message.into())
}

fn state_fault(message: impl Into<String>) -> Fault {
    Fault::new(UNAVAILABLE_STATE, message.into())
}

fn radio_fault(error: crate::RadioError) -> Fault {
    Fault::new(RADIO_FAILURE, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        borrow::Cow,
        io::Write,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::RadioConfig;

    #[derive(Clone)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
        type Writer = LogWriter;

        fn make_writer(&'a self) -> Self::Writer {
            LogWriter(self.0.clone())
        }
    }

    impl Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    async fn handler(name: &'static str, operation: Operation) -> (Radio, RpcHandler) {
        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        let handler = RpcHandler {
            name,
            operation,
            radio: radio.clone(),
        };
        (radio, handler)
    }

    #[tokio::test]
    async fn setters_update_main_sub_and_data_ptt() {
        let (radio, frequency) =
            handler("rig.set_vfoB", Operation::SetFrequency(ReceiverPath::Sub)).await;
        let result = frequency
            .execute(&[Value::Double(7_050_000.0)])
            .await
            .unwrap();
        assert_eq!(result, Value::Double(7_050_000.0));
        assert_eq!(
            radio.latest_state().sub_rx().unwrap().frequency(),
            Some(Frequency::from_hz(7_050_000))
        );

        let ptt = RpcHandler {
            name: "rig.set_ptt",
            operation: Operation::SetPtt,
            radio: radio.clone(),
        };
        assert_eq!(
            ptt.execute(&[Value::Integer(1)]).await.unwrap(),
            void_value()
        );
        assert_eq!(
            radio.latest_state().tx().unwrap().transmitting(),
            Some(true)
        );
        radio.shutdown();
    }

    #[tokio::test]
    async fn bandwidth_commands_use_main_for_vfo_a_and_sub_for_vfo_b() {
        let (radio, bandwidth) =
            handler("rig.set_bwB", Operation::SetBandwidth(ReceiverPath::Sub)).await;
        assert_eq!(
            bandwidth.execute(&[Value::Integer(500)]).await.unwrap(),
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

        let main = RpcHandler {
            name: "rig.get_bwA",
            operation: Operation::GetBandwidth(ReceiverPath::Main),
            radio: radio.clone(),
        };
        assert_eq!(main.execute(&[]).await.unwrap(), Value::Integer(2_400));
        radio.shutdown();
    }

    #[tokio::test]
    async fn getters_use_normalized_state_and_capabilities() {
        let (radio, modes) = handler("rig.get_modes", Operation::GetModes).await;
        let result = modes.execute(&[]).await.unwrap();
        let Value::Array(values) = result else {
            panic!("modes should be an array")
        };
        assert!(values.contains(&Value::String("USB".to_string())));

        let xcvr = RpcHandler {
            name: "rig.get_xcvr",
            operation: Operation::GetXcvr,
            radio: radio.clone(),
        };
        assert_eq!(
            xcvr.execute(&[]).await.unwrap(),
            Value::String("Dummy Radio".to_string())
        );
        radio.shutdown();
    }

    #[tokio::test]
    async fn copy_and_swap_use_main_sub_abstraction() {
        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        radio
            .set_main_frequency(Frequency::from_hz(14_074_000))
            .await
            .unwrap();
        radio.set_main_mode(Mode::DataUsb).await.unwrap();
        copy_receiver(&radio, true, true).await.unwrap();
        assert_eq!(
            radio.latest_state().sub_rx().unwrap().frequency(),
            Some(Frequency::from_hz(14_074_000))
        );
        assert_eq!(
            radio.latest_state().sub_rx().unwrap().mode(),
            Some(Mode::DataUsb)
        );

        radio
            .set_sub_frequency(Frequency::from_hz(7_040_000))
            .await
            .unwrap();
        radio.set_sub_mode(Mode::Cw).await.unwrap();
        swap_receivers(&radio).await.unwrap();
        let state = radio.latest_state();
        assert_eq!(
            state.main_rx().frequency(),
            Some(Frequency::from_hz(7_040_000))
        );
        assert_eq!(state.main_rx().mode(), Some(Mode::Cw));
        assert_eq!(
            state.sub_rx().unwrap().frequency(),
            Some(Frequency::from_hz(14_074_000))
        );
        assert_eq!(state.sub_rx().unwrap().mode(), Some(Mode::DataUsb));
        radio.shutdown();
    }

    #[test]
    fn argument_validation_is_strict_and_descriptive() {
        assert!(one_frequency(&[Value::Double(14_074_000.5)]).is_err());
        assert!(one_bandwidth(&[Value::Integer(-1)]).is_err());
        assert!(one_bandwidth(&[Value::Integer(65_536)]).is_err());
        assert!(one_frequency(&[Value::Double(f64::NAN)]).is_err());
        assert!(one_flag(&[Value::Integer(2)], "PTT").is_err());
        assert!(one_mode(&[Value::String("unknown".to_string())]).is_err());
        assert!(no_params(&[Value::Integer(1)]).is_err());
    }

    #[tokio::test]
    async fn requested_method_surface_is_registered_without_fsk() {
        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        let handlers = build_handlers(radio.clone());
        let expected = [
            "main.set_frequency",
            "main.get_version",
            "rig.get_AB",
            "rig.get_bw",
            "rig.get_bwA",
            "rig.get_bwB",
            "rig.get_mode",
            "rig.get_modeA",
            "rig.get_modeB",
            "rig.get_modes",
            "rig.get_ptt",
            "rig.get_power",
            "rig.get_split",
            "rig.get_vfo",
            "rig.get_vfoA",
            "rig.get_vfoB",
            "rig.get_xcvr",
            "rig.set_frequency",
            "rig.set_bw",
            "rig.set_bwA",
            "rig.set_bwB",
            "rig.set_mode",
            "rig.set_modeA",
            "rig.set_modeB",
            "rig.set_power",
            "rig.set_ptt",
            "rig.set_vfo",
            "rig.set_vfoA",
            "rig.set_vfoB",
            "rig.set_split",
            "rig.set_verify_frequency",
            "rig.set_verify_mode",
            "rig.set_verify_modeA",
            "rig.set_verify_modeB",
            "rig.set_verify_power",
            "rig.set_verify_ptt",
            "rig.set_verify_vfoA",
            "rig.set_verify_vfoB",
            "rig.set_verify_split",
            "rig.swap",
            "rig.cwio_set_wpm",
            "rig.cwio_text",
            "rig.cwio_send",
            "rig.vfoA2B",
            "rig.freqA2B",
            "rig.modeA2B",
        ];
        assert_eq!(handlers.len(), expected.len());
        assert!(expected.iter().all(|name| handlers.contains_key(name)));
        assert!(!handlers.contains_key("rig.fskio_text"));
        radio.shutdown();
    }

    #[tokio::test]
    async fn cwio_text_starts_and_cwio_send_zero_stops() {
        let radio = Radio::connect(RadioConfig::dummy()).await.unwrap();
        radio.set_keyer_speed(20).await.unwrap();
        let text = RpcHandler {
            name: "rig.cwio_text",
            operation: Operation::CwText,
            radio: radio.clone(),
        };
        assert_eq!(
            text.execute(&[Value::String("CQ".to_string())])
                .await
                .unwrap(),
            Value::Integer(1)
        );
        assert_eq!(radio.latest_state().keyer().unwrap().sending(), Some(true));

        let send = RpcHandler {
            name: "rig.cwio_send",
            operation: Operation::CwSend,
            radio: radio.clone(),
        };
        assert_eq!(
            send.execute(&[Value::Integer(1)]).await.unwrap(),
            void_value()
        );
        assert_eq!(radio.latest_state().keyer().unwrap().sending(), Some(true));
        send.execute(&[Value::Integer(0)]).await.unwrap();
        assert_eq!(radio.latest_state().keyer().unwrap().sending(), Some(false));
        radio.shutdown();
    }

    #[test]
    fn unknown_methods_are_logged_at_error_level() {
        let buffer = LogBuffer(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(buffer.clone())
            .finish();
        let body = MethodCall {
            name: Cow::Borrowed("rig.fskio_text"),
            params: vec![],
        }
        .to_xml()
        .unwrap();

        tracing::subscriber::with_default(subscriber, || {
            log_requested_methods(&body, &HashSet::new());
        });

        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("ERROR"), "{output}");
        assert!(output.contains("rig.fskio_text"), "{output}");
        assert!(output.contains("unknown XML-RPC method called"), "{output}");
    }
}
