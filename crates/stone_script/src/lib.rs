// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

mod context;
mod env;
mod error;
mod expr;
mod parser;

pub use context::{Command, ScriptContext};
pub use env::{Action, Definition, ScriptEnv};
pub use error::{Error, EvalError};
pub use expr::{Expr, Fragment};

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
