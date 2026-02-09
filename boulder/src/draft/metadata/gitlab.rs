// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use url::Url;

use super::Source;

pub fn source(upstream: &Url) -> Option<Source> {
    // Attempt to match gitlab.com as well as self-hosted gitlab URLs
    let automatic_regex = Regex::new(
        r"https\:\/\/([a-z0-9\-\.]+)\/([A-Za-z0-9-_]+)\/([A-Za-z0-9-_]+(?:\/[A-Za-z0-9-_]+)?)\/-\/archive\/([A-Za-z0-9\.\-_]+)\/([A-Za-z0-9-_]+)-([A-Za-z0-9\.\-_]+)\.(tar|gz|bz2|xz)"
    )
    .unwrap();

    if let Some(captures) = automatic_regex.captures(upstream.as_str()) {
        let base_url = captures.get(1)?.as_str();

        if !base_url.contains("gitlab") {
            return None;
        }

        let owner = captures.get(2)?.as_str();
        let project = captures.get(3)?.as_str();
        let canonical_project = project.split_once('/').map(|(_, second)| second).unwrap_or(project);
        let version = captures.get(4)?.as_str().to_owned();

        // Strip 'v' if the second character is a digit e.g. v1.2.3
        let version =
            if version.starts_with('v') && version.len() > 1 && version[1..2].chars().all(|c| c.is_ascii_digit()) {
                version[1..].to_owned()
            } else {
                version
            };

        return Some(Source {
            name: canonical_project.to_lowercase(),
            version,
            homepage: format!("https://{base_url}/{owner}/{project}"),
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
    fn test_canonical_gitlab_url() {
        let url_str = "https://gitlab.com/serebit/wraith-master/-/archive/v1.2.1/wraith-master-v1.2.1.tar.bz2";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "wraith-master");
        assert_eq!(source.version, "1.2.1");
        assert_eq!(source.homepage, "https://gitlab.com/serebit/wraith-master");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_self_hosted_gitlab_url_1() {
        let url_str = "https://gitlab.gnome.org/GNOME/pango/-/archive/1.57.0/pango-1.57.0.tar.gz";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "pango");
        assert_eq!(source.version, "1.57.0");
        assert_eq!(source.homepage, "https://gitlab.gnome.org/GNOME/pango");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_self_hosted_gitlab_url_2() {
        let url_str = "https://gitlab.freedesktop.org/serebit/waycheck/-/archive/v1.7.0/waycheck-v1.7.0.tar";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "waycheck");
        assert_eq!(source.version, "1.7.0");
        assert_eq!(source.homepage, "https://gitlab.freedesktop.org/serebit/waycheck");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_self_hosted_gitlab_url_3() {
        let url_str = "https://gitlab.freedesktop.org/xkeyboard-config/xkeyboard-config/-/archive/xkeyboard-config-2.46/xkeyboard-config-xkeyboard-config-2.46.tar.gz";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "xkeyboard-config");
        // TODO: we do not handle the case where the project name is part of the version
        assert_eq!(source.version, "xkeyboard-config-2.46");
        assert_eq!(
            source.homepage,
            "https://gitlab.freedesktop.org/xkeyboard-config/xkeyboard-config"
        );
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_subproject_in_selfhosted_url() {
        let url_str =
            "https://gitlab.archlinux.org/archlinux/mkinitcpio/mkinitcpio/-/archive/v40/mkinitcpio-v40.tar.bz2";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "mkinitcpio");
        assert_eq!(source.version, "40");
        assert_eq!(
            source.homepage,
            "https://gitlab.archlinux.org/archlinux/mkinitcpio/mkinitcpio"
        );
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_version_with_leading_v() {
        let url_str = "https://gitlab.com/serebit/wraith-master/-/archive/v1.2.1/wraith-master-v1.2.1.tar.gz";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "wraith-master");
        assert_eq!(source.version, "1.2.1");
        assert_eq!(source.homepage, "https://gitlab.com/serebit/wraith-master");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_url_without_version_prefix() {
        let url_str = "https://gitlab.com/serebit/wraith-master/-/archive/1.2.1/wraith-master-1.2.1.tar.gz";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "wraith-master");
        assert_eq!(source.version, "1.2.1");
        assert_eq!(source.homepage, "https://gitlab.com/serebit/wraith-master");
        assert_eq!(source.uri, url_str);
    }

    #[test]
    fn test_avoid_github_url_match() {
        let url_str = "https://github.com/GNOME/pango/archive/refs/tags/1.57.0.tar.gz";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_none());
    }

    #[test]
    fn test_invalid_url() {
        let url_str = "https://invalid-url.com";
        let url = Url::parse(url_str).unwrap();

        let source = source(&url);
        assert!(source.is_none());
    }
}
