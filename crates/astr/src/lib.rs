use std::{
    borrow::{Borrow, Cow},
    fmt,
    ops::Deref,
    path::Path,
};

mod diesel;

/// String 'atom'.
///
/// Cloning doesn't allocate. As of the time of writing, uses reference
/// counting. Implementation may change.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AStr(triomphe::Arc<str>);

impl AStr {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AStr {
    fn default() -> Self {
        Self::from(String::new())
    }
}

impl Deref for AStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<str> for AStr {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for AStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<&str> for AStr {
    #[inline]
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<&AStr> for AStr {
    #[inline]
    fn from(value: &AStr) -> Self {
        value.clone()
    }
}

impl From<String> for AStr {
    #[inline]
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

impl From<Cow<'_, str>> for AStr {
    #[inline]
    fn from(value: Cow<'_, str>) -> Self {
        (&*value).into()
    }
}

impl<'a> From<&'a AStr> for Cow<'a, str> {
    #[inline]
    fn from(value: &'a AStr) -> Self {
        Cow::Borrowed(value)
    }
}

impl AsRef<str> for AStr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for AStr {
    fn as_ref(&self) -> &Path {
        self.as_str().as_ref()
    }
}
