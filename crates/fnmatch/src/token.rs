// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::fmt;

/// Components of a globbed pattern.
/// A valid globbed pattern is composed of a list of `Token`s.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Token {
    /// A string literal in the globbed pattern.
    Text(String),
    /// A glob that matches either a single or multiple characters,
    /// excluding the path separator.
    /// A Glob may have a name, so that it is possible to create
    /// an associative map between the name and the value it resolved into.
    Glob { name: Option<String>, matcher: Matcher },
}

/// Types of globs.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Matcher {
    /// Matches exactly one character, excluding the path separator.
    One,
    /// Matches zero or more characters, excluding the path separator.
    Any,
}

impl From<&RawToken> for Matcher {
    fn from(value: &RawToken) -> Self {
        match value {
            RawToken::MatchOne => Self::One,
            RawToken::MatchAny => Self::Any,
            _ => panic!("unsupported value for Matcher"),
        }
    }
}

/// Parses a globbed pattern string into its components.
pub fn tokens(pattern: impl AsRef<str>) -> Vec<Token> {
    let mut tokens = Vec::new();

    let mut raws = &raw_tokens(pattern)[..];
    while !raws.is_empty() {
        let this_tok = &raws[0];
        match this_tok {
            RawToken::Escape => {
                if let Some(next_tok) = raws.get(1)
                    && next_tok.is_escapable()
                {
                    edit_or_add_text_token(&mut tokens, next_tok.to_string());
                    raws = &raws[2..];
                    continue;
                }
                edit_or_add_text_token(&mut tokens, this_tok.to_string());
            }
            RawToken::GroupOpening => {
                if let Some((name, matcher)) = group_parameters(raws) {
                    tokens.push(Token::Glob {
                        name: Some(name),
                        matcher,
                    });
                    raws = &raws[5..];
                    continue;
                }
                edit_or_add_text_token(&mut tokens, this_tok.to_string());
            }
            RawToken::MatchOne | RawToken::MatchAny => tokens.push(Token::Glob {
                name: None,
                matcher: this_tok.into(),
            }),
            _ => edit_or_add_text_token(&mut tokens, this_tok.to_string()),
        }
        raws = &raws[1..];
    }
    tokens
}

#[derive(Clone, Debug, PartialEq)]
enum RawToken {
    /// Escapes the character that follows.
    Escape,
    /// Any text.
    Text(String),
    /// Opens the named glob.
    GroupOpening,
    /// Separates the glob name from the matcher type.
    GroupSeparator,
    /// Closes the named glob.
    GroupClosing,
    /// Matches any single character.
    MatchOne,
    /// Matches zero or more characters.
    MatchAny,
}

impl TryFrom<char> for RawToken {
    type Error = ();

    fn try_from(value: char) -> Result<Self, Self::Error> {
        Ok(match value {
            '\\' => Self::Escape,
            '(' => Self::GroupOpening,
            ':' => Self::GroupSeparator,
            ')' => Self::GroupClosing,
            '?' => Self::MatchOne,
            '*' => Self::MatchAny,
            _ => return Err(()),
        })
    }
}

impl fmt::Display for RawToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawToken::Escape => write!(f, "\\"),
            RawToken::Text(text) => write!(f, "{text}"),
            RawToken::GroupOpening => write!(f, "("),
            RawToken::GroupSeparator => write!(f, ":"),
            RawToken::GroupClosing => write!(f, ")"),
            RawToken::MatchOne => write!(f, "?"),
            RawToken::MatchAny => write!(f, "*"),
        }
    }
}

impl RawToken {
    fn is_escapable(&self) -> bool {
        !matches!(self, Self::Text(_))
    }
}

fn raw_tokens(s: impl AsRef<str>) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    for c in s.as_ref().chars() {
        if let Ok(token) = RawToken::try_from(c) {
            if !text.is_empty() {
                tokens.push(RawToken::Text(text.clone()));
                text.clear();
            }
            tokens.push(token);
        } else {
            text.push(c);
        }
    }
    if !text.is_empty() {
        tokens.push(RawToken::Text(text.clone()));
    }
    tokens
}

