// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Operations that happen post-blit (primarily, triggers within container)
//! Note that we support transaction scope and system scope triggers, invoked
//! before `/usr` is activated and after, respectively.
//!
//! Note that currently we only load from `/usr/share/moss/triggers/{tx,sys.d}/*.yaml`
//! and do not yet support local triggers
use std::{
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::Installation;
use container::Container;
use itertools::Itertools;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Deserialize;
use thiserror::Error;
use tracing::{error, warn};
use triggers::format::{CompiledHandler, Handler, Trigger};

use super::PendingFile;

/// Transaction trigger wrapper
/// These are loaded from `/usr/share/moss/triggers/tx.d/*.yaml`
#[derive(Deserialize, Debug)]
struct TransactionTrigger(Trigger);

impl config::Config for TransactionTrigger {
    fn domain() -> String {
        "tx".into()
    }
}

/// System trigger wrapper
/// These triggers are loaded from `/usr/share/moss/triggers/sys.d/*.yaml`
#[derive(Deserialize, Debug)]
struct SystemTrigger(Trigger);

impl config::Config for SystemTrigger {
    fn domain() -> String {
        "sys".into()
    }
}

/// The trigger scope determines the environment that the trigger runs in
#[derive(Clone, Copy, Debug)]
pub(super) enum TriggerScope<'a> {
    /// A transaction trigger, isolated to `/usr`
    Transaction(&'a Installation, &'a super::Scope),

    /// A system trigger with reduced sandboxing, capable of writes outside `/usr`
    System(&'a Installation, &'a super::Scope),
}

impl TriggerScope<'_> {
    // Determine the correct root directory
    fn root_dir(&self) -> PathBuf {
        match self {
            TriggerScope::Transaction(install, scope) => match scope {
                super::Scope::Stateful => install.staging_dir().clone(),
                super::Scope::Ephemeral { blit_root } => blit_root.clone(),
            },
            TriggerScope::System(install, scope) => match scope {
                super::Scope::Stateful => install.root.clone(),
                super::Scope::Ephemeral { blit_root } => blit_root.clone(),
            },
        }
    }

    /// Join "host" paths, outside the staging filesystem. Ensure no sandbox break for ephemeral
    fn host_path(&self, path: impl AsRef<Path>) -> PathBuf {
        match self {
            TriggerScope::Transaction(install, scope) => match scope {
                super::Scope::Stateful => install.root.join(path),
                super::Scope::Ephemeral { blit_root } => blit_root.join(path),
            },
            TriggerScope::System(install, scope) => match scope {
                super::Scope::Stateful => install.root.join(path),
                super::Scope::Ephemeral { blit_root } => blit_root.join(path),
            },
        }
    }

    /// Join guest paths, inside the staging filesystem. Ensure no sandbox break for ephemeral
    fn guest_path(&self, path: impl AsRef<Path>) -> PathBuf {
        match self {
            TriggerScope::Transaction(install, scope) => match scope {
                super::Scope::Stateful => install.staging_path(path),
                super::Scope::Ephemeral { blit_root } => blit_root.join(path),
            },
            TriggerScope::System(install, scope) => match scope {
                super::Scope::Stateful => install.root.join(path),
                super::Scope::Ephemeral { blit_root } => blit_root.join(path),
            },
        }
    }
}

/// Condensed type for loaded triggers with scope and executor
pub(super) struct TriggerRunner {
    trigger: CompiledHandler,
}

/// Progress callback handler
#[derive(Debug, Clone)]
pub struct Progress<'a> {
    pub completed: u64,
    pub item: &'a str,
}

/// Load all triggers matching the given scope and staging filesystem, return in batches
/// suitable for concurrent/parallel processing.
///
/// # Arguments
///
/// * `scope`  - Trigger execution scope
/// * `fstree` - Virtual filesystem tree populated with records of the staging filesystem
pub(super) fn triggers<'a>(
    scope: TriggerScope<'a>,
    fstree: &vfs::tree::Tree<PendingFile>,
) -> Result<Vec<Vec<TriggerRunner>>, Error> {
    // Pre-calculate trigger root path once
    let trigger_root = {
        let mut path = PathBuf::with_capacity(50);
        path.push("usr");
        path.push("share");
        path.push("moss");
        path.push("triggers");
        path
    };

    let full_trigger_path = scope.root_dir().join(&trigger_root);

    // Load appropriate triggers from their locations and convert back to a vec of Trigger
    let triggers = match scope {
        TriggerScope::Transaction(..) => config::Manager::custom(&full_trigger_path)
            .load::<TransactionTrigger>()
            .into_iter()
            .map(|t| t.0)
            .collect_vec(),
        TriggerScope::System(..) => config::Manager::custom(&full_trigger_path)
            .load::<SystemTrigger>()
            .into_iter()
            .map(|t| t.0)
            .collect_vec(),
    };

    // Load trigger collection, process all the paths, convert to scoped TriggerRunner vec
    let mut collection = triggers::Collection::new(triggers.iter())?;
    collection.process_paths(fstree.iter().map(|m| m.to_string()));
    let batches = collection
        .bake_in_stages()?
        .into_iter()
        .map(|batch| batch.into_iter().map(|trigger| TriggerRunner { trigger }).collect_vec())
        .collect_vec();
    Ok(batches)
}

