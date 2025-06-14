// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::map_collect_result_unit)]

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, anychar, char, digit1, newline},
    combinator::{eof, iterator, map, peek, recognize, value},
    multi::{many_till, many1},
    sequence::{delimited, preceded, terminated},
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

use crate::{Macros, macros::Action};

#[derive(Default)]
pub struct Parser {
    actions: BTreeMap<String, Action>,
    definitions: BTreeMap<String, String>,
    env: Option<String>,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Env is parsed and prependend to the beginning of each
    /// [`Command::Content`]
    pub fn env(self, env: impl ToString) -> Self {
        Self {
            env: Some(env.to_string()),
            ..self
        }
    }

    pub fn add_action(&mut self, identifier: impl ToString, action: Action) {
        self.actions.insert(identifier.to_string(), action);
    }

    pub fn add_definition(&mut self, identifier: impl ToString, definition: impl ToString) {
        self.definitions.insert(identifier.to_string(), definition.to_string());
    }

    pub fn add_macros(&mut self, macros: Macros) {
        for kv in macros.actions {
            self.add_action(kv.key, kv.value);
        }
        for kv in macros.definitions {
            self.add_definition(kv.key, kv.value);
        }
    }

    pub fn parse(&self, input: &str) -> Result<Script, Error> {
        let mut dependencies = BTreeSet::new();

        let Parsed { commands, env } = parse(
            input,
            self.env.as_deref(),
            &self.actions,
            &self.definitions,
            &mut dependencies,
        )?;

        let resolved_actions = self
            .actions
            .iter()
            .filter_map(|(identifier, action)| {
                let result =
                    parse_content_only(&action.command, &self.actions, &self.definitions, &mut BTreeSet::new())
                        .transpose()?;

                Some(result.map(|resolved| (identifier.clone(), resolved)))
            })
            .collect::<Result<_, _>>()?;
        let resolved_definitions = self
            .definitions
            .iter()
            .filter_map(|(identifier, definition)| {
                let result = parse_content_only(definition, &self.actions, &self.definitions, &mut BTreeSet::new())
                    .transpose()?;

                Some(result.map(|resolved| (identifier.clone(), resolved)))
            })
            .collect::<Result<_, _>>()?;

        Ok(Script {
            commands,
            env,
            dependencies: dependencies.into_iter().collect(),
            resolved_actions,
            resolved_definitions,
        })
    }

