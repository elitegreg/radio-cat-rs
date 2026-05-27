mod error;
mod factory;
mod frequency;
mod kenwood;
mod mode;
mod transport;

use async_trait::async_trait;

pub use error::{RadioError, Result};
pub use factory::{create_radio, supported_radio_kinds, RadioKind};
pub use frequency::Frequency;
pub use kenwood::KenwoodRadio;
pub use mode::Mode;
pub use transport::ConnectionConfig;

#[async_trait]
pub trait ControllableRadio: Send + Sync {
    async fn get_frequency(&self) -> Result<Frequency>;
    async fn set_frequency(&self, frequency: Frequency) -> Result<()>;

    async fn get_mode(&self) -> Result<Mode>;
    async fn set_mode(&self, mode: Mode) -> Result<()>;

    async fn send_cw(&self, text: &str) -> Result<()>;
    async fn stop_cw(&self) -> Result<()>;

    async fn get_cw_wpm(&self) -> Result<u16>;
    async fn set_cw_wpm(&self, wpm: u16) -> Result<()>;
}
