// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

pub use self::styled::Styled;
pub use dialoguer;
pub use indicatif::*;

pub mod pretty;
mod styled;

/// The size of a terminal emulator window.
pub struct TermSize {
    /// Width (or number of columns) of the terminal, in characters.
    pub width: usize,
    /// Height (or number of rows) of the terminal, in characters.
    pub height: usize,
}

impl Default for TermSize {
    fn default() -> Self {
        Self { width: 80, height: 24 }
    }
}

impl TermSize {
    /// Returns a valid terminal size. If the system couldn't be queried,
    /// it returns the default value.
    pub fn get() -> Self {
        let size = crossterm::terminal::size().unwrap_or_default();
        if size.0 < 1 || size.1 < 1 {
            return TermSize::default();
        }
        TermSize {
            width: size.0 as usize,
            height: size.1 as usize,
        }
    }
}
