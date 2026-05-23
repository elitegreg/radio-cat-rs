use std::path::PathBuf;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_serial::SerialPortBuilderExt;

use crate::{RadioError, Result};

trait AsyncPort: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T> AsyncPort for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

type BoxedPort = Box<dyn AsyncPort>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionConfig {
    Serial { path: PathBuf, baud_rate: u32 },
    Tcp { host: String, port: u16 },
}

impl ConnectionConfig {
    pub fn serial(path: impl Into<PathBuf>, baud_rate: u32) -> Self {
        Self::Serial {
            path: path.into(),
            baud_rate,
        }
    }

    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        Self::Tcp {
            host: host.into(),
            port,
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
}

impl CatTransport {
    pub(crate) async fn open(connection: &ConnectionConfig) -> Result<Self> {
        let io: BoxedPort = match connection {
            ConnectionConfig::Serial { path, baud_rate } => {
                let stream = tokio_serial::new(path.to_string_lossy().into_owned(), *baud_rate)
                    .open_native_async()?;
                Box::new(stream)
            }
            ConnectionConfig::Tcp { host, port } => {
                let stream = TcpStream::connect((host.as_str(), *port)).await?;
                Box::new(stream)
            }
        };

        Ok(Self { io: Mutex::new(io) })
    }

    async fn write_locked<T>(io: &mut T, command: &str) -> Result<()>
    where
        T: AsyncWrite + Unpin + ?Sized,
    {
        io.write_all(command.as_bytes()).await?;
        io.flush().await?;
        Ok(())
    }

    async fn read_response_locked<T>(io: &mut T) -> Result<String>
    where
        T: AsyncRead + Unpin + ?Sized,
    {
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
    }
}

#[async_trait]
impl CommandIo for CatTransport {
    async fn send(&self, command: &str) -> Result<()> {
        let mut io = self.io.lock().await;
        Self::write_locked(&mut *io, command).await
    }

    async fn query(&self, command: &str) -> Result<String> {
        let mut io = self.io.lock().await;
        Self::write_locked(&mut *io, command).await?;
        Self::read_response_locked(&mut *io).await
    }
}
