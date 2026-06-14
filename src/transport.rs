use async_trait::async_trait;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_serial::SerialPortBuilderExt;

use crate::error::Result;

pub type BoxedCatTransport = Box<dyn CatTransport>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportConfig {
    None,
    Serial { path: String, baud_rate: u32 },
    Tcp { address: String },
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self::None
    }
}

impl TransportConfig {
    pub fn none() -> Self {
        Self::None
    }

    pub fn serial(path: impl Into<String>, baud_rate: u32) -> Self {
        Self::Serial {
            path: path.into(),
            baud_rate,
        }
    }

    pub fn tcp(address: impl Into<String>) -> Self {
        Self::Tcp {
            address: address.into(),
        }
    }

    pub fn tcp_socket(host: impl AsRef<str>, port: u16) -> Self {
        Self::Tcp {
            address: format!("{}:{}", host.as_ref(), port),
        }
    }
}

pub type ConnectionConfig = TransportConfig;

#[async_trait]
pub trait CatTransport: Send {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()>;
    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize>;
    async fn flush(&mut self) -> Result<()>;
}

#[derive(Debug)]
pub struct AsyncIoTransport<T> {
    io: T,
}

impl<T> AsyncIoTransport<T> {
    pub fn new(io: T) -> Self {
        Self { io }
    }

    pub fn into_inner(self) -> T {
        self.io
    }
}

#[async_trait]
impl<T> CatTransport for AsyncIoTransport<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.io.write_all(bytes).await?;
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.io.read(buf).await?)
    }

    async fn flush(&mut self) -> Result<()> {
        self.io.flush().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    pub async fn connect(address: impl AsRef<str>) -> Result<Self> {
        let stream = TcpStream::connect(address.as_ref()).await?;
        Ok(Self { stream })
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

#[async_trait]
impl CatTransport for TcpTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).await?;
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.stream.read(buf).await?)
    }

    async fn flush(&mut self) -> Result<()> {
        self.stream.flush().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SerialTransport {
    stream: tokio_serial::SerialStream,
}

impl SerialTransport {
    pub fn open(path: impl AsRef<str>, baud_rate: u32) -> Result<Self> {
        let stream = tokio_serial::new(path.as_ref(), baud_rate).open_native_async()?;
        Ok(Self { stream })
    }

    pub fn into_inner(self) -> tokio_serial::SerialStream {
        self.stream
    }
}

#[async_trait]
impl CatTransport for SerialTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).await?;
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.stream.read(buf).await?)
    }

    async fn flush(&mut self) -> Result<()> {
        self.stream.flush().await?;
        Ok(())
    }
}

pub async fn open_transport(config: &TransportConfig) -> Result<Option<BoxedCatTransport>> {
    match config {
        TransportConfig::None => Ok(None),
        TransportConfig::Serial { path, baud_rate } => {
            Ok(Some(Box::new(SerialTransport::open(path, *baud_rate)?)))
        }
        TransportConfig::Tcp { address } => {
            Ok(Some(Box::new(TcpTransport::connect(address).await?)))
        }
    }
}

pub fn boxed_transport<T>(transport: T) -> BoxedCatTransport
where
    T: CatTransport + 'static,
{
    Box::new(transport)
}
