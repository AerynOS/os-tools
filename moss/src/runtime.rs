// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    future::Future,
    io,
    sync::{OnceLock, RwLock},
};

use tokio::runtime::{self, Handle};

static RUNTIME: OnceLock<RwLock<Option<Runtime>>> = OnceLock::new();

/// One-time initialisation of the tokio runtime
pub fn init() -> Guard {
    let lock = RUNTIME.get_or_init(Default::default);
    *lock.write().unwrap() = Some(Runtime::new().expect("build runtime"));
    Guard
}

/// Explicit destroy support for the runtime.
/// This allows us to get rid of the runtime when multithreading is not desirable
/// such as entering a [`container::Container`]
fn destroy() {
    let rt = RUNTIME
        .get()
        .unwrap()
        .write()
        .unwrap()
        .take()
        .expect("runtime initialized");
    drop(rt);
}

/// The Guard provides a scoped token to utilise the Runtime
#[must_use = "runtime is dropped with guard"]
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        destroy();
    }
}

/// Lifetime management handle for the runtime
struct Runtime(runtime::Runtime);

impl Runtime {
    /// Construct a new Runtime on the current thread
    fn new() -> io::Result<Self> {
        Ok(Self(runtime::Builder::new_current_thread().enable_all().build()?))
    }
}

/// Run the provided future on the global runtime if it's
/// been initialized, otherwise it creates a temporary runtime
/// to run the future on and then fully drops it.
pub fn block_on<T, F>(task: F) -> T
where
    F: Future<Output = T>,
{
    if let Some(_lock) = RUNTIME.get() {
        let _guard = _lock.read().unwrap();

        if let Some(rt) = &*_guard {
            return rt.0.block_on(task);
        }
    }

    let temp_rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("temp runtime");
    temp_rt.block_on(task)
}

/// Runs the provided function on an executor dedicated to blocking.
pub async fn unblock<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let handle = Handle::current();
    handle.spawn_blocking(f).await.expect("spawn blocking")
}
