use std::{borrow::Cow, error::Error, fmt};

#[derive(Debug)]
pub(crate) struct ErrorWithContext<E> {
    message: Cow<'static, str>,
    error: E,
}

impl<E> fmt::Display for ErrorWithContext<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl<E: Error + 'static> Error for ErrorWithContext<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

pub(crate) trait ResultExt<T, E> {
    fn context(self, message: impl Into<Cow<'static, str>>) -> Result<T, ErrorWithContext<E>>;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn context(self, message: impl Into<Cow<'static, str>>) -> Result<T, ErrorWithContext<E>> {
        match self {
            Ok(ok) => Ok(ok),
            Err(error) => Err(ErrorWithContext {
                message: message.into(),
                error,
            }),
        }
    }
}
