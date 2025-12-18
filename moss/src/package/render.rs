// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::Write;

use tui::{
    Styled,
    pretty::{Column, ColumnDisplay},
};

use crate::{Package, package};

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

        if col != Column::Last {
            _ = write!(writer, "   ");
        }
    }
}

impl<'a> ColumnDisplay for package::Update<'a> {
    fn get_display_width(&self) -> usize {
        self.new.meta.name.to_string().len()
            + self.old.meta.version_identifier.len()
            + self.old.meta.source_release.to_string().len()
            + self.new.meta.version_identifier.len()
            + self.new.meta.source_release.to_string().len()
            + COLUMN_PADDING
            + 6
    }

    fn display_column(&self, writer: &mut impl Write, col: Column, width: usize) {
        let fmt_version = |meta: &package::Meta| format!("{}-{}", meta.version_identifier, meta.source_release);

        let old_version = fmt_version(&self.old.meta);
        let new_version = fmt_version(&self.new.meta);

        let old_version_diff = color_diff(&new_version, &old_version, true);
        let new_version_diff = color_diff(&old_version, &new_version, false);

        _ = write!(
            writer,
            "{} {:width$}{old_version_diff} -> {new_version_diff}",
            self.new.meta.name.to_string().bold(),
            " ",
        );

        if col != Column::Last {
            _ = write!(writer, "   ");
        }
    }
}

fn color_diff(a: &str, b: &str, red: bool) -> String {
    let mut b_segments = to_segments(b).into_iter();

    let mut s = String::with_capacity(b.len() * 2);

    'outer: for a_section in to_segments(a).into_iter().filter_map(Segment::to_section) {
        loop {
            match b_segments.next() {
                Some(Segment::Delim(c)) => s.push(c),
                Some(Segment::Section(b_section)) => {
                    if a_section != b_section {
                        if red {
                            s.push_str(&b_section.red().to_string());
                        } else {
                            s.push_str(&b_section.green().bold().to_string());
                        }
                    } else {
                        s.push_str(&b_section.dim().to_string());
                    }

                    continue 'outer;
                }
                None => break 'outer,
            }
        }
    }

    for segment in b_segments {
        match segment {
            Segment::Delim(c) => s.push(c),
            Segment::Section(section) => {
                if red {
                    s.push_str(&section.red().to_string());
                } else {
                    s.push_str(&section.green().bold().to_string());
                }
            }
        }
    }

    s
}

fn to_segments(s: &str) -> Vec<Segment> {
    s.chars().fold(vec![], |mut acc, c| {
        if c.is_alphanumeric() {
            if let Some(Segment::Section(section)) = acc.last_mut() {
                section.push(c);
            } else {
                acc.push(Segment::Section(String::from_iter([c])));
            }
        } else {
            acc.push(Segment::Delim(c));
        }

        acc
    })
}

enum Segment {
    Delim(char),
    Section(String),
}

impl Segment {
    fn to_section(self) -> Option<String> {
        if let Self::Section(section) = self {
            Some(section)
        } else {
            None
        }
    }
}
