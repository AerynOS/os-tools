use std::io;

use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum RunError {
    /// A generic I/O error occured.
    #[snafu(display("{source}"))]
    Io { source: io::Error },

    /// The `git` executable returned with an error.
    /// A dump of the stderr may be provided.
    #[snafu(display("{}", display_run(&code, &stderr)))]
    Run { code: Option<i32>, stderr: Option<String> },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum OpenError {
    #[snafu(transparent)]
    Run { source: RunError },

    /// The repository is valid, but it is not bare.
    #[snafu(display("this repository is not bare"))]
    NotBare,
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum WorktreeError {
    #[snafu(transparent)]
    Run { source: RunError },

    /// The commit is not identified by its hash.
    #[snafu(display("commit ID \"{commit}\" is not a commit hash"))]
    NotPeeled { commit: String },
}

fn display_run(code: &Option<i32>, stderr: &Option<String>) -> String {
    let mut string = String::from("`git` exited ");

    if let Some(code) = code {
        string.push_str(&format!("with code {code}"));
    } else {
        string.push_str("unexpectedly");
    }

    if let Some(msg) = stderr {
        string.push_str(&format!(". Diagnostic output below:\n{msg}"));
    }

    string
}
