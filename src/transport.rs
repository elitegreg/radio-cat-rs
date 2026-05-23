use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_serial::SerialPortBuilderExt;

use crate::{RadioError, Result};

trait AsyncPort: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T> AsyncPort for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

type BoxedPort = Box<dyn AsyncPort>;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionConfig {
    Serial {
        path: PathBuf,
        baud_rate: u32,
        timeout: Duration,
    },
    Tcp {
        host: String,
        port: u16,
        timeout: Duration,
    },
}

impl ConnectionConfig {
    pub fn serial(path: impl Into<PathBuf>, baud_rate: u32) -> Self {
        Self::Serial {
            path: path.into(),
            baud_rate,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        Self::Tcp {
            host: host.into(),
            port,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        match self {
            Self::Serial {
                path, baud_rate, ..
            } => Self::Serial {
                path,
                baud_rate,
                timeout,
            },
            Self::Tcp { host, port, .. } => Self::Tcp {
                host,
                port,
                timeout,
            },
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            Self::Serial { timeout, .. } | Self::Tcp { timeout, .. } => *timeout,
        }
    }
}

#[async_trait]
pub(crate) trait CommandIo: Send + Sync {
    async fn send(&self, command: &str) -> Result<()>;
    async fn query(&self, command: &str) -> Result<String>;
}

pub(crate) struct CatTransport {
    io: Mutex<BoxedPort>,
    timeout: Duration,
}

impl CatTransport {
    pub(crate) async fn open(connection: &ConnectionConfig) -> Result<Self> {
        let timeout_duration = connection.timeout();
        let io: BoxedPort = match connection {
            ConnectionConfig::Serial {
                path, baud_rate, ..
            } => {
                let stream = tokio_serial::new(path.to_string_lossy().into_owned(), *baud_rate)
                    .open_native_async()?;
                Box::new(stream)
            }
            ConnectionConfig::Tcp {
                host,
                port,
                timeout: connect_timeout,
                ..
            } => {
                let stream = timeout(*connect_timeout, TcpStream::connect((host.as_str(), *port)))
                    .await
                    .map_err(|_| RadioError::Timeout {
                        operation: "TCP connect",
                    })??;
                Box::new(stream)
            }
        };

        Ok(Self {
            io: Mutex::new(io),
            timeout: timeout_duration,
        })
    }

    async fn write_locked<T>(io: &mut T, command: &str, timeout_duration: Duration) -> Result<()>
    where
        T: AsyncWrite + Unpin + ?Sized,
    {
        timeout(timeout_duration, async {
            io.write_all(command.as_bytes()).await?;
            io.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| RadioError::Timeout { operation: "write" })??;

        Ok(())
    }

    async fn read_response_locked<T>(io: &mut T, timeout_duration: Duration) -> Result<String>
    where
        T: AsyncRead + Unpin + ?Sized,
    {
        timeout(timeout_duration, async {
            let mut response = Vec::new();

            loop {
                let mut byte = [0_u8; 1];
                let read = io.read(&mut byte).await?;

                if read == 0 {
                    return Err(RadioError::ConnectionClosed);
                }

                response.push(byte[0]);

                if byte[0] == b';' {
                    break;
                }
            }

            Ok(String::from_utf8(response)?)
        })
        .await
        .map_err(|_| RadioError::Timeout {
            operation: "read response",
        })?
    }
}

#[async_trait]
impl CommandIo for CatTransport {
    async fn send(&self, command: &str) -> Result<()> {
        let mut io = self.io.lock().await;
        Self::write_locked(&mut *io, command, self.timeout).await
    }

    async fn query(&self, command: &str) -> Result<String> {
        let mut io = self.io.lock().await;
        Self::write_locked(&mut *io, command, self.timeout).await?;
        Self::read_response_locked(&mut *io, self.timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_config_uses_default_timeout() {
        match ConnectionConfig::tcp("127.0.0.1", 5002) {
            ConnectionConfig::Tcp { timeout, .. } => assert_eq!(timeout, DEFAULT_TIMEOUT),
            ConnectionConfig::Serial { .. } => panic!("expected TCP config"),
        }
    }

    #[test]
    fn connection_config_can_override_timeout() {
        let timeout = Duration::from_millis(250);

        match ConnectionConfig::serial("/dev/ttyUSB0", 38_400).with_timeout(timeout) {
            ConnectionConfig::Serial {
                baud_rate,
                timeout: configured_timeout,
                ..
            } => {
                assert_eq!(baud_rate, 38_400);
                assert_eq!(configured_timeout, timeout);
            }
            ConnectionConfig::Tcp { .. } => panic!("expected serial config"),
        }
    }
}
