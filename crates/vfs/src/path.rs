use std::ops::Deref;

use astr::AStr;
use derive_more::Debug;

pub fn join(a: &str, b: impl AsRef<str> + Into<AStr>) -> AStr {
    let b_ = b.as_ref();
    if b_.starts_with('/') {
        b.into()
    } else if a.ends_with('/') {
        AStr::from(format!("{a}{b_}"))
    } else {
        AStr::from(format!("{a}/{b_}"))
    }
}

#[derive(Clone, Debug)]
#[debug("{path:?}")]
pub struct VfsPath {
    path: AStr,
    file_name_start_idx: u32,
    parent_end_idx: u32,
}

impl VfsPath {
    pub fn new(path: AStr) -> Self {
        assert!(path.starts_with('/'));
        if path.len() > 1 {
            assert!(!path.ends_with('/'));
        }

        let file_name_start_idx = (path.rfind('/').unwrap() + 1).try_into().unwrap();
        let parent_end_idx = if file_name_start_idx == 1 {
            1
        } else {
            file_name_start_idx - 1
        };
        Self {
            path,
            file_name_start_idx,
            parent_end_idx,
        }
    }

    pub fn astr(&self) -> AStr {
        self.path.clone()
    }

    pub fn file_name(&self) -> &str {
        &self.path[self.file_name_start_idx as usize..]
    }

    pub fn parent(&self) -> Option<&str> {
        (self.path.len() > 1).then(|| &self.path[..self.parent_end_idx as usize])
    }
}

impl Deref for VfsPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

pub fn components(path: &str) -> impl Iterator<Item = &str> {
    path.starts_with('/')
        .then_some("/")
        .into_iter()
        .chain(path.split('/'))
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::VfsPath;

    #[test]
    fn vfs_path_root_behavior() {
        let p = VfsPath::new("/".into());
        assert_eq!(p.parent(), None);
        assert_eq!(p.file_name(), "");
    }

    #[test]
    fn vfs_path_non_root_behavior() {
        let p = VfsPath::new("/a".into());
        assert_eq!(p.parent(), Some("/"));
        assert_eq!(p.file_name(), "a");

        let p = VfsPath::new("/abc/def/xyz".into());
        assert_eq!(p.parent(), Some("/abc/def"));
        assert_eq!(p.file_name(), "xyz");
    }

    #[test]
    #[should_panic]
    fn vfs_path_trailing_slash_invalid() {
        VfsPath::new("/xyz/".into());
    }

    #[test]
    #[should_panic]
    fn vfs_path_no_leading_slash_invalid() {
        VfsPath::new("xyz/abc".into());
    }

    #[test]
    #[should_panic]
    fn vfs_path_no_leading_slash_invalid_2() {
        VfsPath::new("abcxyz".into());
    }
}
