// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Use fnmatch to generate regex matchers from glob-style strings.
//!
//! This crate extends the conventional `glob` style strings to add matching groups
//! by compiling to an internal [Regex].
//!
//!
//! # Example
//! ```
//!     let pattern = "/usr/lib/modules/(version:*)/kernel".parse::<fnmatch::Pattern>().unwrap();
//!     let result = pattern.match_path("/usr/lib/modules/6.2.3/kernel").expect("no kernel match");
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use regex::Regex;
use serde::{Deserialize, de};
use thiserror::Error;

#[derive(Debug)]
enum Fragment {
    /// `?`
    MatchOne,

    /// `*`
    MatchAny,

    /// `\`
    BackSlash,

    /// `.`
    Dot,

    /// `/
    ForwardSlash,

    /// Normal text.
    Text(String),

    /// Group: Name to fragment mapping
    Group(String, Vec<Fragment>),
}

#[derive(Clone)]
struct StringWalker<'a> {
    data: &'a str,
    index: usize,
    length: usize,
}

impl Iterator for StringWalker<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.length {
            None
        } else {
            self.index += 1;
            self.data.get(self.index - 1..self.index).and_then(|s| s.chars().nth(0))
        }
    }
}

impl<'a> StringWalker<'a> {
    /// Return a new StringWalker
    pub fn new(data: &'a str) -> Self {
        Self {
            data,
            index: 0,
            length: data.len(),
        }
    }

    pub fn eat(&mut self, much: usize) {
        self.index += much;
    }

    /// Find next occurrence of the character, and substring up to it
    pub fn substring_to(&self, c: char) -> Option<&'a str> {
        // Clone ourselves and search that iterator.
        let walker = self.clone();
        for (idx, t) in walker.enumerate() {
            if t == c {
                return self.data.get(self.index..self.index + idx);
            }
        }
        None
    }
}

/// Glob-style matching with groups
///
/// You can generate a Pattern by converting from a [String] or string-type
/// using the [FromStr] trait.
#[derive(Debug, Clone)]
pub struct Pattern {
    pattern: String,
    regex: Regex,
    groups: Vec<String>,
}

/// Path match for a [Pattern]
///
/// A Match contains the matching path as well as any captured
/// variables, as determined by the [Pattern]
///
/// Generate a Match by calling [`Pattern::match_path()`]
#[derive(Debug)]
pub struct Match {
    pub path: String,

    /// Captured variables, as defined by the [`Pattern::groups()`]
    pub variables: BTreeMap<String, String>,
}

impl Pattern {
    /// Attempt to match `path` to our `pattern`
    ///
    /// Returns a [Match] if the input path matches the pattern
    pub fn match_path(&self, path: &str) -> Option<Match> {
        match self.regex.captures(path) {
            Some(m) => {
                let kv = self
                    .groups
                    .iter()
                    .map(|k| (k.clone(), m.name(k).unwrap().as_str().to_owned()));
                Some(Match {
                    path: path.into(),
                    variables: kv.collect(),
                })
            }
            None => None,
        }
    }

    /// Return a copy of the internal capture groups
    pub fn groups(&self) -> Vec<String> {
        self.groups.clone()
    }
}

/// [thiserror] compatible Error
#[derive(Error, Debug)]
pub enum Error {
    /// Illegal group syntax
    #[error("malformed group")]
    Group,

    /// Illegal regex
    #[error("invalid regex: {0}")]
    Regex(#[from] regex::Error),
}

fn fragments_from_string(s: &str) -> Result<Vec<Fragment>, Error> {
    let mut walker = StringWalker::new(s);
    let mut builder = vec![];
    let mut text = String::new();
    while let Some(ch) = walker.next() {
        let next_token = match ch {
            '?' => Some(Fragment::MatchOne),
            '*' => Some(Fragment::MatchAny),
            '\\' => Some(Fragment::BackSlash),
            '/' => Some(Fragment::ForwardSlash),
            '.' => Some(Fragment::Dot),
            '(' => {
                let Some(end) = walker.substring_to(')') else {
                    return Err(Error::Group);
                };

                walker.eat(end.len() + 1);

                let splits = end.split(':').collect::<Vec<_>>();
                if splits.len() != 2 {
                    return Err(Error::Group);
                }
                let key = *splits.first().ok_or(Error::Group)?;
                let value = *splits.get(1).ok_or(Error::Group)?;

                let subpattern = fragments_from_string(value)?;
                builder.push(Fragment::Group(key.to_owned(), subpattern));

                None
            }
            ')' => None,
            _ => {
                text.push(ch);
                None
            }
        };

        if let Some(token) = next_token {
            if !text.is_empty() {
                builder.push(Fragment::Text(text.clone()));
                text.clear();
            }
            builder.push(token);
        }
    }

    if !text.is_empty() {
        builder.push(Fragment::Text(text.clone()));
    }

    Ok(builder)
}

fn fragment_to_regex_str(fragment: &Fragment) -> (String, Vec<String>) {
    let mut groups = vec![];
    let string = match fragment {
        Fragment::MatchOne => ".".into(),
        Fragment::MatchAny => "[^\\/]*".into(),
        Fragment::BackSlash => "\\".into(),
        Fragment::ForwardSlash => "\\/".into(),
        Fragment::Dot => "\\.".into(),
        Fragment::Text(t) => t.clone(),
        Fragment::Group(id, elements) => {
            let elements = elements
                .iter()
                .map(|m| {
                    let (s, g) = fragment_to_regex_str(m);
                    groups.extend(g);
                    s
                })
                .collect::<String>();
            groups.push(id.clone());
            format!("(?<{id}>{elements})")
        }
    };
    (string, groups)
}

impl FromStr for Pattern {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let fragments = fragments_from_string(s)?;
        let mut groups = BTreeSet::new();

        let compiled = fragments
            .iter()
            .map(|m| {
                let (s, g) = fragment_to_regex_str(m);
                groups.extend(g);
                s
            })
            .collect::<String>();

        Ok(Self {
            pattern: s.into(),
            regex: Regex::new(&format!("^{compiled}$"))?,
            groups: groups.into_iter().collect(),
        })
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.groups == other.groups
    }
}

impl Eq for Pattern {}

impl PartialOrd for Pattern {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pattern {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.pattern.cmp(&other.pattern) {
            std::cmp::Ordering::Less => return std::cmp::Ordering::Less,
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => return std::cmp::Ordering::Greater,
        }
        self.groups.cmp(&other.groups)
    }
}

#[cfg(test)]
pub mod path_tests {
    use super::Pattern;

    /// test me
    #[test]
    fn test_pattern() {
        let k = "/usr/lib/modules/(version:*)/*".parse::<Pattern>().unwrap();

        let good = k.match_path("/usr/lib/modules/6.2.6/modules.symbols");
        assert!(good.is_some());
        let m = good.unwrap();
        assert_eq!(m.path, "/usr/lib/modules/6.2.6/modules.symbols");
        let version = m.variables.get("version");
        assert!(version.is_some());
        assert_eq!(version.unwrap(), "6.2.6");

        let wide = k.match_path("/usr/lib/modules/6.6.67-51.kvm/kernel/net/netfilter/nft_hash.ko.zst");
        assert!(wide.is_none());
    }
}
