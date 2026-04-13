use serde::{Deserialize, Serialize};

pub use self::identifier::{Identifier, ScopedIdentifier};
pub use self::root_index::RootIndex;

pub mod identifier;
pub mod root_index;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Legacy,
    V0,
    #[serde(untagged)]
    Unsupported(String),
}

impl Format {
    pub const LATEST: Self = Self::V0;
}

impl From<&str> for Format {
    fn from(value: &str) -> Self {
        match value {
            "legacy" => Format::Legacy,
            "v0" => Format::V0,
            _ => Format::Unsupported(value.to_owned()),
        }
    }
}
