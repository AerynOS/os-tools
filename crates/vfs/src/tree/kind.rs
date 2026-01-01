use std::mem::ManuallyDrop;

use astr::AStr;

pub union Kind {
    addr_or_tag: usize,
    symlink_target: ManuallyDrop<AStr>,
}

impl Kind {
    /// Regular path
    pub const REGULAR: Self = Self { addr_or_tag: 0x1 };
    /// Directory (parenting node)
    pub const DIRECTORY: Self = Self { addr_or_tag: 0x2 };

    pub fn symlink(target: AStr) -> Self {
        Self {
            symlink_target: ManuallyDrop::new(target),
        }
    }

    pub fn is_regular(&self) -> bool {
        unsafe { self.addr_or_tag == 0x1 }
    }

    pub fn is_directory(&self) -> bool {
        unsafe { self.addr_or_tag == 0x2 }
    }

    pub fn is_symlink(&self) -> bool {
        unsafe { self.addr_or_tag >= 0x8 }
    }

    pub fn as_symlink(&self) -> Option<&AStr> {
        self.is_symlink().then(|| unsafe { &*self.symlink_target })
    }
}

impl Clone for Kind {
    fn clone(&self) -> Self {
        if let Some(target) = self.as_symlink() {
            let symlink_target = ManuallyDrop::new(AStr::clone(target));
            Self { symlink_target }
        } else {
            let addr_or_tag = unsafe { self.addr_or_tag };
            Self { addr_or_tag }
        }
    }
}

impl Default for Kind {
    fn default() -> Self {
        Self::DIRECTORY
    }
}

impl Drop for Kind {
    fn drop(&mut self) {
        let addr_or_tag = unsafe { self.addr_or_tag };
        debug_assert_ne!(addr_or_tag, 0);
        if addr_or_tag >= 0x8 {
            unsafe {
                ManuallyDrop::drop(&mut self.symlink_target);
            }
        }
    }
}

mod debug_impl {
    use std::fmt;

    #[derive(Debug)]
    #[allow(dead_code)]
    enum Kind<'a> {
        Regular,
        Directory,
        Symlink(&'a str),
    }

    impl super::Kind {
        fn to_enum(&self) -> Kind<'_> {
            if self.is_regular() {
                Kind::Regular
            } else if let Some(target) = self.as_symlink() {
                Kind::Symlink(target)
            } else {
                Kind::Directory
            }
        }
    }

    impl fmt::Debug for super::Kind {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.to_enum().fmt(f)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tree::Kind;

    #[test]
    fn run_this_with_miri() {
        let kind = Kind::symlink("/test/thing".into());
        let kind2 = kind.clone();
        drop(kind);
        drop(kind2);
    }
}
