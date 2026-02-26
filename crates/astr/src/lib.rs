// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    borrow::{Borrow, Cow},
    fmt,
    hash::Hash,
    ops::Deref,
    path::Path,
};

use stable_deref_trait::StableDeref;
use triomphe::{Arc, HeaderWithLength};

mod cow;
#[cfg(feature = "diesel")]
mod diesel;

pub use self::cow::CowAStr;

/// String 'atom'.
///
/// Cloning doesn't allocate. As of the time of writing, uses reference
/// counting. Implementation may change.
#[derive(Clone)]
pub struct AStr(triomphe::ThinArc<(), u8>);

impl AStr {
    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: We only ever store UTF-8,
        // would use ThinArc<(), str> if possible
        unsafe { str::from_utf8_unchecked(&self.0.slice) }
    }
}

impl Default for AStr {
    fn default() -> Self {
        Self::from(String::new())
    }
}

impl Deref for AStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

unsafe impl StableDeref for AStr {}

impl Borrow<str> for AStr {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for AStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

impl fmt::Display for AStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

impl From<&str> for AStr {
    fn from(value: &str) -> Self {
        Self(Arc::into_thin(Arc::from_header_and_slice(
            HeaderWithLength::new((), value.len()),
            value.as_bytes(),
        )))
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
        Self::from(value.as_str())
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
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<Path> for AStr {
    #[inline]
    fn as_ref(&self) -> &Path {
        self.as_str().as_ref()
    }
}

impl PartialEq for AStr {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for AStr {}

impl PartialOrd for AStr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AStr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for AStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::AStr;

    #[test]
    fn basic_use() {
        let empty = AStr::from("");
        let empty2 = empty.clone();
        assert_eq!(format!("{empty}{empty2}{empty}"), "");

        let x = AStr::from("x");
        assert_eq!(format!("{x}x{x}"), "xxx");
    }

    #[test]
    fn long_string() {
        let foo = AStr::from("/foo/bar/helloworld");
        assert_eq!(foo.as_str(), "/foo/bar/helloworld");
    }
}
