// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Signal handling

use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet};
use thiserror::Error;
use zbus::message::{self};

pub use nix::sys::signal::Signal;

/// Ignore the provided signals until [`Guard`] is dropped
pub fn ignore(signals: impl IntoIterator<Item = Signal>) -> Result<Guard, Error> {
    Ok(Guard(
        signals
            .into_iter()
            .map(|signal| {
                let action = unsafe {
                    sigaction(
                        signal,
                        &SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty()),
                    )
                }
                .map_err(Error::Ignore)?;

                Ok(PrevHandler { signal, action })
            })
            .collect::<Result<_, Error>>()?,
    ))
}

// https://www.freedesktop.org/wiki/Software/systemd/inhibit/
pub fn inhibit(what: Vec<&str>, who: String, why: String, mode: String) -> Result<message::Body, Error> {
    let conn = zbus::blocking::Connection::system()?;
    let msg = conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "Inhibit",
        &(what.join(":"), who, why, mode),
    )?;
    let fd = msg.body();
    Ok(fd)
}

/// A guard which restores the previous signal
/// handlers when dropped
pub struct Guard(Vec<PrevHandler>);

impl Drop for Guard {
    fn drop(&mut self) {
        for PrevHandler { signal, action } in &self.0 {
            unsafe {
                let _ = sigaction(*signal, action);
            };
        }
    }
}

struct PrevHandler {
    signal: Signal,
    action: SigAction,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("ignore signal")]
    Ignore(#[source] nix::Error),
    #[error("failed to connect to dbus")]
    Zbus(#[from] zbus::Error),
}
