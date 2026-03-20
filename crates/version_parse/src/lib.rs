// SPDX-FileCopyrightText: Copyright © 2025 AerynOS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Version pattern extraction library for extracting version numbers and project names from file paths and URLs.
//!
//! This crate provides functionality to parse version numbers and project names from various file naming patterns,
//! supporting semantic versions, date-based versions, release series versioning, and other common versioning schemes.
//!
//! # Examples
//! ```
//! use version_parse::{VersionExtractor, Extraction};
//! let extractor = VersionExtractor::new();
//! let result = extractor.extract("myproject-1.2.3.tar.gz")?;
//! assert_eq!(result.name, "myproject");
//! assert_eq!(result.version, "1.2.3");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use regex::Regex;
use snafu::Snafu;
use url::Url;

/// Represents different versioning styles that can be extracted
#[derive(Debug, Clone, PartialEq)]
pub enum VersionStyle {
    /// Semantic versioning pattern (e.g. 1.2.3)
    Semver,
    /// Date-based version (e.g. YYYYMMDD)
    DateBased,
    /// Release series versioning (e.g. 3.24.33)
    ReleaseSeries,
    /// Simple version number (e.g. 46.1)
    Simple,
}

/// Pattern definition for version extraction
pub struct VersionPattern {
    /// The style of versioning this pattern matches
    pub style: VersionStyle,
    /// Pattern for extracting name and version
    pub pattern: Regex,
    /// Priority for matching (lower = tried first)
    pub priority: u8,
}

impl VersionPattern {
    /// Creates a new version pattern
    ///
    /// # Arguments
    /// * `style` - The version style this pattern matches
    /// * `pattern` - Regular expression pattern string
    /// * `priority` - Priority for matching (lower = tried first)
    pub fn new(style: VersionStyle, pattern: &str, priority: u8) -> Result<Self, regex::Error> {
        Ok(Self {
            style,
            pattern: Regex::new(pattern)?,
            priority,
        })
    }
}

/// Version extraction engine that matches patterns against paths/URLs
pub struct VersionExtractor {
    patterns: Vec<VersionPattern>,
}

/// Errors that can occur during version extraction
#[derive(Debug, Snafu)]
pub enum VersionError {
    /// No valid version could be extracted from the path
    #[snafu(display("No version found in path: {path}"))]
    InvalidVersion { path: String },
}

impl Default for VersionExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl VersionExtractor {
    /// Creates a new version extractor with default patterns
    pub fn new() -> Self {
        let mut extractor = Self {
            patterns: Vec::with_capacity(5),
        };
        extractor.add_default_patterns();
        extractor
    }

    /// Adds a custom pattern to the extractor
    ///
    /// Patterns are tried in order of priority (lowest first)
    pub fn add_pattern(&mut self, pattern: VersionPattern) {
        self.patterns.push(pattern);
        self.patterns.sort_by_key(|p| p.priority);
    }

    /// Initialize with default known patterns
    fn add_default_patterns(&mut self) {
        let patterns = vec![
            VersionPattern::new(
                VersionStyle::DateBased,
                r"(?x)
                    (?P<name>[^/]+)
                    [-_]
                    v?(?P<version>\d{8}(?:[-]\d+\.\d+)?)
                    (?:\.(?:tar(?:\.[^/]*)?|zip|tgz))?$
                ",
                5,
            )
            .unwrap(),
            VersionPattern::new(
                VersionStyle::Semver,
                r"(?x)
                    (?P<name>[^/]+)
                    [-_]
                    v?(?P<version>(?:\d+[._]\d+[._]\d+
                        (?:[-.](?:rc|alpha|beta|dev|pre|post|build|\d+))*
                    ))
                    (?:\.(?:tar(?:\.[^/]*)?|zip|tgz))?$
                ",
                10,
            )
            .unwrap(),
            VersionPattern::new(
                VersionStyle::DateBased,
                r"(?x)
                    (?P<name>[^/]+)
                    [-_]
                    v?(?P<version>\d{4}[._]\d{2}[._]\d{2})
                    (?:[-_.][\d.]+)?  # Optional version suffix
                    (?:\.(?:tar(?:\.[^/]*)?|zip|tgz))?$
                ",
                25,
            )
            .unwrap(),
            VersionPattern::new(
                VersionStyle::Simple,
                r"(?x)
                    (?P<name>[^/]+)
                    [-_]
                    v?(?P<version>\d+\.\d+)
                    (?:\.(?:tar(?:\.[^/]*)?|zip|tgz))?$
                ",
                30,
            )
            .unwrap(),
            VersionPattern::new(
                VersionStyle::Simple,
                r"(?x)
                    (?P<name>[^/]+)
                    [-_]
                    v?(?P<version>\d+)
                    (?:\.(?:tar(?:\.[^/]*)?|zip|tgz))?$
                ",
                35,
            )
            .unwrap(),
            VersionPattern::new(
                VersionStyle::Simple,
                r"(?x)
                    (?P<name>.*?)
                    [-]
                    (?P<version>[^-/]+?)
                    (?:\.(?:tar(?:\.[^/]*)?|zip|tgz)|\.[\w]+)?$
                ",
                100,
            )
            .unwrap(),
        ];

        self.patterns = patterns;
        self.patterns.sort_by_key(|p| p.priority);
    }