/// Returns the name and the the matcher of a named glob.
/// If the named glob syntax is invalid, it returns None.
fn group_parameters(raws: &[RawToken]) -> Option<(String, Matcher)> {
    if !matches!(raws[0], RawToken::GroupOpening) {
        return None;
    }
    let name;
    if let RawToken::Text(n) = &raws[1] {
        name = n;
    } else {
        return None;
    }
    if !matches!(raws[2], RawToken::GroupSeparator) {
        return None;
    }
    if !matches!(raws[3], RawToken::MatchOne | RawToken::MatchAny) {
        return None;
    }
    Some((name.clone(), Matcher::from(&raws[3])))
}

fn edit_or_add_text_token(tokens: &mut Vec<Token>, s: String) {
    if let Some(Token::Text(txt)) = tokens.last_mut() {
        txt.push_str(&s);
    } else {
        tokens.push(Token::Text(s));
    }
}

#[cfg(test)]
mod raw_token_tests {
    use super::{RawToken, raw_tokens};

    #[test]
    fn tokenize_only_text() {
        let tokens = raw_tokens("/usr/bin/moss");
        assert_eq!(tokens, vec![RawToken::Text("/usr/bin/moss".to_string())])
    }

    #[test]
    fn tokenize_only_control_chars() {
        let tokens = raw_tokens("\\(:*?)");
        assert_eq!(
            tokens,
            vec![
                RawToken::Escape,
                RawToken::GroupOpening,
                RawToken::GroupSeparator,
                RawToken::MatchAny,
                RawToken::MatchOne,
                RawToken::GroupClosing,
            ]
        )
    }

    #[test]
    fn tokenize_mixed_text() {
        let tokens = raw_tokens("/usr/(bindir:*)/moss");
        assert_eq!(
            tokens,
            vec![
                RawToken::Text("/usr/".to_string()),
                RawToken::GroupOpening,
                RawToken::Text("bindir".to_string()),
                RawToken::GroupSeparator,
                RawToken::MatchAny,
                RawToken::GroupClosing,
                RawToken::Text("/moss".to_string()),
            ]
        )
    }
}

#[cfg(test)]
mod token_tests {
    use super::{Matcher, Token, tokens};

    #[test]
    fn tokenize_only_text() {
        let tokens = tokens("/usr/bin/moss");
        assert_eq!(tokens, vec![Token::Text("/usr/bin/moss".to_string())])
    }

    #[test]
    fn tokenize_with_unnamed_middle_glob() {
        let tokens = tokens("/usr/*/moss");
        assert_eq!(
            tokens,
            vec![
                Token::Text("/usr/".to_string()),
                Token::Glob {
                    name: None,
                    matcher: Matcher::Any
                },
                Token::Text("/moss".to_string()),
            ]
        )
    }

    #[test]
    fn tokenize_with_named_middle_glob() {
        let tokens = tokens("/usr/(bindir:*)/moss");
        assert_eq!(
            tokens,
            vec![
                Token::Text("/usr/".to_string()),
                Token::Glob {
                    name: Some("bindir".to_string()),
                    matcher: Matcher::Any
                },
                Token::Text("/moss".to_string()),
            ]
        )
    }

    #[test]
    fn tokenize_with_unnamed_trailing_glob() {
        let tokens = tokens("/usr/moss*");
        assert_eq!(
            tokens,
            vec![
                Token::Text("/usr/moss".to_string()),
                Token::Glob {
                    name: None,
                    matcher: Matcher::Any
                },
            ]
        )
    }

    #[test]
    fn tokenize_with_escaped_middle_glob() {
        let tokens = tokens(r"/usr/\*/moss");
        assert_eq!(tokens, vec![Token::Text(r"/usr/*/moss".to_string()),])
    }

    #[test]
    fn tokenize_with_invalid_escape() {
        let tokens = tokens(r"/usr/\bin/moss\");
        assert_eq!(tokens, vec![Token::Text(r"/usr/\bin/moss\".to_string()),])
    }
}
