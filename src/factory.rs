use std::str::FromStr;
use tracing::debug;

use crate::{ConnectionConfig, ControllableRadio, GenericElecraft, RadioError, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioKind {
    GenericElecraft,
}

impl RadioKind {
    pub const ALL: &'static [Self] = &[Self::GenericElecraft];

    pub const fn all() -> &'static [Self] {
        Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GenericElecraft => "generic-elecraft",
        }
    }
}

impl FromStr for RadioKind {
    type Err = RadioError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic-elecraft" | "elecraft" | "k4" => Ok(Self::GenericElecraft),
            _ => Err(RadioError::UnknownRadio(value.to_string())),
        }
    }
}

pub const fn supported_radio_kinds() -> &'static [RadioKind] {
    RadioKind::all()
}

pub async fn create_radio(
    kind: RadioKind,
    connection: ConnectionConfig,
) -> Result<Box<dyn ControllableRadio>> {
    debug!(radio_kind = kind.as_str(), ?connection, "creating radio");
    match kind {
        RadioKind::GenericElecraft => Ok(Box::new(GenericElecraft::connect(connection).await?)),
    }
}

#[cfg(test)]
mod tests {
    use super::{supported_radio_kinds, RadioKind};

    #[test]
    fn lists_supported_radio_kinds() {
        assert_eq!(supported_radio_kinds(), &[RadioKind::GenericElecraft]);
    }

    #[test]
    fn parses_supported_radio_aliases() {
        for alias in ["generic-elecraft", "elecraft", "k4"] {
            assert_eq!(
                alias.parse::<RadioKind>().unwrap(),
                RadioKind::GenericElecraft
            );
        }
    }
}
