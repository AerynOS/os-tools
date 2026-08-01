// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use itertools::Itertools;

use crate::upstream::Upstream;

mod basic;
mod github;
mod gitlab;
mod metacpan;
mod pypi;

#[derive(Default)]
pub struct Metadata {
    pub source: Source,
    upstreams: Vec<Upstream>,
}

#[derive(Default)]
pub struct Source {
    pub name: String,
    pub version: String,
    pub homepage: String,
    pub uri: String,
}

impl Metadata {
    pub fn new(upstreams: Vec<Upstream>) -> Self {
        let mut source = Source::default();

        // Try to identify source metadata from the first upstream
        if let Some(upstream) = upstreams.first() {
            let uri = upstream.uri();
            for matcher in Matcher::ALL {
                if let Some(matched) = match matcher {
                    Matcher::Basic => basic::source(&uri),
                    Matcher::Github => github::source(&uri),
                    Matcher::Gitlab => gitlab::source(&uri),
                    Matcher::Pypi => pypi::source(&uri),
                    Matcher::Metacpan => metacpan::source(&uri),
                } {
                    source = matched;
                    break;
                }
            }
        }

        Self { source, upstreams }
    }

    pub fn upstreams(&self) -> String {
        self.upstreams
            .iter()
            .enumerate()
            .map(|(i, upstream)| {
                let uri_to_use = if i == 0 && !self.source.uri.is_empty() {
                    &self.source.uri
                } else {
                    &upstream.uri().to_string()
                };
                let hash = upstream.hash();
                format!("    - {uri_to_use} : {hash}")
            })
            .join("\n")
    }
}

enum Matcher {
    Basic,
    Gitlab,
    Github,
    Pypi,
    Metacpan,
}

impl Matcher {
    const ALL: &'static [Self] = &[Self::Github, Self::Gitlab, Self::Pypi, Self::Metacpan, Self::Basic];
}
