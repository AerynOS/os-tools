// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! # Introduction
//!
//! The `stone_script` crate is used for parsing the script templates used in
//! recipes and macros.
//!
//! # Syntax
//!
//! A script is composed of some text with directives inside of it, these include:
//!  - `"%(_name)"`: Evaluates the builtin value with name `name`, these may not be used on the bottom stack frame
//!  - `"%(name)"`: Evaluates the definition with name `name`
//!  - `"%name"`: Evaluates the action with name `name`
//!  - `"%break_continue"`: Causes a breakpoint to be emitted
//!  - `"%break_exit"`: Causes a breakpoint to be emitted, requests an exit afterwards
//!
//! # Components
//!
//! [ScriptEnv] is the immutable environment a script is run in, which includes:
//!  - builtins (populated with [ScriptEnv::add_builtin] and [ScriptEnv::add_builtin_string])
//!  - definitions (populated with [ScriptEnv::add_definition])
//!  - actions (populated with [ScriptEnv::add_action])
//!
//! [Definition] and [Action] represent definitions and actions, respectively.
//!
//! [ScriptContext] is the mutable context a script runs in, this contains the
//! outputs of the script (commands and dependencies), plus configuration.
//!
//! To evaluate an expression within the [ScriptContext], use [ScriptContext::eval].
//!
//! To get the commands from a [ScriptContext], use [ScriptContext::flush_commands].
//!
//! If skipping breakpoints is not an issue, [ScriptContext::flush_output] and
//! [ScriptContext::flush_to_string] can be used.
//!
//! [Expr] is an expression which can be run inside a [ScriptContext] using a
//! specified [ScriptEnv]. An [Expr] is composed of [Fragment]s. These are
//! evaluated in sequence when run by a [ScriptContext].
//!
//! An [Expr] can be constructed using [Expr::parse] and [Expr::from_fragments].
//!
//! [Fragment] is a part of an [Expr], it can be one of:
//!  - [Fragment::Output]: Outputs some specified text
//!  - [Fragment::Builtin]: Evaluates the builtin with the specified name
//!  - [Fragment::Definition]: Evaluates the definition with the specified name
//!  - [Fragment::Action]: Evaluates the action with the specified name
//!  - [Fragment::Breakpoint]: Marks a breakpoint in the script
//!
//! [Command] represents an output of a script, which could either be some
//! output, or a breakpoint.
//!
//! [Error] is an enum with some errors that can happen while evaluating and
//! parsing scripts.
//!
//! [EvalError] is an enum with some errors that can happen while evaluating
//! scripts:
//!  - [EvalError::BuiltinsNotAllowed]: Returned when builtins are not allowed
//!  - [EvalError::UndefinedDefinition]: Returned when a definition cannot be found
//!  - [EvalError::UndefinedAction]: Returned when an action cannot be found
//!  - [EvalError::UndefinedBuiltin]: Returned when a builtin cannot be found
//!  - [EvalError::TooDeep]: Returned when the call stack gets too deep
//!
//! # Example
//!
//! ```rust
//! use stone_script::{Action, Command, Definition, Expr, ScriptContext, ScriptEnv};
//!
//! // Create the env
//! let mut env = ScriptEnv::new();
//!
//! // Add builtins
//! env.add_builtin_string("cc", "gcc");
//! env.add_builtin_string("prefix", "/usr");
//!
//! // Add proxy definitions to the builtins
//! env.add_definition("cc", Definition {
//!     doc: Some("C compiler command".to_owned()),
//!     value: Expr::parse("%(_cc)").unwrap(),
//! });
//!
//! env.add_definition("prefix", Definition {
//!     doc: Some("/usr prefix".to_owned()),
//!     value: Expr::parse("%(_prefix)").unwrap(),
//! });
//!
//! // Add definitions
//! env.add_definition("cflags", Definition {
//!     doc: Some("Flags to pass to the C compiler".to_owned()),
//!     value: Expr::parse("-Wall -Werror").unwrap(),
//! });
//!
//! env.add_definition("ldflags", Definition {
//!     doc: Some("Flags to pass to the linker".to_owned()),
//!     value: Expr::parse("").unwrap(),
//! });
//!
//! env.add_definition("bindir", Definition {
//!     doc: Some("Relative path of the binary directory, from /usr".to_owned()),
//!     value: Expr::parse("/bin").unwrap(),
//! });
//!
//! // Add actions
//! env.add_action("compile_cc", Action {
//!     doc: Some("Compile a file using the C compiler".to_owned()),
//!     example: Some("%compile_cc main.c -o main.o".to_owned()),
//!     dependencies: vec!["binary(gcc)".to_owned()],
//!     value: Expr::parse("%(cc) %(cflags)").unwrap(),
//! });
//!
//! env.add_action("link", Action {
//!     doc: Some("Link object files using the C compiler".to_owned()),
//!     example: Some("%link a.o b.o c.o -o main".to_owned()),
//!     dependencies: vec!["binary(gcc)".to_owned()],
//!     value: Expr::parse("%(cc) %(ldflags)").unwrap(),
//! });
//!
//! env.add_action("copy", Action {
//!     doc: Some("Copy files to a specific location".to_owned()),
//!     example: Some("%copy file1 file2 file3 dest".to_owned()),
//!     dependencies: vec!["binary(cp)".to_owned()],
//!     value: Expr::parse("cp").unwrap(),
//! });
//!
//! // Parse expr to run
//! let expr = Expr::parse(r#"
//!     %compile_cc blah.c -o blah.o
//!     %compile_cc main.c -o main.o
//!     %break_continue
//!     %link blah.o main.o -o main
//!     %copy main %(prefix)%(bindir)
//! "#).unwrap();
//!
//! // Create context to run expr in
//! let mut ctx = ScriptContext::new();
//!
//! // Evaluate the expression
//! ctx.eval(&env, &expr).unwrap();
//!
//! // Flush the commands from the context and collect them into a vec.
//! let commands: Vec<Command> = ctx.flush_commands().collect();
//!
//! // Check the outputs
//! assert_eq!(commands, vec![
//!     Command::Output { output: "\n    gcc -Wall -Werror blah.c -o blah.o\n    gcc -Wall -Werror main.c -o main.o\n    ".to_owned() },
//!     Command::Breakpoint { line_num: 3, exit: false },
//!     Command::Output { output: "\n    gcc  blah.o main.o -o main\n    cp main /usr/bin\n".to_owned() },
//! ]);
//! ```

