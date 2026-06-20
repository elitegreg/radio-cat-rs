use std::{
    collections::HashMap,
    fmt,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;
use tokio_serial::{available_ports, SerialPortType};

/// One serial-port listing entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPortListEntry {
    /// Port device path or alias path.
    pub name: String,
    /// Human-readable description for this entry.
    pub description: String,
}

impl fmt::Display for SerialPortListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.description.is_empty() {
            f.write_str(&self.name)
        } else {
            write!(f, "{}: {}", self.name, self.description)
        }
    }
}

/// Errors returned by [`list_serial_ports`].
#[derive(Debug, Error)]
pub enum SerialPortListError {
    /// Failed to query available serial ports.
    #[error(transparent)]
    Query(#[from] tokio_serial::Error),
}

/// List serial ports, including `/dev` aliases on Linux/macOS.
///
/// The returned list is sorted by entry name.
///
/// On Linux/macOS, top-level symlinks in `/dev` are scanned and emitted as
/// first-class alias entries with descriptions in the form `(<target>)`.
pub fn list_serial_ports() -> Result<Vec<SerialPortListEntry>, SerialPortListError> {
    let ports = available_ports()?;

    let mut entries = Vec::new();
    let aliases_by_target = build_dev_symlink_alias_map();

    for port in &ports {
        entries.push(SerialPortListEntry {
            name: port.port_name.clone(),
            description: describe_port_type(&port.port_type),
        });

        if port.port_name.starts_with("/dev/") {
            if let Some(aliases) = aliases_by_target.get(&port.port_name) {
                for alias in aliases {
                    if alias == &port.port_name {
                        continue;
                    }
                    entries.push(SerialPortListEntry {
                        name: alias.clone(),
                        description: format!("alias for {}", port.port_name),
                    });
                }
            }
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.description.cmp(&b.description)));
    entries.dedup_by(|a, b| a.name == b.name && a.description == b.description);

    Ok(entries)
}

fn describe_port_type(port_type: &SerialPortType) -> String {
    match port_type {
        SerialPortType::UsbPort(info) => format!(
            "{} {} {}",
            info.manufacturer.as_deref().unwrap_or(""),
            info.product.as_deref().unwrap_or(""),
            info.serial_number.as_deref().unwrap_or("")
        )
        .trim()
        .to_string(),
        SerialPortType::BluetoothPort => "Bluetooth serial port".to_string(),
        SerialPortType::PciPort => "PCI serial port".to_string(),
        SerialPortType::Unknown => "Serial port".to_string(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_dev_symlink_alias_map() -> HashMap<String, Vec<String>> {
    use std::fs;

    let mut aliases_by_target: HashMap<String, Vec<String>> = HashMap::new();

    let Ok(entries) = fs::read_dir("/dev") else {
        return aliases_by_target;
    };

    for entry_result in entries {
        let Ok(entry) = entry_result else {
            continue;
        };

        let link_path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&link_path) else {
            continue;
        };

        if !metadata.file_type().is_symlink() {
            continue;
        }

        let Ok(target) = fs::read_link(&link_path) else {
            continue;
        };

        let target_abs = if target.is_absolute() {
            normalize_absolute_path(&target)
        } else {
            normalize_absolute_path(&Path::new("/dev").join(target))
        };
        let alias_name = normalize_absolute_path(&link_path);

        aliases_by_target
            .entry(target_abs)
            .or_default()
            .push(alias_name);
    }

    for aliases in aliases_by_target.values_mut() {
        aliases.sort();
        aliases.dedup();
    }

    aliases_by_target
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn build_dev_symlink_alias_map() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

fn normalize_absolute_path(path: &Path) -> String {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }

    normalized.display().to_string()
}
