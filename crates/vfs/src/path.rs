use astr::AStr;

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

pub fn file_name(path: &str) -> Option<&str> {
    path.trim_end_matches('/').rsplit('/').next()
}

pub fn parent(path: &str) -> Option<&str> {
    path.trim_end_matches('/').rsplit_once('/').map(|(parent, _)| {
        // We had to have split on a direct descendent of `/`
        if parent.is_empty() { "/" } else { parent }
    })
}

pub fn components(path: &str) -> impl Iterator<Item = &str> {
    path.starts_with('/')
        .then_some("/")
        .into_iter()
        .chain(path.split('/'))
        .filter(|s| !s.is_empty())
}
