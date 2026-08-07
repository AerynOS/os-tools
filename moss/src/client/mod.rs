// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! The core client implementation for the moss package manager
//!
//! A [`Client`] needs to be constructed to handle the initialisation of various
//! databases, plugins and data sources to centralise package query and management
//! operations

use std::{
    borrow::Borrow,
    io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs_err as fs;
use futures_util::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use nix::{
    NixPath,
    errno::Errno,
    libc::{AT_FDCWD, RENAME_EXCHANGE, SYS_renameat2, syscall},
};
use postblit::TriggerScope;
use stone::{StoneDecodedPayload, StonePayloadLayoutRecord};
use thiserror::Error;
use tracing::{info, info_span};
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use self::install::install;
use self::prune::{prune_cache, prune_states};
use self::remove::remove;
use self::sync::sync;
use self::verify::verify;
use crate::{
    Installation, Package, Provider, Registry, Signal, State, SystemModel,
    client::fetch::fetch,
    db, environment, fstree, installation, package,
    registry::plugin::{self, Plugin},
    repository, runtime, signal,
    state::{self, Selection},
    system_model::{self, LoadedSystemModel},
};

pub use self::extract::extract;
pub use self::index::index;
pub use self::self_upgrade::self_upgrade;

mod boot;
mod cache;
mod fetch;
mod install;
mod postblit;
mod remove;
mod self_upgrade;
mod sync;
mod verify;

pub mod extract;
pub mod index;
pub mod prune;

/// A builder for [`Client`]
pub struct ClientBuilder {
    client_name: String,
    installation: Installation,
    repositories: Option<repository::Map>,
    system_model_path: Option<PathBuf>,
    blit_root: Option<PathBuf>,
}

impl ClientBuilder {
    /// Set the repositories
    pub fn repositories(mut self, repositories: repository::Map) -> ClientBuilder {
        self.repositories = Some(repositories);
        self
    }

    /// Set system model path
    pub fn system_model_path(mut self, path: impl Into<PathBuf>) -> ClientBuilder {
        self.system_model_path = Some(path.into());
        self
    }

    /// Set the client to an ephemeral client that doesn't record state changes
    /// and blits to a different root.
    ///
    /// This is useful for installing a root to a container (i.e. Boulder) while
    /// using a shared cache.
    ///
    /// Returns an error on construction if `blit_root` is the same as the installation
    /// root, since the system client should always be stateful.
    pub fn ephemeral(mut self, blit_root: impl Into<PathBuf>) -> ClientBuilder {
        self.blit_root = Some(blit_root.into());
        self
    }

    /// Build the [`Client`]
    pub fn build(mut self) -> Result<Client, Error> {
        if let Some(path) = self.system_model_path {
            self.installation.system_model =
                Some(system_model::load(&path)?.ok_or(Error::ImportSystemModelDoesntExist(path.to_owned()))?);
        }

        let config = config::Manager::system(&self.installation.root, "moss");
        let install_db = db::meta::Database::new(self.installation.db_path("install").to_str().unwrap_or_default())?;
        let state_db = db::state::Database::new(self.installation.db_path("state").to_str().unwrap_or_default())?;
        let layout_db = db::layout::Database::new(self.installation.db_path("layout").to_str().unwrap_or_default())?;

        let repositories = if let Some(repos) = self.repositories {
            repository::Manager::with_explicit(&self.client_name, repos, self.installation.clone())?
        } else if let Some(system_model) = &self.installation.system_model {
            repository::Manager::with_system_model(&self.client_name, system_model.clone(), self.installation.clone())?
        } else {
            repository::Manager::with_config_manager(config.clone(), self.installation.clone())?
        };

        let registry = build_registry(&self.installation, &repositories, &install_db, &state_db)?;

        let mut client = Client {
            config,
            installation: self.installation,
            repositories,
            registry,
            install_db,
            state_db,
            layout_db,
            scope: Scope::Stateful,
        };

        if let Some(blit_root) = self.blit_root {
            client = client.ephemeral(blit_root)?;
        }
        Ok(client)
    }
}

/// A Client is a connection to the underlying package management systems
pub struct Client {
    /// Root that we operate on
    installation: Installation,
    /// Combined set of data sources for current state and potential packages
    registry: Registry,
    /// All installed packages across all states
    install_db: db::meta::Database,
    /// All States
    state_db: db::state::Database,
    /// All layouts for all packages
    layout_db: db::layout::Database,
    /// Runtime configuration for the moss package manager
    config: config::Manager,
    /// All of our configured repositories, to seed the [`crate::registry::Registry`]
    repositories: repository::Manager,
    /// Operational scope (real systems, ephemeral, etc)
    scope: Scope,
}

impl Client {
    /// Construct a new ClientBuilder for the given [`Installation`]
    pub fn builder(client_name: impl ToString, installation: Installation) -> ClientBuilder {
        ClientBuilder {
            client_name: client_name.to_string(),
            installation,
            repositories: None,
            system_model_path: None,
            blit_root: None,
        }
    }

    /// Construct a new Client for the given [`Installation`]
    pub fn new(client_name: impl ToString, installation: Installation) -> Result<Client, Error> {
        Self::builder(client_name.to_string(), installation).build()
    }

    /// Returns `true` if this is an ephemeral client
    pub fn is_ephemeral(&self) -> bool {
        matches!(self.scope, Scope::Ephemeral { .. })
    }

    /// Perform package installation
    pub fn install(&mut self, packages: &[&str], yes: bool, simulate: bool) -> Result<install::Timing, Error> {
        install(self, packages, yes, simulate).map_err(|error| Error::Install(Box::new(error)))
    }

    /// Perform package removals
    pub fn remove(&mut self, packages: &[&str], yes: bool, simulate: bool) -> Result<remove::Timing, Error> {
        remove(self, packages, yes, simulate).map_err(|error| Error::Remove(Box::new(error)))
    }

    /// Perform package fetches
    pub fn fetch(&mut self, packages: &[&str], output_dir: &Path, verbose: bool) -> Result<fetch::Timing, Error> {
        fetch(self, packages, output_dir, verbose).map_err(|error| Error::Fetch(Box::new(error)))
    }

    /// Perform a sync
    pub fn sync(&mut self, yes: bool, simulate: bool) -> Result<sync::Timing, Error> {
        sync(self, yes, simulate).map_err(|error| Error::Sync(Box::new(error)))
    }

    /// Transition to an ephemeral client that doesn't record state changes
    /// and blits to a different root.
    ///
    /// This is useful for installing a root to a container (i.e. Boulder) while
    /// using a shared cache.
    ///
    /// Returns an error if `blit_root` is the same as the installation root,
    /// since the system client should always be stateful.
    pub fn ephemeral(self, blit_root: impl Into<PathBuf>) -> Result<Self, Error> {
        let blit_root = blit_root.into();

        if blit_root.exists() && blit_root.canonicalize()? == self.installation.root.canonicalize()? {
            return Err(Error::EphemeralInstallationRoot);
        }

        Ok(Self {
            scope: Scope::Ephemeral { blit_root },
            ..self
        })
    }

    /// Ensures all repositories have been initialized by ensuring their stone indexes
    /// are downloaded and added to the meta db
    pub async fn ensure_repos_initialized(&mut self) -> Result<usize, Error> {
        let num_initialized = self.repositories.ensure_all_initialized().await?;
        self.registry = build_registry(&self.installation, &self.repositories, &self.install_db, &self.state_db)?;
        Ok(num_initialized)
    }

    /// Reload all configured repositories and refreshes their index file, then update
    /// registry with all active repositories.
    pub async fn refresh_repositories(&mut self) -> Result<(), Error> {
        // Reload manager if config sourced to pickup config changes
        // then refresh indexes
        if self.repositories.is_config_source() {
            self.repositories =
                repository::Manager::with_config_manager(self.config.clone(), self.installation.clone())?;
        };
        self.repositories.refresh_all().await?;

        // Rebuild registry
        self.registry = build_registry(&self.installation, &self.repositories, &self.install_db, &self.state_db)?;

        Ok(())
    }

    pub fn verify(&self, yes: bool, verbose: bool) -> Result<(), Error> {
        if self.scope.is_ephemeral() {
            return Err(Error::EphemeralProhibitedOperation);
        }
        verify(self, yes, verbose)?;
        Ok(())
    }

    /// Prune states with the provided [`prune::Strategy`].
    ///
    /// This allows automatic removal of unused states (and their associated assets)
    /// from the disk, acting as a garbage collection facility.
    pub fn prune_states(&self, strategy: prune::Strategy<'_>, yes: bool) -> Result<(), Error> {
        if self.scope.is_ephemeral() {
            return Err(Error::EphemeralProhibitedOperation);
        }

        prune_states(self, strategy, yes)?;

        Ok(())
    }

    /// Prune all cached data that isn't related to any states or active repositories.
    ///
    /// This will remove all downloaded stones & unpacked asset data for packages not
    /// in that set.
    pub fn prune_cache(&self) -> Result<usize, Error> {
        if self.scope.is_ephemeral() {
            return Err(Error::EphemeralProhibitedOperation);
        }

        prune_cache(
            &self.state_db,
            &self.install_db,
            &self.layout_db,
            &self.installation,
            &self.repositories,
        )
        .map_err(Error::Prune)
    }

    /// Resolves the provided id with the underlying registry, returning the first matching [`Package`]
    pub fn resolve_package(&self, package: &package::Id) -> Result<Package, Error> {
        self.registry
            .by_id(package)
            .next()
            .ok_or(Error::MissingMetadata(package.clone()))
    }

    /// Resolves the provided id's with the underlying registry, returning
    /// the first [`Package`] for each id.
    ///
    /// Packages are sorted by name and deduped before returning.
    pub fn resolve_packages<'a>(
        &self,
        packages: impl IntoIterator<Item = &'a package::Id>,
    ) -> Result<Vec<Package>, Error> {
        let mut metadata = packages
            .into_iter()
            .map(|id| self.registry.by_id(id).next().ok_or(Error::MissingMetadata(id.clone())))
            .collect::<Result<Vec<_>, _>>()?;
        metadata.sort_by_key(|p| p.meta.name.to_string());
        metadata.dedup_by_key(|p| p.meta.name.to_string());
        Ok(metadata)
    }

    /// Returns all unique packages which provide the supplied [`Provider`]
    pub fn lookup_packages_by_provider(&self, provider: &Provider, flags: package::Flags) -> Vec<Package> {
        self.registry
            .by_provider(provider, flags)
            .unique_by(|p| p.id.clone())
            .collect()
    }

    /// Return a sorted iterator of packages matching the given flags
    pub fn list_packages(&self, flags: package::Flags) -> impl Iterator<Item = Package> + '_ {
        self.registry.list(flags)
    }

    /// Returns all packages with names containing the provided keyword
    /// and match the given flags
    pub fn search_packages<'a>(
        &'a self,
        keyword: &'a str,
        flags: package::Flags,
    ) -> impl Iterator<Item = Package> + 'a {
        self.registry.by_keyword(keyword, flags)
    }

    /// Activates the provided state and runs system triggers once applied.
    ///
    /// The current state gets archived.\
    /// Returns the old state that was archived.
    pub fn activate_state(&self, id: state::Id, skip_triggers: bool, skip_boot: bool) -> Result<state::Id, Error> {
        // Fetch the new state
        let new = self.state_db.get(id).map_err(|_| Error::StateDoesntExist(id))?;

        // Get old (current) state
        let Some(old) = self.installation.active_state else {
            return Err(Error::NoActiveState);
        };

        if new.id == old {
            return Err(Error::StateAlreadyActive(id));
        }

        let staging_dir = self.installation.staging_dir();

        // Ensure staging dir exists
        if !staging_dir.exists() {
            fs::create_dir(&staging_dir)?;
        }

        // Move new (archived) state to staging
        fs::rename(self.installation.root_path(new.id.to_string()), &staging_dir)?;

        // Promote staging
        self.promote_staging()?;

        // Archive old state
        self.archive_state(old)?;

        // Build VFS from new state selections
        // to build triggers from
        let fstree = self.vfs(new.selections.iter().map(|selection| &selection.package))?;

        if !skip_triggers {
            // Run system triggers
            Self::apply_triggers(TriggerScope::System(&self.installation, &self.scope), &fstree)?;
        }

        if !skip_boot {
            boot::synchronize(self, &new)?;
        }

        Ok(old)
    }

    /// Create a new recorded state from the provided packages
    /// provided packages and write that state ID to the installation
    /// Then blit the filesystem, promote it, finally archiving the active ID
    ///
    /// Returns `None` if the client is ephemeral
    pub fn new_state(&self, selections: &[Selection], summary: impl ToString) -> Result<Option<State>, Error> {
        let _guard = signal::ignore([Signal::SIGINT])?;
        let _fd = signal::inhibit(
            vec!["shutdown", "sleep", "idle", "handle-lid-switch"],
            "moss".into(),
            "Applying new state".into(),
            "block".into(),
        );

        let explicit_packages =
            self.resolve_packages(selections.iter().filter_map(|s| s.explicit.then_some(&s.package)))?;
        let system_model = update_or_create_system_model(
            self.installation.system_model.clone(),
            &self.repositories,
            &explicit_packages,
        )?;

        let timer = Instant::now();

        let state_span = info_span!(
            "progress",
            phase = summary.to_string().to_lowercase(),
            event_type = "progress"
        );
        let _state_guard = state_span.enter();
        info!(
            total_items = selections.len(),
            progress = 0.0,
            event_type = "progress_start",
        );

        let old_state = self.installation.active_state;

        let fstree = self.blit_root(selections.iter().map(|s| &s.package))?;

        let result = match &self.scope {
            Scope::Stateful => {
                // Add to db
                let state = self.state_db.add(selections, Some(&summary.to_string()), None)?;

                self.apply_stateful_blit(fstree, &state, old_state, system_model)?;

                Ok(Some(state))
            }
            Scope::Ephemeral { blit_root } => {
                self.apply_ephemeral_blit(fstree, blit_root, system_model)?;

                Ok(None)
            }
        };

        info!(
            duration_ms = timer.elapsed().as_millis(),
            items_processed = selections.len(),
            progress = 1.0,
            event_type = "progress_completed",
        );

        result
    }

    /// Apply all triggers with the given scope, wrapping with a progressbar.
    fn apply_triggers(scope: TriggerScope<'_>, fstree: &vfs::Tree<fstree::PendingFile>) -> Result<(), postblit::Error> {
        let triggers = postblit::triggers(scope, fstree)?;

        let progress = ProgressBar::new(triggers.len() as u64).with_style(
            ProgressStyle::with_template("\n|{bar:20.green/blue}| {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("■≡=- "),
        );

        let phase_name = match &scope {
            TriggerScope::Transaction(..) => {
                progress.set_message("Running transaction-scope triggers");
                "transaction-scope-triggers"
            }
            TriggerScope::System(..) => {
                progress.set_message("Running system-scope triggers");
                "system-scope-triggers"
            }
        };

        let timer = Instant::now();

        info!(
            phase = phase_name,
            total_items = triggers.len(),
            progress = 0.0,
            event_type = "progress_start",
        );

        for (i, trigger) in progress.wrap_iter(triggers.iter()).enumerate() {
            trigger.execute()?;

            info!(
                progress = (i + 1) as f32 / triggers.len() as f32,
                current = i + 1,
                total = triggers.len(),
                event_type = "progress_update",
                "Executing `{}`",
                trigger.handler()
            );
        }

        info!(
            phase = phase_name,
            duration_ms = timer.elapsed().as_millis(),
            items_processed = triggers.len(),
            progress = 1.0,
            event_type = "progress_completed",
        );

        progress.finish_and_clear();

        Ok(())
    }

    pub fn apply_stateful_blit(
        &self,
        fstree: vfs::Tree<fstree::PendingFile>,
        state: &State,
        old_state: Option<state::Id>,
        system_model: SystemModel,
    ) -> Result<(), Error> {
        record_state_id(&self.installation.staging_dir(), state.id)?;
        record_os_release(&self.installation.staging_dir())?;
        record_system_model(&self.installation.staging_dir(), system_model)?;

        create_root_links(&self.installation.isolation_dir())?;

        // The container running triggers expects /etc to exist
        let root_etc = self.installation.root.join("etc");
        fs::create_dir_all(root_etc)?;

        let isolation_etc = self.installation.isolation_dir().join("etc");
        fs::create_dir_all(isolation_etc)?;

        // Apply transaction triggers
        Self::apply_triggers(TriggerScope::Transaction(&self.installation, &self.scope), &fstree)?;

        // Staging is only used with [`Scope::Stateful`]
        self.promote_staging()?;

        // Now we got it staged, we need working rootfs
        create_root_links(&self.installation.root)?;

        if let Some(id) = old_state {
            self.archive_state(id)?;
        }

        // At this point we're allowed to run system triggers
        Self::apply_triggers(TriggerScope::System(&self.installation, &self.scope), &fstree)?;

        boot::synchronize(self, state)?;

        Ok(())
    }

    pub fn apply_ephemeral_blit(
        &self,
        fstree: vfs::Tree<fstree::PendingFile>,
        blit_root: &Path,
        system_model: SystemModel,
    ) -> Result<(), Error> {
        record_os_release(blit_root)?;
        record_system_model(blit_root, system_model)?;

        create_root_links(blit_root)?;
        create_root_links(&self.installation.isolation_dir())?;

        // The container running triggers expects /etc to exist
        let etc = blit_root.join("etc");
        fs::create_dir_all(etc)?;

        // ephemeral tx triggers
        Self::apply_triggers(TriggerScope::Transaction(&self.installation, &self.scope), &fstree)?;
        // ephemeral system triggers
        Self::apply_triggers(TriggerScope::System(&self.installation, &self.scope), &fstree)?;

        Ok(())
    }

    /// "Activate" the staging tree
    /// In practice, this means we perform an atomic swap of the `/usr` directory on the
    /// host filesystem with the `/usr` tree within the transaction tree.
    ///
    /// This is performed using `renameat2` and results in instantly available, atomically updated
    /// `/usr`. In combination with the mandated "`/usr`` merge" and statelessness approach of
    /// our project, it provides a unique atomic upgrade strategy.
    fn promote_staging(&self) -> Result<(), Error> {
        if self.scope.is_ephemeral() {
            return Err(Error::EphemeralProhibitedOperation);
        }

        let usr_target = self.installation.root.join("usr");
        let usr_source = self.installation.staging_path("usr");

        // Create the target tree
        if !usr_target.try_exists()? {
            fs::create_dir_all(&usr_target)?;
        }

        // Now swap staging with live
        atomic_swap(&usr_source, &usr_target).map_err(Error::AtomicSwap)?;

        Ok(())
    }

    /// Archive old states (currently not "activated") into their respective tree
    fn archive_state(&self, id: state::Id) -> Result<(), Error> {
        if self.scope.is_ephemeral() {
            return Err(Error::EphemeralProhibitedOperation);
        }

        // After promotion, the old active /usr is now in staging/usr
        let usr_target = self.installation.root_path(id.to_string()).join("usr");
        let usr_source = self.installation.staging_path("usr");
        if let Some(parent) = usr_target.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }
        // hot swap the staging/usr into the root/$id/usr
        fs::rename(usr_source, &usr_target)?;
        Ok(())
    }

    /// Download & unpack the provided packages. Packages already cached will be validated & skipped.
    pub async fn cache_packages<T>(&self, packages: &[T]) -> Result<(), Error>
    where
        T: Borrow<Package>,
    {
        // Setup progress bar
        let multi_progress = MultiProgress::new();

        // Add bar to track total package counts
        let total_progress = multi_progress.add(
            ProgressBar::new(packages.len() as u64).with_style(
                ProgressStyle::with_template("\n|{bar:20.cyan/blue}| {pos}/{len}")
                    .unwrap()
                    .progress_chars("■≡=- "),
            ),
        );
        total_progress.tick();

        let unpacking_in_progress = cache::UnpackingInProgress::default();

        // Download and unpack each package
        let cached = stream::iter(packages)
            .map(|package| async {
                let package: &Package = package.borrow();

                // Setup the progress bar and set as downloading
                let progress_bar = multi_progress.insert_before(
                    &total_progress,
                    ProgressBar::new(package.meta.download_size.unwrap_or_default())
                        .with_message(format!(
                            "{} {}",
                            "Downloading".blue(),
                            package.meta.name.as_str().bold(),
                        ))
                        .with_style(
                            ProgressStyle::with_template(
                                " {spinner} |{percent:>3}%| {wide_msg} {binary_bytes_per_sec:>.dim} ",
                            )
                            .unwrap()
                            .tick_chars("--=≡■≡=--"),
                        ),
                );
                progress_bar.enable_steady_tick(Duration::from_millis(150));

                // Download and update progress
                let download = cache::fetch(&package.meta, &self.installation, |progress| {
                    progress_bar.inc(progress.delta);
                    info!(
                        progress = progress.completed as f32 / progress.total as f32,
                        current = progress.completed as usize,
                        total = progress.total as usize,
                        event_type = "progress_update",
                        "Downloading {}",
                        package.meta.name
                    );
                })
                .await
                .map_err(|err| Error::CacheFetch(err, package.meta.name.clone()))?;

                let is_cached = download.was_cached;

                // Move rest of blocking code to threadpool

                let multi_progress = multi_progress.clone();
                let total_progress = total_progress.clone();
                let unpacking_in_progress = unpacking_in_progress.clone();
                let package = (*package).clone();
                let current_span = tracing::Span::current();

                runtime::unblock(move || {
                    let _guard = current_span.enter();
                    let package_name = &package.meta.name;
                    let download_path = download.path().to_owned();

                    // Set progress to unpacking
                    progress_bar.set_message(format!("{} {}", "Unpacking".yellow(), package_name.to_string().bold()));
                    progress_bar.set_length(1000);
                    progress_bar.set_position(0);

                    // Unpack and update progress
                    let unpacked = download
                        .unpack(unpacking_in_progress.clone(), {
                            let progress_bar = progress_bar.clone();
                            let package_name = package_name.clone();

                            move |progress| {
                                progress_bar.set_position((progress.pct() * 1000.0) as u64);
                                info!(
                                    progress = progress.completed as f32 / progress.total as f32,
                                    current = progress.completed as usize,
                                    total = progress.total as usize,
                                    event_type = "progress_update",
                                    "Unpacking {package_name}",
                                );
                            }
                        })
                        .map_err(|err| Error::CacheUnpack(err, package_name.clone(), download_path))?;

                    // Remove this progress bar
                    progress_bar.finish();
                    multi_progress.remove(&progress_bar);

                    let cached_tag = is_cached
                        .then_some(format!("{}", " (cached)".dim()))
                        .unwrap_or_default();

                    // Write installed line
                    multi_progress.suspend(|| {
                        println!(
                            "{} {}{cached_tag}",
                            "Installed".green(),
                            package_name.to_string().bold()
                        );
                    });

                    // Inc total progress by 1
                    total_progress.inc(1);

                    info!(
                        progress = total_progress.position() as f32 / total_progress.length().unwrap_or(1) as f32,
                        current = total_progress.position() as usize,
                        total = total_progress.length().unwrap_or(0) as usize,
                        event_type = "progress_update",
                        "Cached {}",
                        package_name
                    );

                    Ok((package, unpacked)) as Result<(Package, cache::UnpackedAsset), Error>
                })
                .await
            })
            // Use max network concurrency since we download files here
            .buffer_unordered(environment::MAX_NETWORK_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?;

        // Add layouts & packages to DBs
        runtime::unblock({
            let layout_db = self.layout_db.clone();
            let install_db = self.install_db.clone();
            move || {
                total_progress.set_position(0);
                total_progress.set_length(2);
                total_progress.set_message("Storing DB layouts");
                total_progress.tick();

                // Add layouts
                layout_db.batch_add(cached.iter().flat_map(|(p, u)| {
                    u.payloads
                        .iter()
                        .flat_map(StoneDecodedPayload::layout)
                        .flat_map(|p| p.body.as_slice())
                        .map(|layout| (&p.id, layout))
                }))?;

                total_progress.inc(1);
                total_progress.set_message("Storing DB packages");

                // Add packages
                install_db.batch_add(cached.into_iter().map(|(p, _)| (p.id, p.meta)).collect())?;

                total_progress.inc(1);

                Ok::<_, Error>(())
            }
        })
        .await?;

        // Remove progress
        multi_progress.clear()?;

        Ok(())
    }

    /// Build a [`vfs::Tree`] for the specified packages.
    ///
    /// Returns a newly built [`vfs::Tree`] that can be used in
    /// the creation of fstrees.
    pub fn vfs<'a>(
        &self,
        packages: impl IntoIterator<Item = &'a package::Id>,
    ) -> Result<vfs::Tree<fstree::PendingFile>, Error> {
        fstree::vfs(self.layout_db.query(packages)?).map_err(Error::BuildVfsTree)
    }

    /// Blit the packages to a filesystem root
    ///
    /// This functionality is core to all moss filesystem transactions, forming the entire
    /// staging logic. For all the [`crate::package::Id`] present in the staging state,
    /// query their stored [`StonePayloadLayoutBody`] and cache into a [`vfs::Tree`].
    ///
    /// The new `/usr` filesystem is written in optimal order to a staging tree by making
    /// use of the "at" family of functions (`mkdirat`, `linkat`, etc) with relative directory
    /// file descriptors, linking files from the assets store to provide deduplication.
    ///
    /// This provides a very quick means to generate a hardlinked "snapshot" on-demand,
    /// which can then be activated via [`Self::promote_staging`]
    pub fn blit_root<'a>(
        &self,
        packages: impl IntoIterator<Item = &'a package::Id>,
    ) -> Result<vfs::Tree<fstree::PendingFile>, Error> {
        let blit_target = match &self.scope {
            Scope::Stateful => self.installation.staging_dir(),
            Scope::Ephemeral { blit_root } => blit_root.to_owned(),
        };

        let fstree = self.vfs(packages)?;

        fstree::native::blit_root(&self.installation, &fstree, &blit_target).map_err(Error::Blit)?;

        Ok(fstree)
    }

    fn load_or_create_system_model(&self, path: PathBuf, state: &State) -> Result<SystemModel, Error> {
        match system_model::load(&path).map_err(Error::LoadSystemModel)? {
            Some(system_model) => Ok(system_model.into()),
            None => {
                let active_repos = self
                    .repositories
                    .active()
                    .map(|repo| (repo.id, repo.repository))
                    .collect::<repository::Map>();

                let packages = self
                    .resolve_packages(state.selections.iter().filter_map(|s| s.explicit.then_some(&s.package)))?
                    .into_iter()
                    .map(|package| Provider::package_name(package.meta.name.as_str()))
                    .collect();

                Ok(system_model::create(active_repos, packages))
            }
        }
    }

    /// Export the provided state as a [`SystemModel`]
    pub fn export_state(&self, state: state::Id) -> Result<SystemModel, Error> {
        let state = self.state_db.get(state)?;
        let is_active = self.installation.active_state == Some(state.id);

        let path = if is_active {
            self.installation.root.join("usr/lib/system-model.kdl")
        } else {
            self.installation
                .root_path(state.id.to_string())
                .join("usr/lib/system-model.kdl")
        };

        self.load_or_create_system_model(path, &state)
    }

    /// Print boot status to stdout
    pub fn print_boot_status(&self) -> Result<(), Error> {
        boot::print_status(&self.installation).map_err(Error::Boot)
    }

    /// Synchronize boot for the active state
    pub fn synchronize_boot(&self) -> Result<(), Error> {
        let Some(state_id) = self.installation.active_state else {
            return Err(Error::NoActiveState);
        };

        let state = self.state_db.get(state_id)?;

        boot::synchronize(self, &state).map_err(Error::Boot)
    }

    /// List all states for this moss [`Installation`]
    pub fn list_states(&self) -> Result<Vec<State>, Error> {
        self.state_db
            .list_ids()?
            .into_iter()
            .map(|(id, _)| self.state_db.get(id).map_err(Error::Db))
            .collect()
    }

    /// Return a [`State`] for the provided state id
    pub fn get_state(&self, id: state::Id) -> Result<State, Error> {
        self.state_db.get(id).map_err(Error::Db)
    }

    /// Return the active [`State`] for this moss [`Installation`]
    pub fn get_active_state(&self) -> Result<Option<State>, Error> {
        match self.installation.active_state {
            Some(id) => self.get_state(id).map(Some),
            None => Ok(None),
        }
    }

    /// List all layout entries cached by this moss [`Installation`], which
    /// includes packages installed across all states
    pub fn list_layouts(&self) -> Result<Vec<(package::Id, StonePayloadLayoutRecord)>, Error> {
        self.layout_db.all().map_err(Error::Db)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn mocked(
        installation: Installation,
        registry: Registry,
        layouts: Vec<(package::Id, StonePayloadLayoutRecord)>,
    ) -> Result<Client, Error> {
        let config = config::Manager::system(&installation.root, "moss");
        let install_db = db::meta::Database::new(":memory:")?;
        let state_db = db::state::Database::new(":memory:")?;
        let layout_db = db::layout::Database::new(":memory:")?;

        layout_db.batch_add(layouts.iter().map(|(id, layout)| (id, layout)))?;
        let repositories = repository::Manager::with_config_manager(config.clone(), installation.clone())?;

        Ok(Client {
            config,
            installation,
            repositories,
            registry,
            install_db,
            state_db,
            layout_db,
            scope: Scope::Stateful,
        })
    }
}

