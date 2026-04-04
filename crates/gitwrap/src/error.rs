use std::{fmt, io};

/// The error type for operations using the `git` executable.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] InnerError);

impl Error {
    /// Returns the kind of I/O error if such error happened.
    /// Otherwise, it returns [None].
    ///
    /// Any I/O error is related to calling the `git` executable.
    /// Refer to [Self::run_failed] for I/O errors that occurred within
    /// `git`'s execution.
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        if let InnerError::Io(err) = &self.0 {
            Some(err.kind())
        } else {
            None
        }
    }

    /// Returns whether `git` exited with an error code.
    pub fn run_failed(&self) -> bool {
        matches!(self.0, InnerError::Run { .. })
    }

    /// Returns the kind of violated [Constraint] if such error happened.
    /// Otherwise, it returns [None].
    pub fn constraint(&self) -> Option<&Constraint> {
        if let InnerError::Constraint(con) = &self.0 {
            Some(con)
        } else {
            None
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Constraint {
    /// The repository is valid, but it is not bare.
    NotBare,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InnerError {
    /// A generic I/O error occurred.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// The `git` executable returned with an error.
    /// A dump of the stderr may be provided.
    #[error("{}", display_run(code, stderr))]
    Run { code: Option<i32>, stderr: Option<String> },

    #[error(transparent)]
    Constraint(#[from] Constraint),
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBare => write!(f, "this repository is not bare"),
        }
    }
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
