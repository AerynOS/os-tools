// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use crate::{
    Error,
    parser::{self, Token},
};

/// A fragment of an expression
#[derive(Debug, Clone, PartialEq)]
pub enum Fragment {
    /// Some plain text to be emitted
    Output(String),
    /// Reference to a builtin; `"%(_name)"`
    Builtin(String),
    /// Reference to a definition; `"%(name)"`
    Definition(String),
    /// Reference to an action; `%name`
    Action(String),
    /// Breakpoint; `"%break_exit"` and `"%break_continue"`
    Breakpoint {
        /// The line number this breakpoint is on
        line_num: usize,
        /// Whether the script should exit after this breakpoint
        exit: bool,
    },
}

impl Fragment {
    /// Create a [Fragment::Output] from a str
    pub fn output_str(output: &str) -> Self {
        Fragment::Output(output.to_owned())
    }
}

/// A stone script expression
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// The fragments that the expression is composed of
    pub fragments: Vec<Fragment>,
}

impl Expr {
    /// Create an `Expr` from a sequence of `Fragment`
    ///
    /// ```rust
    /// # use stone_script::{Expr, Fragment};
    ///
    /// let _expr = Expr::from_fragments(vec![
    ///     Fragment::output_str("meow"),
    ///     Fragment::Builtin("woof".to_owned()),
    ///     Fragment::Definition("honk".to_owned()),
    ///     Fragment::Action("moo".to_owned()),
    ///     Fragment::Breakpoint { line_num: 0, exit: false },
    /// ]);
    /// ```
    pub fn from_fragments(fragments: impl IntoIterator<Item = Fragment>) -> Self {
        Expr {
            fragments: fragments.into_iter().collect(),
        }
    }

    /// Parse a string into an `Expr`
    ///
    /// ```rust
    /// # use stone_script::{Expr, Fragment};
    ///
    /// let expr = Expr::parse("%(greeting) my good %(honorific)").unwrap();
    ///
    /// let expected = Expr::from_fragments(vec![
    ///     Fragment::Definition("greeting".to_owned()),
    ///     Fragment::output_str(" my good "),
    ///     Fragment::Definition("honorific".to_owned()),
    /// ]);
    ///
    /// assert_eq!(expr, expected);
    /// ```
    pub fn parse(source: &str) -> Result<Self, Error> {
        let mut line_num = 0;
        let mut fragments = Vec::new();

        parser::tokens(source, |tok| {
            match tok {
                Token::Action(name) => {
                    fragments.push(Fragment::Action(name.to_owned()));
                }
                Token::Builtin(name) => {
                    fragments.push(Fragment::Builtin(name.to_owned()));
                }
                Token::Definition(name) => {
                    fragments.push(Fragment::Definition(name.to_owned()));
                }
                Token::Plain(output) => {
                    fragments.push(Fragment::output_str(output));
                }
                Token::Newline => {
                    fragments.push(Fragment::output_str("\n"));
                    line_num += 1;
                }
                Token::Breakpoint { exit } => {
                    fragments.push(Fragment::Breakpoint { line_num, exit });
                }
            }

            Ok(())
        })?;

        Ok(Expr { fragments })
    }

    /// Dump an `Expr` back into its source code
    ///
    /// ```rust
    /// # use stone_script::{Expr, Fragment};
    ///
    /// let expr = Expr::from_fragments(vec![
    ///     Fragment::Action("say".to_owned()),
    ///     Fragment::output_str(" hello "),
    ///     Fragment::Definition("location".to_owned()),
    /// ]);
    ///
    /// let mut output = String::new();
    ///
    /// expr.dump(&mut output);
    ///
    /// assert_eq!(output, "%say hello %(location)");
    /// ```
    pub fn dump(&self, output: &mut String) {
        for fragment in &self.fragments {
            match fragment {
                Fragment::Output(o) => {
                    // TODO(lumi): support escapes
                    output.push_str(o);
                }
                Fragment::Builtin(name) => {
                    output.push_str("%(_");
                    output.push_str(name);
                    output.push(')');
                }
                Fragment::Definition(name) => {
                    output.push_str("%(");
                    output.push_str(name);
                    output.push(')');
                }
                Fragment::Action(name) => {
                    output.push('%');
                    output.push_str(name);
                }
                Fragment::Breakpoint { exit, .. } => {
                    if *exit {
                        output.push_str("%break_exit");
                    } else {
                        output.push_str("%break_continue");
                    }
                }
            }
        }
    }

    /// Like `dump`, but allocates the string itself
    ///
    /// ```rust
    /// # use stone_script::{Expr, Fragment};
    ///
    /// let expr = Expr::from_fragments(vec![
    ///     Fragment::Action("say".to_owned()),
    ///     Fragment::output_str(" hello "),
    ///     Fragment::Definition("location".to_owned()),
    /// ]);
    ///
    /// assert_eq!(expr.dump_to_string(), "%say hello %(location)");
    /// ```
    pub fn dump_to_string(&self) -> String {
        let mut output = String::new();
        self.dump(&mut output);
        output
    }

    /// Concatenate two exprs, one after the other
    pub fn concat(&self, other: &Expr) -> Expr {
        Expr {
            fragments: self.fragments.iter().chain(other.fragments.iter()).cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_test(source: &str) {
        let expr = Expr::parse(source).unwrap();
        assert_eq!(source, expr.dump_to_string());
    }

    #[test]
    fn parse_works() {
        roundtrip_test("");
        roundtrip_test("gay goat grub");
        roundtrip_test("%(wustc) owo.ws");
        roundtrip_test("%sign_into_law transrights");
        roundtrip_test("%catapult %(you) snugglepile");
        roundtrip_test("%broadcast my fedi fwiends are cuties owo");
        roundtrip_test("shouldn't forget the builtins %(_plsdontforgetme)");
        roundtrip_test(
            "hello mr. goose\n\ni am writing to you regarding %(topic),\nyou have voted %(vote) on %(topic), and i am not happy with this\n\ni will never vote for  you again\n\nunkind regards,\negg",
        );

        // TODO(lumi): roundtrip tests don't currently work with escapes
        // roundtrip_test("we're all 100%% meowed with no way out");
    }
}