    pub fn parse_content(&self, input: &str) -> Result<String, Error> {
        parse_content_only(input, &self.actions, &self.definitions, &mut Default::default())
            .map(Option::unwrap_or_default)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Content to execute
    Content(String),
    /// Breakpoint
    Break(Breakpoint),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breakpoint {
    pub line_num: usize,
    pub exit: bool,
}

#[derive(Debug)]
pub struct Script {
    pub commands: Vec<Command>,
    pub dependencies: Vec<String>,
    pub env: Option<String>,
    /// Fully resolved actions
    pub resolved_actions: BTreeMap<String, String>,
    /// Fully resolved definitions
    pub resolved_definitions: BTreeMap<String, String>,
}

struct Parsed {
    commands: Vec<Command>,
    env: Option<String>,
}

fn parse(
    input: &str,
    env: Option<&str>,
    actions: &BTreeMap<String, Action>,
    definitions: &BTreeMap<String, String>,
    dependencies: &mut BTreeSet<String>,
) -> Result<Parsed, Error> {
    let mut line_num = 0;
    let mut content = String::new();
    let mut commands = vec![];

    // Parse `env` since it can contain macros
    let env = env
        .map(|env| parse_content_only(env, actions, definitions, dependencies))
        .transpose()?
        .flatten();

    // Prepend env to content
    let prepend_env = |content: &str| {
        format!(
            "{}{content}",
            env.as_ref().map(|env| format!("{env}\n")).unwrap_or_default()
        )
        .trim()
        .to_owned()
    };

    tokens(input, |token| {
        match token {
            Token::Action(identifier) => {
                let action = actions
                    .get(identifier)
                    .ok_or(Error::UnknownAction(identifier.to_owned()))?;
                dependencies.extend(action.dependencies.clone());

                if let Some(nested) = parse_content_only(&action.command, actions, definitions, dependencies)? {
                    content.push_str(&nested);
                }
            }
            Token::Definition(identifier) => {
                let definition = definitions
                    .get(identifier)
                    .ok_or(Error::UnknownDefinition(identifier.to_owned()))?;

                if let Some(nested) = parse_content_only(definition, actions, definitions, dependencies)? {
                    content.push_str(&nested);
                }
            }
            Token::Plain(plain) => content.push_str(plain),
            Token::Newline => {
                line_num += 1;
                content.push('\n');
            }
            Token::Break { exit } => {
                let content = prepend_env(&std::mem::take(&mut content));
                if !content.is_empty() {
                    commands.push(Command::Content(content));
                }
                commands.push(Command::Break(Breakpoint { line_num, exit }));
            }
        }
        Ok(())
    })?;

    let content = prepend_env(&content);
    if !content.is_empty() {
        commands.push(Command::Content(content));
    }

    Ok(Parsed { commands, env })
}

/// Extract the `parse` call as content only, used for parsing nested macros
fn parse_content_only(
    input: &str,
    actions: &BTreeMap<String, Action>,
    definitions: &BTreeMap<String, String>,
    dependencies: &mut BTreeSet<String>,
) -> Result<Option<String>, Error> {
    Ok(parse(input, None, actions, definitions, dependencies)?
        .commands
        .into_iter()
        .next()
        .and_then(|command| {
            if let Command::Content(content) = command {
                Some(content)
            } else {
                None
            }
        }))
}

#[derive(Debug)]
enum Token<'a> {
    Action(&'a str),
    Definition(&'a str),
    Plain(&'a str),
    Newline,
    Break { exit: bool },
}

fn tokens(input: &str, f: impl FnMut(Token<'_>) -> Result<(), Error>) -> Result<(), Error> {
    // A-Za-z0-9_
    let identifier = |input| recognize(many1(alt((alpha1, digit1, tag("_")))))(input);
    // %identifier
    let action = |input| preceded(char('%'), identifier)(input);
    // %(identifier)
    let definition = |input| preceded(char('%'), delimited(char('('), identifier, char(')')))(input);
    // action or definition
    let macro_ = alt((action, definition));
    // %% -> %
    let escaped = |input| preceded(char('%'), value("%", char('%')))(input);
    // Escaped or any char until newline, escape, next macro or EOF
    let plain = alt((
        escaped,
        recognize(many_till(anychar, peek(alt((recognize(newline), escaped, macro_))))),
        recognize(terminated(many1(anychar), eof)),
    ));

    let token = alt((
        map(newline, |_| Token::Newline),
        map(action, |action| match action {
            "break_continue" => Token::Break { exit: false },
            "break_exit" => Token::Break { exit: true },
            _ => Token::Action(action),
        }),
        map(definition, Token::Definition),
        map(plain, Token::Plain),
    ));

    let mut iter = iterator(input, token);

    iter.map(f).collect::<Result<(), Error>>()?;

    iter.finish().map_err(convert_error)?;

    Ok(())
}

fn convert_error(err: nom::Err<(&str, nom::error::ErrorKind)>) -> nom::Err<nom::error::Error<String>> {
    err.to_owned().map(|(i, e)| nom::error::Error::new(i, e))
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown action macro: %{0}")]
    UnknownAction(String),
    #[error("unknown definition macro: %({0})")]
    UnknownDefinition(String),
    #[error("parse script")]
    Parser(#[from] nom::Err<nom::error::Error<String>>),
}

#[cfg(test)]
mod test {
    use crate::macros::Action;

    use super::*;

    #[test]
    fn parse_script() {
        let input = "\n\n%patch %%escaped %{ %break_continue\n%break_exit %(pkgdir)/0001-deps-analysis-elves-In-absence-of-soname.-make-one-u.patch";

        let mut parser = Parser::new();
        parser.add_action(
            "patch",
            Action {
                description: "test".into(),
                example: None,
                command: "patch -v %(nested_flag)".into(),
                dependencies: vec!["patch".into()],
            },
        );

        for (id, definition) in [
            ("nested_flag", "--args=%(nested_arg),b,c"),
            ("nested_arg", "a"),
            ("pkgdir", "%(root)/pkg"),
            ("root", "/mason"),
        ] {
            parser.add_definition(id, definition);
        }

        let script = parser.parse(input).unwrap();

        assert_eq!(
            script.commands,
            vec![
                Command::Content("patch -v --args=a,b,c %escaped %{".to_owned()),
                Command::Break(Breakpoint {
                    exit: false,
                    line_num: 2,
                }),
                Command::Break(Breakpoint {
                    exit: true,
                    line_num: 3,
                }),
                Command::Content(
                    "/mason/pkg/0001-deps-analysis-elves-In-absence-of-soname.-make-one-u.patch".to_owned()
                ),
            ]
        );
        assert_eq!(script.dependencies, vec!["patch".to_owned()]);
    }

    #[test]
    fn break_line_num() {
        let test = "patch (pkgdir)/security/CVE-2022-47016.patch\n%break_continue\nconfigure";

        let breakpoint = Parser::new().parse(test).unwrap().commands.remove(1);

        assert_eq!(
            breakpoint,
            Command::Break(Breakpoint {
                line_num: 1,
                exit: false,
            })
        );

        let test = "# Currently the emul32 chain for harfbuzz is on the large side. Revisit\n%meson -Dharfbuzz=disabled\n%break_continue";

        let mut parser = Parser::new();
        parser.add_action(
            "meson",
            Action {
                description: "test".into(),
                example: None,
                command: "meson -j 1".into(),
                dependencies: vec![],
            },
        );
        let breakpoint = parser.parse(test).unwrap().commands.remove(1);

        assert_eq!(
            breakpoint,
            Command::Break(Breakpoint {
                line_num: 2,
                exit: false,
            })
        );
    }
}
