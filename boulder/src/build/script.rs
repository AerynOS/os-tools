// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use stone_script::{Command, Error, Expr, ScriptContext, ScriptEnv};

/// A bundle of the output of a script and the execution environment used to
/// create it.
#[derive(Debug)]
pub struct ScriptBundle {
    /// ScriptEnv used to compile this bundle
    env: ScriptEnv,
    /// Compiled prefix
    prefix: String,
    /// Compiled dependencies
    dependencies: Vec<String>,
    /// Compiled commands
    commands: Vec<Command>,
}

impl ScriptBundle {
    /// Compile a [ScriptBundle] from an [stone_script::Expr], using a specified [stone_script::ScriptEnv]
    pub fn build(env: ScriptEnv, prefix_expr: &Expr, expr: &Expr) -> Result<Self, Error> {
        let prefix = stone_script::eval_to_string(&env, prefix_expr)?;
        let mut ctx = ScriptContext::new();
        ctx.eval(&env, expr)?;
        let commands = ctx
            .flush_commands()
            .map(|command| match command {
                Command::Output { output } => Command::Output {
                    output: format!("{prefix}\n{output}"),
                },
                command => command,
            })
            .collect();
        Ok(Self {
            env,
            prefix,
            dependencies: ctx.dependencies.iter().cloned().collect(),
            commands,
        })
    }

    /// Access the [stone_script::ScriptEnv] used to compile this [ScriptBundle]
    pub fn env(&self) -> &ScriptEnv {
        &self.env
    }

    /// Access the compiled prefix as a string
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Access the resulting dependencies
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    /// Access the resulting commands
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }
}
