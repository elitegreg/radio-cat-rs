use async_trait::async_trait;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_serial::SerialPortBuilderExt;

use crate::error::Result;

pub type BoxedCatTransport = Box<dyn CatTransport>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TransportConfig {
    #[default]
    None,
    Serial {
        path: String,
        baud_rate: u32,
    },
    Tcp {
        address: String,
    },
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
        let host = host.as_ref();
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        Self::Tcp {
            address: format!("{host}:{port}"),
        }
    }
}

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
        tracing::debug!(byte_count = bytes.len(), "transport write");
        self.io.write_all(bytes).await?;
        Ok(())
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        let count = self.io.read(buf).await?;
        if count > 0 {
            tracing::trace!(byte_count = count, payload = ?&buf[..count], "transport read bytes");
        }
        Ok(count)
    }

    async fn flush(&mut self) -> Result<()> {
        tracing::trace!("transport flush");
        self.io.flush().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct TcpTransport {
    inner: AsyncIoTransport<TcpStream>,
}

impl TcpTransport {
    pub async fn connect(address: impl AsRef<str>) -> Result<Self> {
        let stream = TcpStream::connect(address.as_ref()).await?;
        Ok(Self {
            inner: AsyncIoTransport::new(stream),
        })
    }

    pub fn into_inner(self) -> TcpStream {
        self.inner.into_inner()
    }
}

#[async_trait]
impl CatTransport for TcpTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.write_all(bytes).await
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.inner.read_some(buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        self.inner.flush().await
    }
}

#[derive(Debug)]
pub struct SerialTransport {
    inner: AsyncIoTransport<tokio_serial::SerialStream>,
}

impl SerialTransport {
    pub fn open(path: impl AsRef<str>, baud_rate: u32) -> Result<Self> {
        let stream = tokio_serial::new(path.as_ref(), baud_rate).open_native_async()?;
        Ok(Self {
            inner: AsyncIoTransport::new(stream),
        })
    }

    pub fn into_inner(self) -> tokio_serial::SerialStream {
        self.inner.into_inner()
    }
}

#[async_trait]
impl CatTransport for SerialTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.write_all(bytes).await
    }

    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.inner.read_some(buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        self.inner.flush().await
    }
}

pub async fn open_transport(config: &TransportConfig) -> Result<Option<BoxedCatTransport>> {
    match config {
        TransportConfig::None => {
            tracing::info!("radio configured without transport");
            Ok(None)
        }
        TransportConfig::Serial { path, baud_rate } => {
            tracing::info!(path = %path, baud_rate = *baud_rate, "opening serial transport");
            let transport = SerialTransport::open(path, *baud_rate)?;
            tracing::info!(path = %path, baud_rate = *baud_rate, "serial transport opened");
            Ok(Some(Box::new(transport)))
        }
        TransportConfig::Tcp { address } => {
            tracing::info!(address = %address, "opening TCP transport");
            let transport = TcpTransport::connect(address).await?;
            tracing::info!(address = %address, "TCP transport connected");
            Ok(Some(Box::new(transport)))
        }
    }
}

pub fn boxed_transport<T>(transport: T) -> BoxedCatTransport
where
    T: CatTransport + 'static,
{
    Box::new(transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_socket_brackets_ipv6_literals() {
        assert_eq!(
            TransportConfig::tcp_socket("2001:db8::1", 4532),
            TransportConfig::tcp("[2001:db8::1]:4532")
        );
    }

    #[test]
    fn tcp_socket_does_not_double_bracket_ipv6_literals() {
        assert_eq!(
            TransportConfig::tcp_socket("[2001:db8::1]", 4532),
            TransportConfig::tcp("[2001:db8::1]:4532")
        );
    }
}
