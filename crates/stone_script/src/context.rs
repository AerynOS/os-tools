// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, mem};

use crate::{Error, Expr, Fragment, ScriptEnv, error::EvalError};

const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Output { output: String },
    Breakpoint { line_num: usize, exit: bool },
}

/// The mutable context of a script evaluation
#[derive(Debug, Clone, Default)]
pub struct ScriptContext {
    /// Temporary output buffer, for coalescing repeated Command::Output
    output: String,

    /// Current output commands of the script
    commands: Vec<Command>,

    /// Current dependencies of the script
    pub dependencies: BTreeSet<String>,

    /// If this is set, builtins are always allowed, else they are only allowed
    /// inside of definitions and actions
    pub always_allow_builtins: bool,
}

impl ScriptContext {
    /// Create a new, empty script context
    ///
    /// ```rust
    /// # use stone_script::ScriptContext;
    ///
    /// let _ctx = ScriptContext::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    fn finish(&mut self) {
        if !self.output.is_empty() {
            let command = Command::Output {
                output: mem::take(&mut self.output),
            };
            self.commands.push(command);
        }
    }

    /// Evaluate an expression in this script context, using `env` as its environment
    ///
    /// ```rust
    /// # use stone_script::{Definition, Expr, ScriptContext, ScriptEnv};
    ///
    /// let expr = Expr::parse("you have won %(amount) packs of tofu! retrieve them at %(location)!").unwrap();
    ///
    /// let mut ctx = ScriptContext::new();
    ///
    /// let mut env = ScriptEnv::new();
    /// env.add_definition("amount", Definition { value: Expr::parse("12").unwrap(), doc: None });
    /// env.add_definition("location", Definition { value: Expr::parse("the cabin of mr valued goose").unwrap(), doc: None });
    ///
    /// ctx.eval(&env, &expr).unwrap();
    ///
    /// assert_eq!(ctx.flush_to_string(), "you have won 12 packs of tofu! retrieve them at the cabin of mr valued goose!");
    /// ```
    pub fn eval(&mut self, env: &ScriptEnv, expr: &Expr) -> Result<(), Error> {
        let mut stack = Vec::with_capacity(MAX_DEPTH);

        fn unwind_stack<'a>(stack: &mut Vec<Frame<'a>>) -> StackTrace {
            let mut trace = Vec::new();
            while let Some(frame) = stack.pop() {
                let fragment = &frame.expr.fragments[frame.progress - 1];
                trace.push(match fragment {
                    Fragment::Output(output) => format!("output {output:?}"),
                    Fragment::Builtin(name) => format!("builtin {name:?}"),
                    Fragment::Definition(name) => format!("definition {name:?}"),
                    Fragment::Action(name) => format!("action {name:?}"),
                    Fragment::Breakpoint { line_num, exit } => format!("breakpoint at {line_num} exit={exit:?}"),
                });
            }
            StackTrace { frames: trace }
        }

        struct Frame<'a> {
            expr: &'a Expr,
            progress: usize,
        }

        stack.push(Frame { expr, progress: 0 });

        while let Some(frame) = stack.last_mut() {
            if frame.progress >= frame.expr.fragments.len() {
                let _ = stack.pop();
                continue;
            }

            frame.progress += 1;

            match &frame.expr.fragments[frame.progress - 1] {
                Fragment::Output(o) => {
                    self.output.push_str(o);
                }
                Fragment::Builtin(name) => {
                    // builtins are not allowed in the top level
                    if !self.always_allow_builtins && stack.len() == 1 {
                        return Err(Error::Eval {
                            source: EvalError::BuiltinsNotAllowed,
                            trace: unwind_stack(&mut stack),
                        });
                    }
                    let value = env.builtins.get(name).ok_or_else(|| Error::Eval {
                        source: EvalError::UndefinedBuiltin { name: name.to_owned() },
                        trace: unwind_stack(&mut stack),
                    })?;
                    if stack.len() >= MAX_DEPTH {
                        return Err(Error::Eval {
                            source: EvalError::TooDeep,
                            trace: unwind_stack(&mut stack),
                        });
                    }
                    stack.push(Frame {
                        expr: &value,
                        progress: 0,
                    });
                }
                Fragment::Definition(name) => {
                    if let Some(definition) = env.definitions.get(name) {
                        if stack.len() >= MAX_DEPTH {
                            return Err(Error::Eval {
                                source: EvalError::TooDeep,
                                trace: unwind_stack(&mut stack),
                            });
                        }
                        stack.push(Frame {
                            expr: &definition.value,
                            progress: 0,
                        });
                    } else {
                        return Err(Error::Eval {
                            source: EvalError::UndefinedDefinition { name: name.to_owned() },
                            trace: unwind_stack(&mut stack),
                        });
                    }
                }
                Fragment::Action(name) => {
                    let action = env.actions.get(name).ok_or_else(|| Error::Eval {
                        source: EvalError::UndefinedAction { name: name.to_owned() },
                        trace: unwind_stack(&mut stack),
                    })?;
                    if stack.len() >= MAX_DEPTH {
                        return Err(Error::Eval {
                            source: EvalError::TooDeep,
                            trace: unwind_stack(&mut stack),
                        });
                    }
                    self.dependencies.extend(action.dependencies.iter().cloned());
                    stack.push(Frame {
                        expr: &action.value,
                        progress: 0,
                    });
                }
                Fragment::Breakpoint { line_num, exit } => {
                    self.finish();
                    self.commands.push(Command::Breakpoint {
                        line_num: *line_num,
                        exit: *exit,
                    });
                }
            }
        }

        self.finish();

        Ok(())
    }

    /// Flush all the emitted commands from the context
    pub fn flush_commands(&mut self) -> impl Iterator<Item = Command> {
        self.commands.drain(..)
    }

    /// Flush all of the emitted commands from the output, and push them onto
    /// the end of the target [String]
    pub fn flush_output(&mut self, target: &mut String) {
        for command in self.flush_commands() {
            match command {
                Command::Output { output } => {
                    target.push_str(&output);
                }
                Command::Breakpoint { .. } => {}
            }
        }
    }

    /// Flush all of the emitted commands from the output into a newly allocated
    /// [String]
    pub fn flush_to_string(&mut self) -> String {
        let mut output = String::new();
        self.flush_output(&mut output);
        output
    }
}

