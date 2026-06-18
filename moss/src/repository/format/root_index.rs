// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{Format, Identifier, ScopedIdentifier};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootIndex {
    pub formats: FormatsMeta,
    pub streams: IndexMap<Identifier, StreamMeta>,
    pub tags: IndexMap<Identifier, TagMeta>,
    pub history: IndexMap<Identifier, HistoryMeta>,
}

impl RootIndex {
    pub fn resolve_version_to_history<'a>(
        &'a self,
        version: &'a ScopedIdentifier,
    ) -> Option<(&'a Identifier, &'a HistoryMeta)> {
        let ident = match version {
            ScopedIdentifier::Stream(identifier) => self.streams.get(identifier).map(|meta| &meta.history),
            ScopedIdentifier::Tag(identifier) => self.tags.get(identifier).map(|meta| &meta.history),
            ScopedIdentifier::History(identifier) => Some(identifier),
        }?;

        self.get_history(ident).map(|meta| (ident, meta))
    }

    fn get_history(&self, version: &Identifier) -> Option<&HistoryMeta> {
        self.history.get(version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamMeta {
    pub format: Format,
    pub history: Identifier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagMeta {
    pub format: Format,
    pub history: Identifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryMeta {
    pub format: Format,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatsMeta {
    pub v0: FormatV0Meta,
    #[serde(flatten)]
    pub unsupported: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatV0Meta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_via: Option<ScopedIdentifier>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_roundtrip_root_index() {
        let json = r#"{
  "formats": {
    "v0": {
      "upgrade_via": "tag/v0-final"
    },
    "v1": {
      "some_future_v1_key": true
    }
  },
  "streams": {
    "volatile": {
      "format": "v1",
      "history": "3"
    },
    "unstable": {
      "format": "v0",
      "history": "2",
      "tag": "v0-final"
    }
  },
  "tags": {
    "v0-final": {
      "format": "v0",
      "history": "2"
    }
  },
  "history": {
    "1": {
      "format": "v0"
    },
    "2": {
      "format": "v0"
    },
    "3": {
      "format": "v1"
    }
  }
}"#;

        let decoded = RootIndex {
            formats: FormatsMeta {
                v0: FormatV0Meta {
                    upgrade_via: Some(ScopedIdentifier::Tag(ident("v0-final"))),
                },
                unsupported: IndexMap::from_iter([(
                    "v1".to_owned(),
                    serde_json::Value::Object(serde_json::Map::from_iter([(
                        "some_future_v1_key".to_owned(),
                        serde_json::Value::Bool(true),
                    )])),
                )]),
            },
            streams: IndexMap::from_iter([
                (
                    ident("volatile"),
                    StreamMeta {
                        format: Format::Unsupported("v1".to_owned()),
                        history: ident("3"),
                        tag: None,
                    },
                ),
                (
                    ident("unstable"),
                    StreamMeta {
                        format: Format::V0,
                        history: ident("2"),
                        tag: Some(ident("v0-final")),
                    },
                ),
            ]),
            tags: IndexMap::from_iter([(
                ident("v0-final"),
                TagMeta {
                    format: Format::V0,
                    history: ident("2"),
                },
            )]),
            history: IndexMap::from_iter([
                (ident("1"), HistoryMeta { format: Format::V0 }),
                (ident("2"), HistoryMeta { format: Format::V0 }),
                (
                    ident("3"),
                    HistoryMeta {
                        format: Format::Unsupported("v1".to_owned()),
                    },
                ),
            ]),
        };

        let actual_decoded = serde_json::from_str::<RootIndex>(json).unwrap();

        assert_eq!(actual_decoded, decoded);

        let roundtripped = serde_json::to_string_pretty(&actual_decoded).unwrap();

        assert_eq!(roundtripped, json);
    }

    fn ident(s: &str) -> Identifier {
        Identifier::new(s).expect("valid idenitifer")
    }
}
