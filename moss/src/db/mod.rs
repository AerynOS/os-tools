// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use thiserror::Error;

pub mod layout;
pub mod meta;
mod migrations;
pub mod state;

#[derive(Clone)]
struct Connection(Arc<Mutex<rusqlite::Connection>>);

impl Connection {
    fn new(connection: rusqlite::Connection) -> Self {
        Self(Arc::new(Mutex::new(connection)))
    }

    fn exec<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> T) -> T {
        let _guard = self.0.lock().expect("mutex guard");
        f(&_guard)
    }

    fn exec_mut<T>(&self, f: impl FnOnce(&mut rusqlite::Connection) -> T) -> T {
        let mut _guard = self.0.lock().expect("mutex guard");
        f(&mut _guard)
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection").finish()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Row not found")]
    RowNotFound,
    #[error("invalid {0}: {1}")]
    Decode(&'static str, String),
    #[error(transparent)]
    Dbms(#[from] rusqlite::Error),
}
