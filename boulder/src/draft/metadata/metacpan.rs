// SPDX-FileCopyrightText: Copyright © 2025 Aeryn OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use url::Url;

use super::Source;

pub fn source(upstream: &Url) -> Option<Source> {
    let re = Regex::new(
        r#"^https://cpan\.metacpan\.org/authors/id/[A-Z]/[A-Z]{2}/[A-Z0-9]+/([A-Za-z0-9._+-]+-\d+(?:\.\d+)*)(?:\.tar\.(?:gz|bz2|xz)|\.zip)$"#
    ).ok()?;

    let captures = re.captures(upstream.as_str())?;

    let module = captures.get(1)?.as_str().to_owned();
    let parts: Vec<&str> = module.split('-').collect();

    let name = format!(
        "perl-{}-{}",
        parts
            .first()
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "unknown".to_owned()),
        parts
            .get(1)
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "unknown".to_owned())
    );

    let version = parts.get(2).unwrap_or(&"0.0");

    let homepage = format!(
        "https://metacpan.org/pod/{}::{}",
        parts.first().unwrap_or(&"unknown"),
        parts.get(1).unwrap_or(&"unknown")
    );

    Some(Source {
        name,
        version: (*version).to_owned(),
        homepage,
        uri: upstream.to_string(),
    })
}
