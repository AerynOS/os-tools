// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::future::Future;

use tokio::runtime::{self, Handle};

/// Run the provided future on a single use runtime that
/// is dropped before returning the completed task
pub fn block_on<T, F>(task: F) -> T
where
    F: Future<Output = T>,
{
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
