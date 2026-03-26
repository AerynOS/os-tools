// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashMap, convert, path::MAIN_SEPARATOR, str::FromStr};

use serde_core::de;

use crate::token::{Matcher, Token, tokens};

/// A globbed pattern that matches file paths.
///
/// Within the path string the "?" and "*" characters can
/// be used to match, respectively, exactly one and zero or more characters.
/// Additionally, the "(groupname:selector)" syntax can be used to extend the aforementionted
/// matchers with a name. If there is a match, the name will appear in [Match] in association
/// with the value it resolved into. "groupname" can be any string; "selector" is one of the two
/// supported matchers.
///
/// The matchers and the the characters used in the group syntax can be escaped with a backslash ("\\").
///
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Pattern {
    tokens: Vec<Token>,
}

impl FromStr for Pattern {
    type Err = convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self { tokens: tokens(s) })
    }
}

impl<'de> de::Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde_core::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl Pattern {
    /// Creates a new Pattern. Equivalent to `s.parse::<Pattern>()`.
    pub fn new(s: impl AsRef<str>) -> Self {
        s.as_ref().parse().unwrap()
    }

    /// Tries to match the pattern with a filepath.
    /// If there is no match, None is returned.
    pub fn matches(&self, path: impl AsRef<str>) -> Option<Match> {
        let mut matc = Match::default();

        let mut path_walk = path.as_ref();
        let mut tokens = self.tokens.iter().peekable();
        while let Some(tok) = tokens.next() {
            match tok {
                Token::Text(txt) => {
                    if !match_text(&mut path_walk, txt) {
                        return None;
                    }
                }
                Token::Glob { name, matcher } => match matcher {
                    Matcher::One => {
                        if !match_glob_one(&mut path_walk, name, &mut matc) {
                            return None;
                        }
                    }
                    Matcher::Any => {
                        if !match_glob_any(&mut path_walk, name, tokens.peek(), &mut matc) {
                            return None;
                        }
                    }
                },
            }
        }
        matc.path = path.as_ref().to_owned();
        Some(matc)
    }
}

/// Path match for a [Pattern].
///
/// A Match contains the matching path as well as any captured
/// variables, as determined by the [Pattern].
///
/// Generate a Match by calling [`Pattern::matches()`].
#[derive(Debug, Default)]
pub struct Match {
    /// Path that matched the pattern.
    pub path: String,
    /// Named groups with the value they resolved into.
    pub groups: HashMap<String, String>,
}

fn match_text(path: &mut &str, txt: &str) -> bool {
    if !path.starts_with(txt) {
        return false;
    }
    *path = &path[txt.len()..];
    true
}

fn match_glob_one(path: &mut &str, group_name: &Option<String>, matc: &mut Match) -> bool {
    if let Some(next_char) = path.chars().next() {
        if next_char == MAIN_SEPARATOR {
            return false;
        }
        if let Some(group_name) = group_name {
            matc.groups.insert(group_name.to_string(), next_char.to_string());
        }
        *path = &path[1..];
        return true;
    }
    false
}

fn match_glob_any(path: &mut &str, group_name: &Option<String>, next_tok: Option<&&Token>, matc: &mut Match) -> bool {
    let matched_string;
    if next_tok.is_some() {
        if let Some(filename) = substring_to(path, MAIN_SEPARATOR) {
            matched_string = filename;
            *path = &path[filename.len()..];
        } else {
            return false;
        }
    } else {
        matched_string = *path;
        *path = &path[path.len()..];
    }

    if let Some(group_name) = group_name {
        matc.groups.insert(group_name.to_string(), matched_string.to_owned());
    }
    true
}

fn substring_to(s: &str, divider: char) -> Option<&str> {
    let index = s.find(divider)?;
    Some(&s[..index])
}

#[cfg(test)]
mod single_glob_tests {
    use super::Pattern;

    #[test]
    fn pattern_doesnt_match_literal() {
        let p = Pattern::new("/usr/bin");
        assert!(p.matches("/usr/lib64").is_none());
    }

    #[test]
    fn pattern_matches_literal() {
        let p = Pattern::new("/usr/bin");
        assert!(p.matches("/usr/bin").is_some());
    }

    #[test]
    /// "?" must not match the path separator.
    fn pattern_fails_with_glob_one_separator() {
        let p = Pattern::new("?usr/bin/moss");
        assert!(p.matches("/usr/bin/moss").is_none());
    }

    #[test]
    fn pattern_doesnt_match_with_glob_one_separator() {
        let p = Pattern::new("/usr/bin?/moss");
        assert!(p.matches("/usr/bin/moss").is_none());
    }

    #[test]
    fn pattern_matches_with_glob_one_separator() {
        let p = Pattern::new("/usr/bin/mos?");
        assert!(p.matches("/usr/bin/moss").is_some());
    }

    #[test]
    /// "*" must not match the path separator.
    fn pattern_fails_with_glob_any_separator() {
        let p = Pattern::new("*usr/bin/moss");
        assert!(p.matches("/usr/bin/moss").is_none());
    }

    #[test]
    fn pattern_doesnt_match_with_glob_any() {
        let p = Pattern::new("/usr/b*/moss");
        assert!(p.matches("/usr/sbin/moss").is_none());
    }

    #[test]
    fn pattern_matches_with_glob_any() {
        let p = Pattern::new("/usr/*/moss");
        assert!(p.matches("/usr/bin/moss").is_some());
    }

    #[test]
    fn pattern_matches_with_trailing_glob_any() {
        let p = Pattern::new("/usr/bin/moss*");
        assert!(p.matches("/usr/bin/moss").is_some());
    }

    #[test]
    fn pattern_matches_group_partial_filename() {
        let p = Pattern::new("/usr/b(partname:*)/moss");
        let vars = p.matches("/usr/bin/moss").unwrap().groups;
        assert!(vars.get("partname").is_some_and(|value| value == "in"));
    }

    #[test]
    fn pattern_matches_group_whole_filename() {
        let p = Pattern::new("/usr/(bindir:*)/moss");
        let vars = p.matches("/usr/bin/moss").unwrap().groups;
        assert!(vars.get("bindir").is_some_and(|value| value == "bin"));
    }
}

#[cfg(test)]
mod multiple_globs_tests {
    use super::Pattern;

    #[test]
    fn pattern_doesnt_match_two_glob_ones() {
        let p = Pattern::new("/us?/bin?/moss");
        assert!(p.matches("/usr/bin/moss").is_none());
    }

    #[test]
    fn pattern_matches_two_glob_ones() {
        let p = Pattern::new("/us?/bi?/moss");
        assert!(p.matches("/usr/bin/moss").is_some());
    }

    #[test]
    fn pattern_doesnt_match_two_glob_anys() {
        let p = Pattern::new("/usr/s*/mos*");
        assert!(p.matches("/usr/bin/moss").is_none());
    }

    #[test]
    fn pattern_matches_two_glob_anys() {
        let p = Pattern::new("/usr/*/mos*");
        assert!(p.matches("/usr/bin/moss").is_some());
    }
}
