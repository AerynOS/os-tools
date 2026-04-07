// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

use tui::Styled;

use crate::{architecture::BuildTarget, build};

const PROGRESS_WIDTH: usize = 6;
const ELAPSED_WIDTH: usize = 13;

#[derive(Default)]
pub struct Timing {
    initialize: Duration,
    populate: BTreeMap<Populate, Duration>,
    fetch: Duration,
    build: BTreeMap<BuildTarget, BTreeMap<Option<build::pgo::Stage>, BTreeMap<build::job::Phase, BuildEntry>>>,
    analyze: Duration,
    emit: Duration,
}

impl Timing {
    pub fn begin(&mut self, kind: Kind) -> Timer {
        Timer(kind, Instant::now())
    }

    pub fn finish(&mut self, timer: Timer) {
        self.record(timer.0, timer.1.elapsed());
    }

    pub fn record(&mut self, kind: impl Into<Kind>, elapsed: Duration) {
        match kind.into() {
            Kind::Initialize => self.initialize = elapsed,
            Kind::Populate(populate) => {
                self.populate.insert(populate, elapsed);
            }
            Kind::Fetch => self.fetch = elapsed,
            Kind::Build(
                build @ Build {
                    target,
                    pgo_stage,
                    phase,
                },
            ) => {
                self.build
                    .entry(target)
                    .or_default()
                    .entry(pgo_stage)
                    .or_default()
                    .insert(phase, BuildEntry { build, elapsed });
            }
            Kind::Analyze => self.analyze = elapsed,
            Kind::Emit => self.emit = elapsed,
        }
    }

    pub fn print_table(&self) {
        let max_prefix_length = self
            .build
            .values()
            .flat_map(|stages| {
                stages
                    .values()
                    .flat_map(|phases| phases.values().map(BuildEntry::max_prefix_length))
            })
            .max()
            .unwrap_or_default()
            // No-op (less than "Populate (moss)")
            // .chain(self.populate.keys().map(|k| k.to_string().len()))
            // .max("Initialize".len())
            // .max("Fetch".len())
            // .max("Analyze".len())
            // .max("Emit".len());
            .max("Populate (moss)".len());
        let total_elapsed = self
            .build
            .values()
            .flat_map(|stages| stages.values().flat_map(|phases| phases.values().map(|e| e.elapsed)))
            .chain(self.populate.values().copied())
            .sum::<Duration>()
            + self.initialize
            + self.fetch
            + self.analyze
            + self.emit;

        println!(
            "P{:<max_prefix_length$}  {:>ELAPSED_WIDTH$} {:>PROGRESS_WIDTH$}",
            "hase", "Elapsed", "%",
        );
        println!(
            "│{:<max_prefix_length$}  {} {}",
            "Initialize",
            fmt_elapsed(self.initialize),
            fmt_progress(self.initialize, total_elapsed)
        );

        println!("│{}", "Populate (moss)".dim());
        for (key, elapsed) in &self.populate {
            let gap = max_prefix_length - (key.to_string().len() + 1);

            println!(
                "│{}{}{}  {} {}",
                "│".dim(),
                key.styled(),
                " ".repeat(gap),
                fmt_elapsed(*elapsed),
                fmt_progress(*elapsed, total_elapsed)
            );
        }

        println!(
            "│{:<max_prefix_length$}  {} {}",
            "Fetch",
            fmt_elapsed(self.fetch),
            fmt_progress(self.fetch, total_elapsed)
        );

        for (target, stages) in &self.build {
            println!("│{}", build::build_target_prefix(*target, 0));

            for (stage, phases) in stages {
                if let Some(stage) = stage {
                    println!("│{}", build::pgo_stage_prefix(*stage, 0));
                }

                for (phase, entry) in phases {
                    let gap = max_prefix_length - (phase.to_string().len() + if stage.is_some() { 2 } else { 1 });

                    println!(
                        "│{}{}  {} {}",
                        build::phase_prefix(*phase, stage.is_some(), 0),
                        " ".repeat(gap),
                        fmt_elapsed(entry.elapsed),
                        fmt_progress(entry.elapsed, total_elapsed),
                    );
                }
            }
        }

        println!(
            "│{:<max_prefix_length$}  {} {}",
            "Analyze",
            fmt_elapsed(self.analyze),
            fmt_progress(self.analyze, total_elapsed)
        );
        println!(
            "│{:<max_prefix_length$}  {} {}",
            "Emit",
            fmt_elapsed(self.emit),
            fmt_progress(self.emit, total_elapsed),
        );
        println!(
            "{}",
            "─".repeat(1 + max_prefix_length + 2 + ELAPSED_WIDTH + 1 + PROGRESS_WIDTH),
        );
        println!(
            "T{:<max_prefix_length$}  {} {}",
            "otal",
            fmt_elapsed(total_elapsed),
            fmt_progress(total_elapsed, total_elapsed)
        );
        println!();
    }
}