/// A stack trace for an evaluation of an [Expr] in a [ScriptContext]
#[derive(Debug)]
pub struct StackTrace {
    /// Stack frames in the trace, most recent first
    pub frames: Vec<String>,
}

#[cfg(test)]
mod test {
    use crate::env::{Action, Definition};

    use super::*;

    fn expr_output_test(expr: &Expr, output: &str) {
        let mut ctx = ScriptContext::new();
        let env = ScriptEnv::new();
        ctx.eval(&env, &expr).unwrap();
        assert_eq!(ctx.flush_to_string(), output);
    }

    #[test]
    fn eval_works() {
        expr_output_test(&Expr::from_fragments(vec![Fragment::output_str("meow :3")]), "meow :3");

        expr_output_test(
            &Expr::from_fragments(vec![
                Fragment::output_str("you"),
                Fragment::output_str("are"),
                Fragment::output_str("gay"),
                Fragment::output_str(":3"),
            ]),
            "youaregay:3",
        );
    }

    #[test]
    fn breakpoint_works() {
        let mut ctx = ScriptContext::new();
        let env = ScriptEnv::new();
        ctx.eval(
            &env,
            &Expr::from_fragments(vec![
                Fragment::output_str("do not--\n"),
                Fragment::Breakpoint {
                    line_num: 0,
                    exit: false,
                },
                Fragment::output_str("what did i say???"),
                Fragment::Breakpoint {
                    line_num: 1,
                    exit: true,
                },
            ]),
        )
        .unwrap();
        let expected = &[
            Command::Output {
                output: "do not--\n".to_owned(),
            },
            Command::Breakpoint {
                line_num: 0,
                exit: false,
            },
            Command::Output {
                output: "what did i say???".to_owned(),
            },
            Command::Breakpoint {
                line_num: 1,
                exit: true,
            },
        ];
        for (i, command) in ctx.flush_commands().enumerate() {
            assert_eq!(expected[i], command);
        }
    }

