use std::ops::Deref;

use super::AStr;

pub enum CowAStr<'a> {
    Borrowed(&'a AStr),
    Owned(AStr),
}

impl Deref for CowAStr<'_> {
    type Target = AStr;

    fn deref(&self) -> &Self::Target {
        match self {
            CowAStr::Borrowed(astr) => astr,
            CowAStr::Owned(astr) => astr,
        }
    }
}
