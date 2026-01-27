use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, TryStreamExt, stream};
use thiserror::Error;
use tracing::{info, info_span, instrument};
use tui::{
    MultiProgress, ProgressBar, ProgressStyle, Styled,
    pretty::{ColumnDisplay, autoprint_columns},
};
use url::{ParseError, Url};

use crate::{
    Client, Package, Provider,
    client::{
        self,
        cache::{self},
    },
    environment,
    package::{self, Flags, Meta},
    runtime, util,
};

/// Fetch a set of packages.
#[instrument(skip(client), fields(ephemeral = client.is_ephemeral()))]
pub fn fetch(client: &mut Client, pkgs: &[&str], output_dir: &PathBuf) -> Result<Timing, Error> {
    let mut timing = Timing::default();
    let mut instant = Instant::now();

    let input = resolve_input(pkgs, client)?;

    timing.resolve = instant.elapsed();
    info!(
        total_packages = input.len(),
        packages_to_fetch = input.len(),
        resolve_time_ms = timing.resolve.as_millis(),
        "Package resolution for fetch completed"
    );

    println!("The following package(s) will be fetched:");
    println!();
    autoprint_columns(&input);

    instant = Instant::now();

    let cache_packages_span = info_span!("progress", phase = "cache_packages", event_type = "progress");
    let _cache_packages_guard = cache_packages_span.enter();
    info!(total_items = input.len(), progress = 0.0, event_type = "progress_start");

    let multi_progress = MultiProgress::new();

    // Add bar to track total package counts
    let total_progress = multi_progress.add(
        ProgressBar::new(input.len() as u64).with_style(
            ProgressStyle::with_template("\n|{bar:20.cyan/blue}| {pos}/{len}")
                .unwrap()
                .progress_chars("■≡=- "),
        ),
    );
    total_progress.tick();

    runtime::block_on(async {
        let stream = stream::iter(input.clone()).map(|meta| async {
            let progress_bar = multi_progress.insert_before(
                &total_progress,
                ProgressBar::new(meta.download_size.unwrap_or_default())
                    .with_message(format!("{} {}", "Downloading".blue(), meta.name.as_str().bold(),))
                    .with_style(
                        ProgressStyle::with_template(
                            " {spinner} |{percent:>3}%| {wide_msg} {binary_bytes_per_sec:>.dim} ",
                        )
                        .unwrap()
                        .tick_chars("--=≡■≡=--"),
                    ),
            );
            progress_bar.enable_steady_tick(Duration::from_millis(150));

            let download = cache::fetch(&meta, &client.installation, |progress| {
                progress_bar.inc(progress.delta);
                info!(
                    progress = progress.completed as f32 / progress.total as f32,
                    current = progress.completed as usize,
                    total = progress.total as usize,
                    event_type = "progress_update",
                    "Downloading {}",
                    meta.name
                );
            })
            .await
            .map_err(|err| Error::CacheFetch(err, meta.name.clone()))?;

            let parsed = Url::parse(&meta.uri.unwrap())?;
            let file_name = parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap();

            let dest_path = output_dir.join(file_name);

            let multi_progress = multi_progress.clone();
            let total_progress = total_progress.clone();

            util::hardlink_or_copy(download.path(), &dest_path)?;

            runtime::unblock(move || {
                progress_bar.finish();
                multi_progress.remove(&progress_bar);

                let is_cached = download.was_cached;

                let cached_tag = is_cached
                    .then_some(format!("{}", " (cached)".dim()))
                    .unwrap_or_default();

                multi_progress.suspend(|| {
                    // Print the relative instead of absolute path to user
                    let path_to_print = if let Ok(cwd) = std::env::current_dir() {
                        dest_path.strip_prefix(cwd).ok().unwrap_or(&dest_path)
                    } else {
                        &dest_path
                    };

                    println!(
                        "{} {}{cached_tag} {}",
                        "Fetched".green(),
                        meta.name.to_string().bold(),
                        path_to_print.display()
                    );
                });

                total_progress.inc(1);
            })
            .await;

            Ok(()) as Result<(), Error>
        });

        let buffered = stream.buffer_unordered(environment::MAX_NETWORK_CONCURRENCY);

        let res: Result<(), Error> = buffered.try_collect().await;
        res
    })?;

    timing.fetch = instant.elapsed();
    info!(
        duration_ms = timing.fetch.as_millis(),
        items_processed = input.len(),
        progress = 1.0,
        event_type = "progress_completed",
    );
    drop(_cache_packages_guard);

    Ok(timing)
}

/// Resolves the package arguments as valid input packages. Returns an error
/// if any args are invalid.
#[instrument(skip(client))]
fn resolve_input(pkgs: &[&str], client: &Client) -> Result<Vec<Meta>, Error> {
    // Parse pkg args into valid / invalid sets
    let queried = pkgs.iter().map(|p| find_packages(p, client));

    let mut results = vec![];

    for (id, pkg) in queried {
        if let Some(pkg) = pkg {
            // We'll need to resolve explicitly by id to populate the full meta.uri
            let resolved_pkg_meta = client.registry.by_id(&pkg.id).next().ok_or(Error::NoPackage(id))?;

            results.push(resolved_pkg_meta.meta);
        } else {
            return Err(Error::NoPackage(id));
        }
    }

    Ok(results)
}

/// Resolve a package name to the first package
fn find_packages(id: &str, client: &Client) -> (String, Option<Package>) {
    let provider = Provider::from_name(id).unwrap();
    let result = client
        .registry
        .by_provider(&provider, Flags::new().with_available())
        .next();

    // First only, pre-sorted
    (id.into(), result)
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),

    #[error("cancelled")]
    Cancelled,

    #[error("client")]
    Client(#[from] client::Error),

    /// The given package couldn't be found
    #[error("no package found: {0}")]
    NoPackage(String),

    #[error("string processing")]
    Dialog(#[from] tui::dialoguer::Error),

    #[error("failed to parse package uri")]
    ParseError(#[from] ParseError),

    #[error("could not determine filename from uri: {0}")]
    NoFileNameInUri(String),

    #[error("fetch package {1}")]
    CacheFetch(#[source] cache::FetchError, package::Name),
}

/// Simple timing information for Fetch
#[derive(Default)]
pub struct Timing {
    pub resolve: Duration,
    pub fetch: Duration,
}

impl ColumnDisplay for Meta {
    fn get_display_width(&self) -> usize {
        self.name.as_str().chars().count()
    }

    fn display_column(&self, writer: &mut impl io::prelude::Write, _col: tui::pretty::Column, width: usize) {
        let _ = writeln!(
            writer,
            "{}{:width$}  {}",
            self.name.as_str().bold(),
            " ".repeat(width),
            self.uri.clone().unwrap().as_str()
        );
    }
}
