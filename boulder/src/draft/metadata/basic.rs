// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use url::Url;

use crate::util;

use super::Source;

pub fn source(upstream: &Url) -> Option<Source> {
    let filename = util::uri_file_name(upstream);

    let regex = Regex::new(r"^([a-zA-Z0-9-]+)-([a-zA-Z0-9._-]+)\.(zip|tar|sh|bin\.*)").ok()?;
    let captures = regex.captures(filename)?;

    let name = captures.get(1)?.as_str().to_owned();
    let version = captures.get(2)?.as_str().to_owned();

    let (homepage, _) = upstream.as_str().rsplit_once('/')?;

    Some(Source {
        name,
        version,
        homepage: homepage.to_owned(),
    })
}
