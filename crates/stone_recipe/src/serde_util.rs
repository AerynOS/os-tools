use serde::Deserialize;

pub fn default_true() -> bool {
    true
}

pub fn stringy_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Inner {
        Bool(bool),
        String(String),
    }

    match Inner::deserialize(deserializer)? {
        Inner::Bool(bool) => Ok(bool),
        // Really yaml...
        Inner::String(s) => match s.as_str() {
            "y" | "Y" | "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" => Ok(true),
            "n" | "N" | "no" | "No" | "NO" | "false" | "False" | "FALSE" | "off" | "Off" | "OFF" => Ok(false),
            _ => Err(serde::de::Error::custom("invalid boolean: expected true or false")),
        },
    }
}
