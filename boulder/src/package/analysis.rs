// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
};

use moss::{Dependency, Provider};
use stone::write::digest;
use tui::{ProgressBar, ProgressStyle, Styled};

use crate::{Paths, Recipe};

use super::collect::{Collector, PathInfo};

mod handler;

pub type BoxError = Box<dyn std::error::Error>;

pub struct Chain<'a> {
    handlers: Vec<Box<dyn Handler>>,
    recipe: &'a Recipe,
    paths: &'a Paths,
    collector: &'a Collector,
    hasher: &'a mut digest::Hasher,
    pub buckets: BTreeMap<String, Bucket>,
}

impl<'a> Chain<'a> {
    pub fn new(paths: &'a Paths, recipe: &'a Recipe, collector: &'a Collector, hasher: &'a mut digest::Hasher) -> Self {
        Self {
            handlers: vec![
                Box::new(handler::ignore_blocked),
                Box::new(handler::binary),
                Box::new(handler::elf),
                Box::new(handler::pkg_config),
                Box::new(handler::python),
                Box::new(handler::cmake),
                // Catch-all if not excluded
                Box::new(handler::include_any),
            ],
            paths,
            recipe,
            collector,
            hasher,
            buckets: Default::default(),
        }
    }

    pub fn process(&mut self, paths: impl IntoIterator<Item = PathInfo>) -> Result<(), BoxError> {
        println!("│Analyzing artefacts (» = Include, × = Ignore, ^ = Replace)");

        let mut queue = paths.into_iter().collect::<VecDeque<_>>();

        let pb = ProgressBar::new(queue.len() as u64)
            .with_message("Analyzing")
            .with_style(
                ProgressStyle::with_template("\n|{bar:20.red/blue}| {pos}/{len} {wide_msg}")
                    .unwrap()
                    .progress_chars("■≡=- "),
            );
        pb.tick();

        'paths: while let Some(mut path) = queue.pop_front() {
            let bucket = self.buckets.entry(path.package.clone()).or_default();

            pb.set_message(format!("Analyzing {}", path.target_path.display()));

            'handlers: for handler in &self.handlers {
                // Only give handlers ability to update
                // certain bucket fields
                let mut bucket_mut = BucketMut {
                    providers: &mut bucket.providers,
                    dependencies: &mut bucket.dependencies,
                    hasher: self.hasher,
                    recipe: self.recipe,
                    paths: self.paths,
                };

                let response = handler.handle(&mut bucket_mut, &mut path)?;

                response.generated_paths.into_iter().try_for_each(|path| {
                    let info = self.collector.path(&path, self.hasher)?;

                    queue.push_back(info);

                    Ok(()) as Result<(), BoxError>
                })?;

                match response.decision {
                    Decision::NextHandler => continue 'handlers,
                    Decision::IgnoreFile { reason } => {
                        pb.suspend(|| {
                            println!(
                                "│A{} {} {}",
                                "│ ×".yellow(),
                                format!("{}", path.target_path.display()).dim(),
                                format!("({reason})").yellow()
                            );
                        });
                        pb.inc(1);
                        continue 'paths;
                    }
                    Decision::IncludeFile => {
                        pb.suspend(|| println!("│A{} {}", "│ »".green(), path.target_path.display()));
                        pb.inc(1);
                        bucket.paths.push(path);
                        continue 'paths;
                    }
                    Decision::ReplaceFile { newpath } => {
                        let newpathinfo = self.collector.path(&newpath, self.hasher)?;
                        pb.println(format!(
                            "│A{} {} » {}",
                            "│ ^".dark_magenta(),
                            format!("{}", path.target_path.display()).dim(),
                            newpathinfo.target_path.display()
                        ));
                        pb.inc(1);
                        bucket.paths.push(newpathinfo);
                        continue 'paths;
                    }
                }
            }
        }

        pb.finish_and_clear();
        println!();

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Bucket {
    providers: BTreeSet<Provider>,
    dependencies: BTreeSet<Dependency>,
    pub paths: Vec<PathInfo>,
}

impl Bucket {
    pub fn providers(&self) -> impl Iterator<Item = &Provider> {
        self.providers.iter()
    }

    pub fn dependencies(&self) -> impl Iterator<Item = &Dependency> {
        // We shouldn't self depend on things we provide
        self.dependencies
            .iter()
            .filter(|d| !self.providers.iter().any(|p| p.kind == d.kind && p.name == d.name))
    }
}

pub struct BucketMut<'a> {
    pub providers: &'a mut BTreeSet<Provider>,
    pub dependencies: &'a mut BTreeSet<Dependency>,
    pub hasher: &'a mut digest::Hasher,
    pub recipe: &'a Recipe,
    pub paths: &'a Paths,
}

pub struct Response {
    pub decision: Decision,
    pub generated_paths: Vec<PathBuf>,
}

pub enum Decision {
    NextHandler,
    IgnoreFile { reason: String },
    IncludeFile,
    ReplaceFile { newpath: PathBuf },
}

impl From<Decision> for Response {
    fn from(decision: Decision) -> Self {
        Self {
            decision,
            generated_paths: vec![],
        }
    }
}

pub trait Handler {
    fn handle(&self, bucket: &mut BucketMut<'_>, path: &mut PathInfo) -> Result<Response, BoxError>;
}

impl<T> Handler for T
where
    T: Fn(&mut BucketMut<'_>, &mut PathInfo) -> Result<Response, BoxError>,
{
    fn handle(&self, bucket: &mut BucketMut<'_>, path: &mut PathInfo) -> Result<Response, BoxError> {
        (self)(bucket, path)
    }
}
