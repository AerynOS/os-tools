// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::Write;

use tui::{
    Styled,
    pretty::{Column, ColumnDisplay},
};

use crate::Package;

/// We always pad columns by 3 spaces to just not jank up the output
const COLUMN_PADDING: usize = 3;

/// Allow display packages in column form
impl ColumnDisplay for Package {
    fn get_display_width(&self) -> usize {
        ColumnDisplay::get_display_width(&self)
    }

    fn display_column(&self, writer: &mut impl Write, col: Column, width: usize) {
        ColumnDisplay::display_column(&self, writer, col, width);
    }
}

impl ColumnDisplay for &Package {
    fn get_display_width(&self) -> usize {
        self.meta.name.to_string().len()
            + self.meta.version_identifier.len()
            + self.meta.source_release.to_string().len()
            + COLUMN_PADDING
    }

    fn display_column(&self, writer: &mut impl Write, col: Column, width: usize) {
        _ = write!(
            writer,
            "{} {:width$}{}-{}",
            self.meta.name.to_string().bold(),
            " ",
            self.meta.version_identifier.clone().magenta(),
            self.meta.source_release.to_string().dim(),
        );

        if self.meta.build_release > 1 {
            _ = write!(writer, "-{}", self.meta.build_release.to_string().dim());
        }

        if col != Column::Last {
            _ = write!(writer, "   ");
        }
    }
}