mod context;
mod env;
mod error;
mod expr;
mod parser;

pub use context::{Command, ScriptContext};
pub use env::{Action, Definition, ScriptEnv};
pub use error::{Error, EvalError};
pub use expr::{Expr, Fragment};

/// Evaluate an [Expr] in a single-use context and extract the output as a string
///
/// ```rust
/// # use stone_script::{Definition, Expr, ScriptEnv};
///
/// let mut env = ScriptEnv::new();
/// env.add_definition("name", Definition {
///     doc: None,
///     value: Expr::parse("Seymour").unwrap(),
/// });
/// env.add_definition("item", Definition {
///     doc: None,
///     value: Expr::parse("fast food").unwrap(),
/// });
/// env.add_definition("adverb", Definition {
///     doc: None,
///     value: Expr::parse("Delightfully").unwrap(),
/// });
///
/// let expr = Expr::parse("What if I would purchase %(item) and disguise it as my own cooking? %(adverb) devilish, %(name).").unwrap();
///
/// assert_eq!(
///     stone_script::eval_to_string(&env, &expr).unwrap(),
///     "What if I would purchase fast food and disguise it as my own cooking? Delightfully devilish, Seymour.",
/// );
/// ```
pub fn eval_to_string(env: &ScriptEnv, expr: &Expr) -> Result<String, Error> {
    let mut ctx = ScriptContext::new();
    ctx.eval(env, expr)?;
    Ok(ctx.flush_to_string())
}

#[cfg(test)]
mod test {
    use super::*;

    fn make_env() -> ScriptEnv {
        let mut env = ScriptEnv::new();
        env.add_builtin_string("compiler_cc", "gcc");
        env.add_definition(
            "compiler_cc",
            Definition {
                doc: None,
                value: Expr::parse("%(_compiler_cc)").unwrap(),
            },
        );
        env.add_definition(
            "cc",
            Definition {
                doc: None,
                value: Expr::parse("%(compiler_cc)").unwrap(),
            },
        );
        env.add_definition(
            "rc_service",
            Definition {
                doc: None,
                value: Expr::parse("rc-service").unwrap(),
            },
        );
        env.add_action(
            "retrieve",
            Action {
                doc: None,
                example: None,
                dependencies: vec!["binary(wget)".to_owned()],
                value: Expr::parse("wget").unwrap(),
            },
        );
        env.add_action(
            "annoy",
            Action {
                doc: None,
                example: None,
                dependencies: vec!["binary(poke)".to_owned()],
                value: Expr::parse("poke -n 100").unwrap(),
            },
        );
        env.add_action(
            "nhac",
            Action {
                doc: None,
                example: None,
                dependencies: vec!["catnip".to_owned()],
                // rip your finger
                value: Expr::parse(r#"%retrieve bitey-girlfriend; %annoy bitey-girlfriend"#).unwrap(),
            },
        );
        env
    }

    fn full_expr_test(source: &str, output: &str, dependencies: &[&str]) {
        let expr = Expr::parse(source).unwrap();
        let mut ctx = ScriptContext::new();
        let env = make_env();
        ctx.eval(&env, &expr).unwrap();
        assert_eq!(ctx.flush_to_string(), output);
        assert_eq!(
            ctx.dependencies,
            dependencies.into_iter().map(|&dep| dep.to_owned()).collect()
        );
    }

    #[test]
    fn eval_works() {
        full_expr_test("meow :3", "meow :3", &[]);
        full_expr_test("%(cc) -o meow meow.c", "gcc -o meow meow.c", &[]);
        full_expr_test(
            "%(rc_service) restart gardenhouse",
            "rc-service restart gardenhouse",
            &[],
        );
        full_expr_test(
            "%nhac",
            "wget bitey-girlfriend; poke -n 100 bitey-girlfriend",
            &["catnip", "binary(wget)", "binary(poke)"],
        );
    }
}
