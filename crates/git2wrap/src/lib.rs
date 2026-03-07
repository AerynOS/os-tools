use std::{fs, io, path::Path};
use url::Url;

pub struct Repository(git2::Repository);

impl Repository {
    /// Tries to open a bare repository from a local directory.
    pub fn open_bare<P: AsRef<Path>>(path: P) -> Result<Self, git2::Error> {
        Ok(Self(git2::Repository::open_bare(path)?))
    }

    /// Tries to clone a bare repository from a source URL
    /// into a local directory.
    pub fn clone_bare(url: &Url, path: &Path) -> Result<Self, Error> {
        CloneOptions::new().clone_bare(url, path)
    }

    /// Checks if the repository has a Git reference.
    /// It returns true if it does, false if it doesn't or an error
    /// occurred while querying the repository.
    pub fn has_ref(&self, git_ref: &str) -> bool {
        self.0.find_reference(git_ref).is_ok()
    }

    /// Fetches new references from upstream.
    pub fn update(&self) -> Result<(), Error> {
        let mut remote = self.0.find_remote("origin")?;
        Ok(remote.fetch(&["HEAD"], None, None)?)
    }

    /// Adds a new worktree.
    pub fn add_worktree(&self, path: &Path, git_ref: &str) -> Result<Worktree, Error> {
        let reff = self.0.find_reference(git_ref)?;
        let mut options = git2::WorktreeAddOptions::new();
        options.reference(Some(&reff));

        let name = path
            .file_name()
            .ok_or(Error::BranchName("".to_string()))?
            .to_string_lossy();

        Ok(Worktree(self.0.worktree(&name, path, Some(&options))?))
    }
}

pub struct Worktree(git2::Worktree);

impl Worktree {
    pub fn remove(self) -> Result<(), Error> {
        let mut force_prune = git2::WorktreePruneOptions::new();
        force_prune.locked(true).valid(true);

        self.0.prune(Some(&mut force_prune))?;
        Ok(fs::remove_dir_all(self.0.path())?)
    }
}

pub struct CloneOptions<'a> {
    progress_callback: Option<Box<git2::IndexerProgress<'a>>>,
}

impl<'a> CloneOptions<'a> {
    pub fn new() -> Self {
        Self {
            progress_callback: None,
        }
    }

    pub fn progress_callback<F>(&mut self, cb: F) -> &mut Self
    where
        F: FnMut(git2::Progress<'_>) -> bool + 'a,
    {
        self.progress_callback = Some(Box::new(cb));
        self
    }

    pub fn clone_bare(self, url: &Url, path: &Path) -> Result<Repository, Error> {
        let mut builder = git2::build::RepoBuilder::new();
        builder.bare(true);

        if let Some(cb) = self.progress_callback {
            let mut remote_callbacks = git2::RemoteCallbacks::new();
            remote_callbacks.transfer_progress(cb);

            let mut fetch_options = git2::FetchOptions::new();
            fetch_options.remote_callbacks(remote_callbacks);
        }

        Ok(Repository(builder.clone(url.as_str(), path)?))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Git(#[from] git2::Error),
    #[error("\"{0}\" is not a valid branch name")]
    BranchName(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}
