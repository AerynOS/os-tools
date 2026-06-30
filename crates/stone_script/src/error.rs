// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use snafu::prelude::*;

use crate::context::StackTrace;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum EvalError {
    #[snafu(display("undefined action {name}"))]
    UndefinedAction { name: String },
    #[snafu(display("undefined definition {name}"))]
    UndefinedDefinition { name: String },
    #[snafu(display("undefined builtin {name}"))]
    UndefinedBuiltin { name: String },
    #[snafu(display("builtins not allowed"))]
    BuiltinsNotAllowed,
    #[snafu(display("script call stack too deep"))]
    TooDeep,
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("eval error: {source}"))]
    Eval { source: EvalError, trace: StackTrace },
    #[snafu(display("syntax error"))]
    Syntax {
        source: nom::Err<nom::error::Error<String>>,
    },
}
