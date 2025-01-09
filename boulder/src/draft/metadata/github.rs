// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use url::Url;

use super::Source;

pub fn source(upstream: &Url) -> Option<Source> {
    let automatic_regex = Regex::new(
        r"\w+\:\/\/github\.com\/([A-Za-z0-9-_]+)\/([A-Za-z0-9-_]+)\/archive\/refs\/tags\/([A-Za-z0-9.-_]+)\.(tar|zip)",
    )
    .ok()?;
    let manual_regex = Regex::new(
        r"\w+\:\/\/github\.com\/([A-Za-z0-9-_]+)\/([A-Za-z0-9-_]+)\/releases\/download\/([A-Za-z0-9-_.]+)\/.*",
    )
    .ok()?;

    for matcher in [automatic_regex, manual_regex] {
        let Some(captures) = matcher.captures(upstream.as_str()) else {
            continue;
        };

        let owner = captures.get(1)?.as_str();
        let project = captures.get(2)?.as_str();
        let version = captures.get(3)?.as_str().to_owned();

        return Some(Source {
            name: project.to_lowercase(),
            version,
            homepage: format!("https://github.com/{owner}/{project}"),
        });
    }

    None
}