/// Add root symlinks & os-release file
fn create_root_links(root: &Path) -> io::Result<()> {
    let links = vec![
        ("usr/sbin", "sbin"),
        ("usr/bin", "bin"),
        ("usr/lib", "lib"),
        ("usr/lib", "lib64"),
        ("usr/lib32", "lib32"),
    ];

    'linker: for (source, target) in links.into_iter() {
        let final_target = root.join(target);
        let staging_target = root.join(format!("{target}.next"));

        if staging_target.exists() {
            fs::remove_file(&staging_target)?;
        }

        if final_target.exists() && final_target.is_symlink() && final_target.read_link()?.to_string_lossy() == source {
            continue 'linker;
        }
        symlink(source, &staging_target)?;
        fs::rename(staging_target, final_target)?;
    }

    Ok(())
}

/// syscall based wrapper for renameat2 so we can support musl libc which
/// unfortunately does not expose the API.
/// largely modelled on existing renameat2 API in nix crate
fn atomic_swap<A: ?Sized + NixPath, B: ?Sized + NixPath>(old_path: &A, new_path: &B) -> nix::Result<()> {
    let result = old_path.with_nix_path(|old| {
        new_path.with_nix_path(|new| unsafe {
            syscall(
                SYS_renameat2,
                AT_FDCWD,
                old.as_ptr(),
                AT_FDCWD,
                new.as_ptr(),
                RENAME_EXCHANGE,
            )
        })
    })?? as i32;
    Errno::result(result).map(drop)
}

