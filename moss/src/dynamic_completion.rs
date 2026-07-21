use clap_complete::CompletionCandidate;
use std::path::PathBuf;

use super::{Installation, client::Client, package};

fn generate_results(client: Client, flags: package::Flags, prefix: &str) -> Vec<CompletionCandidate> {
    client
        .prefix_search(prefix, flags)
        .filter(|name| name.as_str().starts_with(prefix))
        .map(|name| CompletionCandidate::from(name.as_str()))
        .collect()
}

fn default_client() -> Client {
    let root = PathBuf::from("/");
    let installation = Installation::open(root, None).unwrap();
    Client::new("moss", installation).unwrap()
}

pub fn prefix_completer(flags: package::Flags) -> impl Fn(&std::ffi::OsStr) -> Vec<CompletionCandidate> {
    move |prefix: &std::ffi::OsStr| {
        let Some(prefix) = prefix.to_str() else {
            return vec![];
        };
        if prefix.is_empty() {
            return vec![];
        }
        generate_results(default_client(), flags, prefix)
    }
}