/// Execute triggers based on TriggerScope
///
/// Execute either transaction or system scope triggers using container sandboxing as necessary
pub fn execute_triggers(
    scope: TriggerScope<'_>,
    triggers: &[Vec<TriggerRunner>],
    on_progress: impl Fn(Progress<'_>) + Send + Sync,
) -> Result<(), Error> {
    match scope {
        scope @ TriggerScope::Transaction(install, _) => {
            execute_transaction_triggers(install, scope, triggers, &on_progress)?;
        }
        scope @ TriggerScope::System(install, _) => {
            execute_system_triggers(install, scope, triggers, &on_progress)?;
        }
    };

    Ok(())
}

/// Execute transaction triggers
///
/// Transaction triggers are run via sandboxing ([`container::Container`]) to limit their
/// system view, and limit write access. Each batch of triggers are executed in parallel
/// to speed up execution time.
fn execute_transaction_triggers<P>(
    install: &Installation,
    scope: TriggerScope<'_>,
    triggers: &[Vec<TriggerRunner>],
    on_progress: P,
) -> Result<(), Error>
where
    P: Fn(Progress<'_>) + Send + Sync,
{
    // TODO: Add caching support via /var/
    let isolation = Container::new(install.isolation_dir())
        .networking(false)
        .bind_ro(scope.host_path("etc"), "/etc")
        .bind_rw(scope.guest_path("usr"), "/usr")
        .work_dir("/");

    isolation.run(|| execute_triggers_directly(triggers, &on_progress))?;

    Ok(())
}

/// Execute system triggers
///
/// System triggers will execute without any sandboxing when moss is used directly against the
/// live root filesystem, and will force sandboxing when using a non-`/` root (such as using the
/// `-D argument with `moss install`). Each batch of triggers is executed in parallel to speed up
/// execution time.
fn execute_system_triggers<P>(
    install: &Installation,
    scope: TriggerScope<'_>,
    triggers: &[Vec<TriggerRunner>],
    on_progress: P,
) -> Result<(), Error>
where
    P: Fn(Progress<'_>) + Send + Sync,
{
    // OK, if the root == `/` then we can run directly, otherwise we need to containerise with RW.
    if install.root.to_string_lossy() == "/" {
        execute_triggers_directly(triggers, on_progress)?;
    } else {
        let isolation = Container::new(install.isolation_dir())
            .networking(false)
            .bind_rw(scope.host_path("etc"), "/etc")
            .bind_rw(scope.guest_path("usr"), "/usr")
            .work_dir("/");

        isolation.run(|| execute_triggers_directly(triggers, &on_progress))?;
    }
    Ok(())
}

impl TriggerRunner {
    pub fn handler(&self) -> &Handler {
        self.trigger.handler()
    }
}

/// Internal executor for triggers.
fn execute_triggers_directly<P>(triggers: &[Vec<TriggerRunner>], on_progress: P) -> Result<(), Error>
where
    P: Fn(Progress<'_>) + Send + Sync,
{
    let rayon_runtime = rayon::ThreadPoolBuilder::new().build().expect("rayon runtime");

    let counter = AtomicUsize::new(0);

    rayon_runtime.install(|| {
        triggers.iter().try_for_each(|batch| {
            batch.par_iter().try_for_each(|trigger| {
                let res = execute_trigger_directly(&trigger.trigger);
                let completed = counter.fetch_add(1, Ordering::Relaxed);
                (on_progress)(Progress {
                    completed: completed as u64,
                    item: match trigger.handler() {
                        Handler::Run { run, .. } => run,
                        Handler::Delete { .. } => "delete operation",
                    },
                });
                res
            })
        })
    })?;
    Ok(())
}

/// Internal executor for individual triggers.
fn execute_trigger_directly(trigger: &CompiledHandler) -> Result<(), Error> {
    match trigger.handler() {
        Handler::Run { run, args } => {
            let cmd = process::Command::new(run).args(args).current_dir("/").output()?;

            if let Some(code) = cmd.status.code() {
                if code != 0 {
                    // Convert outputs once and reuse
                    let stdout = String::from_utf8_lossy(&cmd.stdout);
                    let stderr = String::from_utf8_lossy(&cmd.stderr);

                    warn!(
                        command = run,
                        args = ?args,
                        exit_code = code,
                        stdout = %stdout,
                        stderr = %stderr,
                        "Trigger exited with non-zero status code"
                    );
                }
            } else {
                error!(
                    command = run,
                    args = ?args,
                    "Failed to execute trigger"
                );
            }
        }
        Handler::Delete { .. } => todo!(),
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("container")]
    Container(#[from] container::Error),

    #[error("triggers")]
    Triggers(#[from] triggers::Error),

    #[error("io")]
    IO(#[from] std::io::Error),
}
