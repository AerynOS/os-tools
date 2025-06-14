// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Pretty printing for moss CLI

use std::{
    cmp::{max, min},
    io::{Write, stdout},
};

use crate::TermSize;

/// Simplistic handling of renderable display columns
/// allowing implementations to handle first, n and last specific alignment
#[derive(PartialEq)]
pub enum Column {
    First,
    Nth(usize),
    Last,
}

/// Implementing ColumnDisplay for a type allows use of the print_to_columns
/// function, ie pretty print in individual alphabetically sorted columns
pub trait ColumnDisplay: Sized {
    /// Implementations return their full display size
    fn get_display_width(&self) -> usize;

    /// Render to the given Writer
    fn display_column(&self, writer: &mut impl Write, col: Column, width: usize);
}

pub fn print_columns<T: ColumnDisplay>(items: &[T], colnum: usize) {
    column_printer(items, Some(colnum));
}

/// Prints a vec of items that implement the ColumnDisplay trait.
///
/// These will be printed in individual columns assuming that the input order is
/// alphabetically sorted, to give each column an ascending alpha sort.
pub fn autoprint_columns<T: ColumnDisplay>(items: &[T]) {
    column_printer(items, None);
}

fn column_printer<T: ColumnDisplay>(items: &[T], colnum: Option<usize>) {
    let max_width = TermSize::get().width;

    let Some(largest_element) = items.iter().max_by_key(|p| p.get_display_width()) else {
        return;
    };
    let largest_width = min(max_width, largest_element.get_display_width() + 1);

    let colnum = colnum.unwrap_or_else(|| max(1, max_width / largest_width));
    let rownum = ((items.len() as f32) / (colnum as f32)).ceil() as usize;

    let mut stdout = stdout().lock();
    for y in 0..rownum {
        for x in 0..colnum {
            let idx = y + (x * rownum);
            let state = items.get(idx);
            if let Some(state) = state {
                let column = if x == 0 {
                    Column::First
                } else if x == colnum - 1 {
                    Column::Last
                } else {
                    Column::Nth(x)
                };
                state.display_column(
                    &mut stdout,
                    column,
                    largest_width.saturating_sub(state.get_display_width()),
                );
            }
        }
        let _ = writeln!(stdout);
    }
}
