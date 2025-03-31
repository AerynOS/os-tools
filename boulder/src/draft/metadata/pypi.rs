// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use url::Url;

use crate::util;

use super::Source;

pub fn source(upstream: &Url) -> Option<Source> {
    let regex = Regex::new(
        r"^https://files\.pythonhosted\.org/packages/[a-f0-9]{2}/[a-f0-9]{2}/[a-f0-9]+/([^/]+)-([\d.]+)\.tar\.gz$",
    )
    .ok()?;

    let filename = util::uri_file_name(upstream);

    let captures = regex.captures(upstream.as_str())?;

    let name = captures.get(1)?.as_str().to_owned();
    let version = captures.get(2)?.as_str().to_owned();

    let first_char = &name.chars().next().unwrap_or_default();

    let pkg_name = if !name.starts_with("python-") {
        format!("python-{name}")
    } else {
        name.to_string()
    };

    Some(Source {
        name: pkg_name,
        version,
        homepage: format!("https://pypi.org/project/{name}"),
        uri: format!("https://files.pythonhosted.org/packages/source/{first_char}/{name}/{filename}"),
    })
}