fn record_state_id(root: &Path, state: state::Id) -> Result<(), Error> {
    let usr = root.join("usr");
    fs::create_dir_all(&usr)?;
    let state_path = usr.join(".stateID");
    fs::write(state_path, state.to_string())?;
    Ok(())
}

/// Record the operating system release info
/// Requires `os-info.json` to be present in the root, otherwise
/// we'll somewhat spitefully generate a generic os-release.
fn record_os_release(root: &Path) -> Result<(), Error> {
    let os_info_path = root.join("usr").join("lib").join("os-info.json");
    let os_release_data = match os_info::load_os_info_from_path(os_info_path) {
        Ok(ref info) => {
            let os_rel: os_info::OsRelease = info.into();
            os_rel.to_string()
        }
        Err(_) => {
            // Fallback to a generic os-release to break the system
            // TLDR: Implement your OS properly.
            format!(
                r#"NAME="Unbranded OS"
                VERSION="{version}"
                ID="unbranded-os"
                VERSION_CODENAME={version}
                VERSION_ID="{version}"
                PRETTY_NAME="Unbranded OS {version} - I forgot to add os-info.json"
                HOME_URL="https://github.com/AerynOS/os-info"
                BUG_REPORT_URL="https://.com""#,
                version = "no-os-info.json"
            )
        }
    };

    // It's possible this doesn't exist if
    // we remove all packages (=
    let dir = root.join("usr").join("lib");
    if !dir.exists() {
        fs::create_dir(&dir)?;
    }

    fs::write(dir.join("os-release"), os_release_data)?;

    Ok(())
}

