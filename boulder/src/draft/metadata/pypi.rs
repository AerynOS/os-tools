// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use moss::util;
use regex::Regex;
use url::Url;

use super::Source;

pub fn source(upstream: &Url) -> Option<Source> {
    let regex = Regex::new(
        r"^https://files\.pythonhosted\.org/packages/[a-f0-9]{2}/[a-f0-9]{2}/[a-f0-9]+/([^/]+)-([\d.]+)\.tar\.gz$",
    )
    .unwrap();

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

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn test_regex_typical_pypi_url() {
        let url_str = "https://files.pythonhosted.org/packages/59/83/a60af4e83c492c7dceceeabd677aa87bbaf2d8910b3d1b973295e560f421/pyzk-0.9.tar.gz";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "python-pyzk");
        assert_eq!(source.version, "0.9");
        assert_eq!(source.homepage, "https://pypi.org/project/pyzk");
        assert_eq!(
            source.uri,
            "https://files.pythonhosted.org/packages/source/p/pyzk/pyzk-0.9.tar.gz"
        );
    }
}