    #[test]
    fn stack_overflow_works_with_builtin() {
        let mut ctx = ScriptContext::new();
        ctx.always_allow_builtins = true;
        let mut env = ScriptEnv::new();
        env.add_builtin("kaboom", Expr::parse("%(_kaboom)").unwrap());
        let expr = Expr::parse("%(_kaboom)").unwrap();
        std::assert_matches!(
            ctx.eval(&env, &expr),
            Err(Error::Eval {
                source: EvalError::TooDeep,
                ..
            })
        );
    }

    #[test]
    fn stack_overflow_works_with_definition() {
        let mut ctx = ScriptContext::new();
        let mut env = ScriptEnv::new();
        env.add_definition(
            "kaboom",
            Definition {
                doc: None,
                value: Expr::parse("%(kaboom)").unwrap(),
            },
        );
        let expr = Expr::parse("%(kaboom)").unwrap();
        std::assert_matches!(
            ctx.eval(&env, &expr),
            Err(Error::Eval {
                source: EvalError::TooDeep,
                ..
            })
        );
    }

    #[test]
    fn stack_overflow_works_with_action() {
        let mut ctx = ScriptContext::new();
        let mut env = ScriptEnv::new();
        env.add_action(
            "kaboom",
            Action {
                doc: None,
                example: None,
                dependencies: Vec::new(),
                value: Expr::parse("%kaboom").unwrap(),
            },
        );
        let expr = Expr::parse("%kaboom").unwrap();
        std::assert_matches!(
            ctx.eval(&env, &expr),
            Err(Error::Eval {
                source: EvalError::TooDeep,
                ..
            })
        );
    }

    #[test]
    fn stack_overflow_works_with_combination() {
        let mut ctx = ScriptContext::new();
        let mut env = ScriptEnv::new();
        env.add_builtin("scissors", Expr::parse("%rock").unwrap());
        env.add_definition(
            "paper",
            Definition {
                doc: None,
                value: Expr::parse("%(_scissors)").unwrap(),
            },
        );
        env.add_action(
            "rock",
            Action {
                doc: None,
                example: None,
                dependencies: Vec::new(),
                value: Expr::parse("%(paper)").unwrap(),
            },
        );
        let expr = Expr::parse("%rock").unwrap();
        std::assert_matches!(
            ctx.eval(&env, &expr),
            Err(Error::Eval {
                source: EvalError::TooDeep,
                ..
            })
        );
    }

    #[test]
    fn call_stack_works() {
        let mut ctx = ScriptContext::new();
        let mut env = ScriptEnv::new();
        env.add_action(
            "compile",
            Action {
                doc: None,
                example: None,
                dependencies: Vec::new(),
                value: Expr::parse("%(cc)").unwrap(),
            },
        );
        env.add_definition(
            "cc",
            Definition {
                doc: None,
                value: Expr::parse("gcc %(cc_params)").unwrap(),
            },
        );
        env.add_definition(
            "cc_params",
            Definition {
                doc: None,
                value: Expr::parse("-Wall -Werror %(extra_params)").unwrap(),
            },
        );
        let expr = Expr::parse("%compile test.c").unwrap();
        let Err(Error::Eval { source, trace }) = ctx.eval(&env, &expr) else {
            panic!("expected an error");
        };
        if let EvalError::UndefinedDefinition { name } = source {
            assert_eq!(name, "extra_params");
        } else {
            panic!("expected an undefined definition error");
        }
        assert_eq!(trace.frames.len(), 4);
        assert_eq!(trace.frames[0], "definition \"extra_params\"");
        assert_eq!(trace.frames[1], "definition \"cc_params\"");
        assert_eq!(trace.frames[2], "definition \"cc\"");
        assert_eq!(trace.frames[3], "action \"compile\"");
    }

    #[test]
    fn test_case_pgo_dir() {
        let mut env = ScriptEnv::new();

        env.add_definition(
            "pgo_dir",
            Definition {
                doc: None,
                value: Expr::parse("/mason/build/x86_64-pgo").unwrap(),
            },
        );

        let mut ctx = ScriptContext::new();

        ctx.eval(&env, &Expr::parse("-fprofile-generate=%(pgo_dir)/IR").unwrap())
            .unwrap();

        assert_eq!(ctx.flush_to_string(), "-fprofile-generate=/mason/build/x86_64-pgo/IR");
    }
}