fn update_or_create_system_model(
    current: Option<LoadedSystemModel>,
    repositories: &repository::Manager,
    packages: &[Package],
) -> Result<SystemModel, Error> {
    let active_repos = repositories
        .active()
        .map(|repo| (repo.id, repo.repository))
        .collect::<repository::Map>();

    match current {
        // Update existing w/ incoming packages
        Some(existing) => SystemModel::from(existing)
            .sync_packages(packages)
            .map_err(Error::UpdateSystemModel),

        // Generate a fresh system-model file
        None => {
            let packages = packages
                .iter()
                .map(|package| Provider::package_name(package.meta.name.as_str()))
                .collect();

            Ok(system_model::create(active_repos, packages))
        }
    }
}

fn record_system_model(root: &Path, system_model: SystemModel) -> Result<(), Error> {
    let dir = root.join("usr").join("lib");

    if !dir.exists() {
        fs::create_dir(&dir)?;
    }

    fs::write(dir.join("system-model.kdl"), system_model.encoded())?;

    Ok(())
}

#[derive(Clone, Debug)]
enum Scope {
    Stateful,
    Ephemeral { blit_root: PathBuf },
}

impl Scope {
    fn is_ephemeral(&self) -> bool {
        matches!(self, Self::Ephemeral { .. })
    }
}

