// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! [ScriptEnv] is used to store the immutable environment that a script runs in
//!
//! ```rust
//! # use stone_script::{ScriptEnv, Definition, Action, Expr};
//!
//! let mut env = ScriptEnv::new();
//!
//! env.add_builtin("rustc", "rustc");
//!
//! env.add_definition("rustflags", Definition {
//!   doc: None,
//!   value: Expr::parse("-C opt-level=3").unwrap(),
//! });
//!
//! env.add_action("compile", Action {
//!   doc: None,
//!   example: None,
//!   dependencies: vec!["binary(rustc)".to_owned()],
//!   value: Expr::parse("%(rustc) %(rustflags)").unwrap(),
//! });
//! ```

use std::collections::BTreeMap;

use crate::Expr;

/// A script definition
///
/// Written out, this is of the form `"%(name)"`
#[derive(Debug, Clone)]
pub struct Definition {
    /// Usage documentation
    pub doc: Option<String>,
    /// Expression to evaluate when called
    pub value: Expr,
}

/// A script action
///
/// Written out, this is of the form `"%name"`
#[derive(Debug, Clone)]
pub struct Action {
    /// Usage documentation
    pub doc: Option<String>,
    /// Usage example
    pub example: Option<String>,
    /// Dependencies to add when encountered
    pub dependencies: Vec<String>,
    /// Expression to evaluate when called
    pub value: Expr,
}

/// The immutable context of a script evaluation
#[derive(Debug, Clone, Default)]
pub struct ScriptEnv {
    /// Map of all currently defined builtins
    pub builtins: BTreeMap<String, String>,
    /// Map of all currently defined definitions
    pub definitions: BTreeMap<String, Definition>,
    /// Map of all currently defined actions
    pub actions: BTreeMap<String, Action>,
}

impl ScriptEnv {
    /// Create a new, empty [ScriptEnv]
    ///
    /// ```rust
    /// # use stone_script::ScriptEnv;
    ///
    /// let _env = ScriptEnv::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a builtin to this [ScriptEnv]
    ///
    /// ```rust
    /// # use stone_script::ScriptEnv;
    ///
    /// let mut env = ScriptEnv::new();
    ///
    /// env.add_builtin("rustc", "rustc");
    /// ```
    pub fn add_builtin(&mut self, name: impl ToString, value: impl ToString) {
        self.builtins.insert(name.to_string(), value.to_string());
    }

    /// Add a definition to this [ScriptEnv]
    ///
    /// ```rust
    /// # use stone_script::{ScriptEnv, Definition, Expr};
    ///
    /// let mut env = ScriptEnv::new();
    ///
    /// env.add_definition("rustflags", Definition {
    ///     doc: None,
    ///     value: Expr::parse("-C opt-level=3").unwrap(),
    /// });
    /// ```
    pub fn add_definition(&mut self, name: impl ToString, definition: Definition) {
        self.definitions.insert(name.to_string(), definition);
    }

    /// Add an action to this [ScriptEnv]
    ///
    /// ```rust
    /// # use stone_script::{ScriptEnv, Action, Expr};
    ///
    /// let mut env = ScriptEnv::new();
    ///
    /// env.add_action("compile", Action {
    ///     doc: None,
    ///     example: None,
    ///     dependencies: vec!["binary(rustc)".to_owned()],
    ///     value: Expr::parse("%(rustc) %(rustflags)").unwrap(),
    /// });
    /// ```
    pub fn add_action(&mut self, name: impl ToString, action: Action) {
        self.actions.insert(name.to_string(), action);
    }
}
