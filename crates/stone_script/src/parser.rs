// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, anychar, char, digit1, newline},
    combinator::{eof, iterator, map, peek, recognize, value},
    multi::{many_till, many1},
    sequence::{delimited, preceded, terminated},
};
use snafu::prelude::*;

use crate::error::{Error, SyntaxSnafu};

#[derive(Debug, PartialEq)]
pub enum Token<'a> {
    Action(&'a str),
    Definition(&'a str),
    Builtin(&'a str),
    Plain(&'a str),
    Newline,
    Breakpoint { exit: bool },
}

pub fn tokens(input: &str, mut callback: impl FnMut(Token<'_>) -> Result<(), Error>) -> Result<(), Error> {
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
            "break_continue" => Token::Breakpoint { exit: false },
            "break_exit" => Token::Breakpoint { exit: true },
            action => Token::Action(action),
        }),
        map(definition, |definition| match definition.strip_prefix("_") {
            Some(builtin) => Token::Builtin(builtin),
            None => Token::Definition(definition),
        }),
        map(plain, Token::Plain),
    ));

    let mut iter = iterator(input, token);

    for item in &mut iter {
        callback(item)?;
    }

    iter.finish().map_err(convert_error).context(SyntaxSnafu)?;

    Ok(())
}

fn convert_error(err: nom::Err<(&str, nom::error::ErrorKind)>) -> nom::Err<nom::error::Error<String>> {
    err.to_owned().map(|(i, e)| nom::error::Error::new(i, e))
}

#[cfg(test)]
mod test {
    use super::*;

    fn token_test(source: &str, expected: &[Token<'static>]) {
        let mut index = 0;

        tokens(source, |tok| {
            assert_eq!(tok, expected[index]);
            index += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(index, expected.len());
    }

    #[test]
    fn tokens_works() {
        token_test(
            "have you valued your goose",
            &[Token::Plain("have you valued your goose")],
        );

        token_test(
            "goose value:\n71",
            &[Token::Plain("goose value:"), Token::Newline, Token::Plain("71")],
        );

        token_test(
            "hello %(honorific), you have won 1 million euros",
            &[
                Token::Plain("hello "),
                Token::Definition("honorific"),
                Token::Plain(", you have won 1 million euros"),
            ],
        );

        token_test(
            "%do_not_the contributor",
            &[Token::Action("do_not_the"), Token::Plain(" contributor")],
        );

        token_test(
            "%watch jojo's bizarre adventure, part %(part)",
            &[
                Token::Action("watch"),
                Token::Plain(" jojo's bizarre adventure, part "),
                Token::Definition("part"),
            ],
        );

        token_test(
            "bjarne syrup is 100%% natural",
            &[
                Token::Plain("bjarne syrup is 100"),
                Token::Plain("%"),
                Token::Plain(" natural"),
            ],
        );

        token_test(
            "ikey plz fix this :plead: %break_continue %break_exit",
            &[
                Token::Plain("ikey plz fix this :plead: "),
                Token::Breakpoint { exit: false },
                Token::Plain(" "),
                Token::Breakpoint { exit: true },
            ],
        );

        token_test(
            "%(_compiler_cc) %(cc)",
            &[
                Token::Builtin("compiler_cc"),
                Token::Plain(" "),
                Token::Definition("cc"),
            ],
        );
    }
}