/// Build a [`crate::registry::Registry`] during client initialisation
///
/// # Arguments
///
/// * `installation` - Describe our installation target tree
/// * `repositories` - Configured repositories to laoad [`crate::registry::Plugin::Repository`]
/// * `installdb`    - Installation database opened in the installation tree
/// * `statedb`      - State database opened in the installation tree
fn build_registry(
    installation: &Installation,
    repositories: &repository::Manager,
    installdb: &db::meta::Database,
    statedb: &db::state::Database,
) -> Result<Registry, Error> {
    let state = match installation.active_state {
        Some(id) => Some(statedb.get(id)?),
        None => None,
    };

    let mut registry = Registry::default();

    registry.add_plugin(Plugin::Cobble(plugin::Cobble::default()));
    registry.add_plugin(Plugin::Active(plugin::Active::new(state, installdb.clone())));

    for repo in repositories.active() {
        registry.add_plugin(Plugin::Repository(plugin::Repository::new(repo)));
    }

    Ok(registry)
}

/// Client-relevant error mapping type
#[derive(Debug, Error)]
pub enum Error {
    #[error("root must have an active state")]
    NoActiveState,
    #[error("state {0} already active")]
    StateAlreadyActive(state::Id),
    #[error("state {0} doesn't exist")]
    StateDoesntExist(state::Id),
    #[error("No metadata found for package {0:?}")]
    MissingMetadata(package::Id),
    #[error("Ephemeral client not allowed on installation root")]
    EphemeralInstallationRoot,
    #[error("Operation not allowed with ephemeral client")]
    EphemeralProhibitedOperation,
    #[error("installation")]
    Installation(#[from] installation::Error),
    #[error("fetch package {1}")]
    CacheFetch(#[source] cache::FetchError, package::Name),
    #[error("unpack package {1}, file {2}")]
    CacheUnpack(#[source] cache::UnpackError, package::Name, PathBuf),
    #[error("repository manager")]
    Repository(#[from] repository::manager::Error),
    #[error("db")]
    Db(#[from] db::Error),
    #[error("prune")]
    Prune(#[from] prune::Error),
    #[error("io")]
    Io(#[from] io::Error),
    #[error("filesystem")]
    Filesystem(#[from] vfs::tree::Error),
    #[error("blit")]
    Blit(#[source] fstree::native::Error),
    #[error("postblit")]
    PostBlit(#[from] postblit::Error),
    #[error("boot")]
    Boot(#[from] boot::Error),
    /// Had issues processing user-provided string input
    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),
    /// The operation was explicitly cancelled at the user's request
    #[error("cancelled")]
    Cancelled,
    #[error("ignore signals during blit")]
    BlitSignalIgnore(#[from] signal::Error),
    #[error("load system model")]
    LoadSystemModel(#[from] system_model::LoadError),
    #[error("update system model")]
    UpdateSystemModel(#[from] system_model::UpdateError),
    #[error("install")]
    Install(#[source] Box<install::Error>),
    #[error("remove")]
    Remove(#[source] Box<remove::Error>),
    #[error("fetch")]
    Fetch(#[source] Box<fetch::Error>),
    #[error("sync")]
    Sync(#[source] Box<sync::Error>),
    #[error("system model doesn't exist at {0:?}")]
    ImportSystemModelDoesntExist(PathBuf),
    #[error("build vfs tree")]
    BuildVfsTree(#[source] vfs::tree::Error),
    #[error("atomic swap")]
    AtomicSwap(#[source] Errno),
}
