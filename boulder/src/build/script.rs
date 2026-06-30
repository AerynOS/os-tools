// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

/// A bundle of a script and its execution environment
#[derive(Debug)]
pub struct ScriptBundle {
    /// The environment to run the script in
    pub env: stone_script::ScriptEnv,
    /// Prefix to be added to the script before running it (previously called `env`)
    pub prefix: stone_script::Expr,
    /// The parsed script
    pub expr: stone_script::Expr,
}
