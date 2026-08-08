// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use stone_recipe::upstream::{self, SourceUri};

use super::Source;

pub fn source(upstream: &SourceUri) -> Option<Source> {
    // We accept no usernames and ports,
    // just plain access to github.com.
    if upstream.url.authority() != "github.com" {
        return None;
    }
    // We do not support query parameters or fragments in the URL.
    if upstream.url.query().is_some() || upstream.url.fragment().is_some() {
        return None;
    }

    let path_matchers: &[Regex] = match upstream.kind {
        upstream::Kind::Archive => &[
            Regex::new(r"([\w-]+)\/([\w-]+)\/archive\/refs\/tags\/([\w.-]+)\.(?:tar|zip)").unwrap(),
            Regex::new(r"([\w-]+)\/([\w-]+)\/releases\/download\/([\w.-]+)\/.*").unwrap(),
        ],
        upstream::Kind::Git => &[Regex::new(r"([\w-]+)\/([\w-]+)(?:\.git)?").unwrap()],
    };

    for matcher in path_matchers {
        let Some(captures) = matcher.captures(upstream.url.path()) else {
            continue;
        };

        let owner = captures.get(1)?.as_str();
        let project = captures.get(2)?.as_str();
        let version = captures.get(3).map_or("UNDETECTED", |v| v.as_str()).to_owned();

        // Strip 'v' if the second character is a digit e.g. v1.2.3
        let version =
            if version.starts_with('v') && version.len() > 1 && version[1..2].chars().all(|c| c.is_ascii_digit()) {
                version[1..].to_owned()
            } else {
                version
            };

        return Some(Source {
            name: project.to_lowercase(),
            version,
            homepage: format!("https://github.com/{owner}/{project}"),
            uri: upstream.to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn test_automatic_regex() {
        let url_str = "https://github.com/GNOME/pango/archive/refs/tags/1.57.0.tar.gz";
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "pango");
        assert_eq!(source.version, "1.57.0");
        assert_eq!(source.homepage, "https://github.com/GNOME/pango");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_manual_regex() {
        let url_str = "https://github.com/streamlink/streamlink/releases/download/8.2.0/streamlink-8.2.0.tar.gz";
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "streamlink");
        assert_eq!(source.version, "8.2.0");
        assert_eq!(source.homepage, "https://github.com/streamlink/streamlink");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_automatic_regex_with_v_prefix() {
        let url_str = "https://github.com/chatty/chatty/archive/refs/tags/v0.28.tar.gz";
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "chatty");
        assert_eq!(source.version, "0.28");
        assert_eq!(source.homepage, "https://github.com/chatty/chatty");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_manual_regex_string_version() {
        let url_str = "https://github.com/unicode-org/icu/releases/download/release-76-1/icu4c-76_1-src.tgz";
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "icu");
        // TODO: we do not handle string version prefixes
        assert_eq!(source.version, "release-76-1");
        assert_eq!(source.homepage, "https://github.com/unicode-org/icu");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_git_repo() {
        let uri = SourceUri {
            kind: upstream::Kind::Git,
            url: Url::parse("https://github.com/rust-lang/rust.git").unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "rust");
        assert_eq!(source.version, "UNDETECTED");
        assert_eq!(source.homepage, "https://github.com/rust-lang/rust");
        assert_eq!(source.uri, uri.to_string());
    }
}