    /// Extracts version and name information from a path or URL
    ///
    /// # Arguments
    /// * `path` - Path or URL to extract version info from
    ///
    /// # Returns
    /// * `Ok(Extraction)` containing name and version if successful
    /// * `Err(VersionError)` if no version could be extracted
    pub fn extract(&self, path: &str) -> Result<Extraction, VersionError> {
        if let Some(result) = self.try_extract_vcs_url(path) {
            return result;
        }

        if let Some(filename) = path.split('/').next_back() {
            for pattern in &self.patterns {
                if let Some(caps) = pattern.pattern.captures(filename)
                    && let (Some(name), Some(version)) = (caps.name("name"), caps.name("version"))
                {
                    return Ok(Extraction {
                        name: name.as_str().to_owned(),
                        version: version.as_str().to_owned(),
                    });
                }
            }
        }

        Err(VersionError::InvalidVersion { path: path.to_owned() })
    }

    /// Attempts to extract version info from GitHub/GitLab URLs
    fn try_extract_vcs_url(&self, path: &str) -> Option<Result<Extraction, VersionError>> {
        if !path.contains("github.com") && !path.contains("gitlab.com") {
            return None;
        }

        let url = Url::parse(path).ok()?;

        match url.host_str() {
            Some("github.com") if url.path().contains("archive/refs/tags/") => {
                let parts: Vec<&str> = url.path().split('/').collect();
                let project = parts.get(2)?;
                let asset = parts.last()?;
                let faux = format!("{project}-{asset}");
                Some(self.extract(&faux).map(|matched| Extraction {
                    name: project.to_string(),
                    ..matched
                }))
            }
            Some("gitlab.com") if url.path().contains("repository/archive.tar.gz") => {
                let parts: Vec<&str> = url.path().split('/').collect();
                let project = parts.get(2)?;
                let faux = format!("{project}-archive.tar.gz");
                Some(self.extract(&faux).map(|matched| Extraction {
                    name: project.to_string(),
                    ..matched
                }))
            }
            _ => None,
        }
    }
}

/// Holds the extracted version information
#[derive(Debug, PartialEq)]
pub struct Extraction {
    /// Project/package name
    pub name: String,
    /// Version string
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract() {
        let known_good = vec![
            (
                "https://download.gnome.org/sources/NetworkManager/1.50/NetworkManager-1.50.0.tar.xz",
                Extraction {
                    version: "1.50.0".to_string(),
                    name: "NetworkManager".to_string(),
                },
            ),
            (
                "https://github.com/cli/cli/archive/refs/tags/v2.63.2.tar.gz",
                Extraction {
                    version: "2.63.2".to_string(),
                    name: "cli".to_string(),
                },
            ),
            (
                "https://www.x.org/pub/individual/xserver/xwayland-24.1.4.tar.xz",
                Extraction {
                    version: "24.1.4".to_string(),
                    name: "xwayland".to_string(),
                },
            ),
            (
                "https://download.gnome.org/sources/gtk+/3.24/gtk+-3.24.33.tar.xz",
                Extraction {
                    version: "3.24.33".to_string(),
                    name: "gtk+".to_string(),
                },
            ),
            (
                "https://www.nano-editor.org/dist/v8/nano-8.3.tar.xz",
                Extraction {
                    version: "8.3".to_string(),
                    name: "nano".to_string(),
                },
            ),
            (
                "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.13.4.tar.xz",
                Extraction {
                    version: "6.13.4".to_string(),
                    name: "linux".to_string(),
                },
            ),
            (
                "https://github.com/intel/Intel-Linux-Processor-Microcode-Data-Files/archive/refs/tags/microcode-20250211.tar.gz",
                Extraction {
                    version: "20250211".to_string(),
                    name: "Intel-Linux-Processor-Microcode-Data-Files".to_string(),
                },
            ),
            (
                "https://download.gnome.org/sources/gnome-disk-utility/46/gnome-disk-utility-46.1.tar.xz",
                Extraction {
                    version: "46.1".to_string(),
                    name: "gnome-disk-utility".to_string(),
                },
            ),
            (
                "https://thrysoee.dk/editline/libedit-20221030-3.1.tar.gz",
                Extraction {
                    version: "20221030-3.1".to_string(),
                    name: "libedit".to_string(),
                },
            ),
            (
                "https://www.sudo.ws/dist/sudo-1.9.16p2.tar.gz",
                Extraction {
                    version: "1.9.16p2".to_string(),
                    name: "sudo".to_string(),
                },
            ),
            (
                "https://download.nvidia.com/XFree86/nvidia-persistenced/nvidia-persistenced-570.86.16.tar.bz2",
                Extraction {
                    version: "570.86.16".to_string(),
                    name: "nvidia-persistenced".to_string(),
                },
            ),
            (
                "https://us.download.nvidia.com/XFree86/Linux-x86_64/570.86.16/NVIDIA-Linux-x86_64-570.86.16.run",
                Extraction {
                    version: "570.86.16".to_string(),
                    name: "NVIDIA-Linux-x86_64".to_string(),
                },
            ),
            (
                "https://github.com/pop-os/cosmic-applets/archive/refs/tags/epoch-1.0.0-alpha.6.tar.gz",
                Extraction {
                    version: "1.0.0-alpha.6".to_string(),
                    name: "cosmic-applets".to_string(),
                },
            ),
        ];

        let extractor = VersionExtractor::new();
        for (path, expected) in known_good {
            eprintln!("Testing path: {}", path);
            let result = extractor.extract(path).expect("Failed to extract version");
            eprintln!("Expected: {:?}, got: {:?}", expected, result);
            assert_eq!(result, expected);
        }
    }
}
