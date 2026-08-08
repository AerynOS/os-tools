// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use regex::Regex;
use stone_recipe::upstream::{self, SourceUri};

use super::Source;

pub fn source(upstream: &SourceUri) -> Option<Source> {
    // We only support anonymous access.
    if upstream.url.username() != "" {
        return None;
    }
    // Attempt to match gitlab.com as well as self-hosted gitlab URLs
    if !upstream.url.authority().contains("gitlab") {
        return None;
    }
    // We do not support query parameters or fragments in the URL.
    if upstream.url.query().is_some() || upstream.url.fragment().is_some() {
        return None;
    }

    let mut path = upstream.url.path();
    let path_matcher = match upstream.kind {
        upstream::Kind::Archive => Regex::new(
            r"([\w-]+)\/([\w.-]+(?:\/[\w.-]+)?)\/-\/archive\/([\w.-]+)\/([\w-]+)-([\w.-]+)\.(?:tar|gz|bz2|xz)",
        )
        .unwrap(),
        upstream::Kind::Git => {
            path = upstream.url.path().strip_suffix(".git").unwrap_or(upstream.url.path());
            Regex::new(r"([\w-]+)\/([\w.-]+(?:\/[\w.-]+)?)").unwrap()
        }
    };

    if let Some(captures) = path_matcher.captures(path) {
        let owner = captures.get(1)?.as_str();
        let project = captures.get(2)?.as_str();
        let canonical_project = project.split_once('/').map(|(_, second)| second).unwrap_or(project);
        let version = captures.get(3).map_or("UNDETECTED", |v| v.as_str()).to_owned();

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
            homepage: format!("https://{}/{owner}/{project}", upstream.url.authority()),
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };
        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
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
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_none());
    }

    #[test]
    fn test_invalid_url() {
        let url_str = "https://invalid-url.com";
        let uri = SourceUri {
            kind: upstream::Kind::Archive,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_none());
    }

    #[test]
    fn test_git_repo() {
        let url_str = "https://gitlab.freedesktop.org/mesa/mesa3d.org.git";
        let uri = SourceUri {
            kind: upstream::Kind::Git,
            url: Url::parse(url_str).unwrap(),
        };

        let source = source(&uri);
        assert!(source.is_some());

        let source = source.unwrap();
        assert_eq!(source.name, "mesa3d.org");
        assert_eq!(source.version, "UNDETECTED");
        assert_eq!(source.homepage, "https://gitlab.freedesktop.org/mesa/mesa3d.org");
        assert_eq!(source.uri, uri.to_string());
    }
}