pub struct Timer(Kind, Instant);

pub enum Kind {
    /// Initialize boulder
    Initialize,
    /// Populate root from moss
    Populate(Populate),
    /// Fetch upstreams
    Fetch,
    /// Build phase
    Build(Build),
    /// Analyze install paths
    Analyze,
    /// Emit artefacts
    Emit,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
pub enum Populate {
    /// Resolve DAG
    Resolve,
    /// Fetch packages
    Fetch,
    /// Blit packages
    Blit,
}

impl Populate {
    fn styled(&self) -> impl fmt::Display + use<> {
        match self {
            Populate::Resolve => self.to_string().cyan(),
            Populate::Fetch => self.to_string().blue(),
            Populate::Blit => self.to_string().green(),
        }
    }
}

impl From<Populate> for Kind {
    fn from(value: Populate) -> Self {
        Kind::Populate(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Build {
    /// Build target (arch)
    pub target: BuildTarget,
    /// PGO stage, if applicable
    pub pgo_stage: Option<build::pgo::Stage>,
    /// Build phase (prepare, setup, build, etc)
    pub phase: build::job::Phase,
}

struct BuildEntry {
    build: Build,
    elapsed: Duration,
}

impl BuildEntry {
    pub fn max_prefix_length(&self) -> usize {
        self.build
            .target
            .to_string()
            .len()
            .max(
                self.build
                    .pgo_stage
                    .map(|stage| stage.to_string().len() + 1)
                    .unwrap_or_default(),
            )
            .max(self.build.phase.to_string().len() + if self.build.pgo_stage.is_some() { 2 } else { 1 })
    }
}

/// Format a template of `000h00m00.00s`, removing
/// leading zeros for spaces if the duration is
/// too small
fn fmt_elapsed(duration: Duration) -> String {
    let _seconds = duration.as_secs_f32() % 60.0;
    let _minutes = (duration.as_secs() / 60) % 60;
    let _hours = duration.as_secs() / 3600;

    // Only pad zeros if next unit exists
    let seconds = if _minutes > 0 {
        format!("{_seconds:0>5.2}s")
    } else {
        format!("{_seconds:>5.2}s")
    };

    let minutes = if _minutes > 0 {
        // Only pad zeros if next unit exists
        if _hours > 0 {
            format!("{_minutes:0>2}m")
        } else {
            format!("{_minutes:>2}m")
        }
    } else {
        " ".repeat(3)
    };

    let hours = if _hours > 0 {
        format!("{_hours:>3}h")
    } else {
        " ".repeat(4)
    };

    format!("{hours}{minutes}{seconds}")
}

fn fmt_progress(elapsed: Duration, total: Duration) -> String {
    let pct = elapsed.as_secs_f32() / total.as_secs_f32() * 100.0;

    format!("{pct:>5.1}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seconds_only() {
        let elapsed = Duration::from_secs_f32(1.004);
        assert_eq!(fmt_elapsed(elapsed), "        1.00s");
    }

    #[test]
    fn test_minutes_and_seconds() {
        let elapsed = Duration::from_secs(60);
        assert_eq!(fmt_elapsed(elapsed), "     1m00.00s");
    }

    #[test]
    fn test_minutes_and_seconds2() {
        let elapsed = Duration::from_secs(61);
        assert_eq!(fmt_elapsed(elapsed), "     1m01.00s");
    }

    #[test]
    fn test_hours_minutes_and_seconds() {
        let elapsed = Duration::from_secs(3600);
        assert_eq!(fmt_elapsed(elapsed), "  1h    0.00s");
    }

    #[test]
    fn test_hours_minutes_and_seconds2() {
        let elapsed = Duration::from_secs(3601);
        assert_eq!(fmt_elapsed(elapsed), "  1h    1.00s");
    }

    #[test]
    fn test_hours_minutes_and_seconds3() {
        let elapsed = Duration::from_secs(3661);
        assert_eq!(fmt_elapsed(elapsed), "  1h01m01.00s");
    }

    #[test]
    fn test_two_hours_or_more() {
        let elapsed = Duration::from_secs(7200);
        assert_eq!(fmt_elapsed(elapsed), "  2h    0.00s");
    }

    #[test]
    fn test_two_hours_or_more2() {
        let elapsed = Duration::from_secs(7201);
        assert_eq!(fmt_elapsed(elapsed), "  2h    1.00s");
    }

    #[test]
    fn test_two_hours_or_more3() {
        let elapsed = Duration::from_secs(7261);
        assert_eq!(fmt_elapsed(elapsed), "  2h01m01.00s");
    }

    #[test]
    fn test_two_hours_or_more100() {
        let elapsed = Duration::from_secs(360_061);
        assert_eq!(fmt_elapsed(elapsed), "100h01m01.00s");
    }
}
