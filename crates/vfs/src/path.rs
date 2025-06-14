pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_owned()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
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
