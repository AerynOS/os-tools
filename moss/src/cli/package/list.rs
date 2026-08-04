// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use moss::{Client, Installation, environment, package::Flags};
use tui::Styled;

use crate::cli::package::{self, ListArgs};

pub fn list(args: ListArgs, installation: Installation) -> Result<(), package::Error> {
    if !args.repositories.is_empty() {
        unimplemented!("--repositories not yet supported")
    }

    let mut flags = Flags::new();
    let mut sync = None;
    if args.available {
        flags = flags.with_available();
    }
    if args.explicit {
        flags = flags.with_installed().with_explicit();
    }
    if args.sync {
        flags = flags.with_installed();
        sync = Some(Sync::All);
    }

    // Grab a client for the target, enumerate packages
    let client = Client::new(environment::NAME, installation)?;
    let pkgs = client.list_packages(flags).collect::<Vec<_>>();

    let sync_available = if sync.is_some() {
        client.list_packages(Flags::new().with_available()).collect::<Vec<_>>()
    } else {
        vec![]
    };

    if pkgs.is_empty() {
        return Err(package::Error::NoneFound);
    }

    // map to renderable state
    let mut set = pkgs
        .into_iter()
        .map(|p| {
            let sync = sync_available
                .iter()
                // Get first (priority based)
                .find(|u| u.meta.name == p.meta.name)
                // Ensure it's an upgrade (if `upgrades-only`)
                // otherwise check if it's a change
                .filter(|u| {
                    if matches!(sync, Some(Sync::Upgrades)) {
                        u.meta.source_release > p.meta.source_release
                    } else {
                        u.meta.source_release != p.meta.source_release
                    }
                })
                .map(|u| Revision {
                    version: u.meta.version_identifier.clone(),
                    release: u.meta.source_release.to_string(),
                });

            Format {
                name: p.meta.name.to_string(),
                revision: Revision {
                    version: p.meta.version_identifier,
                    release: p.meta.source_release.to_string(),
                },
                summary: p.meta.summary,
                explicit: if flags == Flags::new().with_installed() {
                    p.flags.explicit
                } else {
                    true
                },
                sync,
            }
        })
        .filter(|item| if sync.is_some() { item.sync.is_some() } else { true })
        .collect::<Vec<_>>();

    // Thanks to priorities, first in list is the winning candidate in list available.
    // Therefore sort by name and dedupe is safe as we mask the lower priority items out.
    set.sort_by_key(|s| s.name.clone());
    set.dedup_by_key(|s| s.name.clone());

    // Grab maximum length
    let max_length = set.iter().map(Format::size).max().unwrap_or_default() + 2;

    // render
    for item in set {
        let width = max_length - item.size() + 2;
        let name = if item.explicit {
            item.name.bold()
        } else {
            item.name.dim()
        };
        print!("{name} {:width$} ", " ");

        let print_revision = |rev: Revision, is_sync| {
            let version = if is_sync {
                rev.version.green()
            } else {
                rev.version.magenta()
            };
            print!("{version}-{}", rev.release.dim());
        };

        // Print revision
        print_revision(item.revision, false);

        // Print sync version
        if let Some(sync) = item.sync {
            print!(" => ");
            print_revision(sync, true);
        }

        println!(" - {}", item.summary);
    }

    Ok(())
}

enum Sync {
    All,
    Upgrades,
}

#[derive(Debug)]
struct Format {
    name: String,
    summary: String,
    revision: Revision,
    explicit: bool,
    sync: Option<Revision>,
}

impl Format {
    fn size(&self) -> usize {
        self.name.len() + self.revision.size() + self.sync.as_ref().map(Revision::size).unwrap_or_default()
    }
}

#[derive(Debug)]
struct Revision {
    version: String,
    release: String,
}

impl Revision {
    fn size(&self) -> usize {
        self.version.len() + self.release.len()
    }
}
