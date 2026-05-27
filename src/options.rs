use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct RadioOptions {
    values: HashMap<String, String>,
}

impl RadioOptions {
    pub(crate) fn parse(raw: &str) -> Self {
        let mut values = HashMap::new();

        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let Some((key, value)) = part.split_once('=') else {
                continue;
            };

            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().to_string();

            if key.is_empty() {
                continue;
            }

            values.insert(key, value);
        }

        Self { values }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(&key.to_ascii_lowercase())
            .map(std::string::String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::RadioOptions;

    #[test]
    fn parses_generic_key_value_options() {
        let options = RadioOptions::parse(
            "civ.rig_addr=0x94,civ.controller_addr=0xE0,civ.retry_max=5,unknown=value",
        );

        assert_eq!(options.get("civ.rig_addr"), Some("0x94"));
        assert_eq!(options.get("civ.controller_addr"), Some("0xE0"));
        assert_eq!(options.get("civ.retry_max"), Some("5"));
        assert_eq!(options.get("unknown"), Some("value"));
    }

    #[test]
    fn ignores_malformed_parts() {
        let options = RadioOptions::parse("civ.rig_addr=0x94,broken,=ignored");

        assert_eq!(options.get("civ.rig_addr"), Some("0x94"));
        assert_eq!(options.get("broken"), None);
    }
}
