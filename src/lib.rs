mod error;
mod factory;
mod flex_native;
mod frequency;
mod icom_civ;
mod kenwood;
mod mode;
mod options;
mod transport;
mod yaesu_newcat;

use async_trait::async_trait;

pub use error::{RadioError, Result};
pub use factory::{create_radio, create_radio_with_io, supported_radio_kinds, RadioKind};
pub use flex_native::{FlexNativeModel, FlexNativeRadio};
pub use frequency::Frequency;
pub use icom_civ::{IcomCivRadio, IcomModel};
pub use kenwood::{KenwoodModel, KenwoodRadio};
pub use mode::Mode;
pub use transport::ConnectionConfig;
pub use yaesu_newcat::{YaesuModel, YaesuNewCatRadio};

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
