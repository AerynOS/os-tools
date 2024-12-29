// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::process::Command;

use fs_err as fs;
use nix::unistd::{Pid, User, getgid, getuid};
use snafu::{ResultExt, Snafu, ensure};

pub fn idmap(pid: Pid) -> Result<(), Error> {
    let uid = getuid();
    let gid = getgid();
    let username = User::from_uid(uid)
        .context(GetUserByUidSnafu)?
        .map(|user| user.name)
        .unwrap_or_default();

    let subuid_mappings = load_sub_mappings(Kind::User, uid.as_raw(), &username)?;
    let subgid_mappings = load_sub_mappings(Kind::Group, gid.as_raw(), &username)?;

    let uid_mappings = format_id_mappings(&subuid_mappings);
    let gid_mappings = format_id_mappings(&subgid_mappings);

    add_id_mappings(pid, Kind::User, uid.as_raw(), &uid_mappings)?;
    add_id_mappings(pid, Kind::Group, gid.as_raw(), &gid_mappings)?;

    Ok(())
}

#[derive(Debug, Clone, Copy, strum::Display)]
pub enum Kind {
    #[strum(serialize = "uid")]
    User,
    #[strum(serialize = "gid")]
    Group,
}

fn load_sub_mappings(kind: Kind, id: u32, username: &str) -> Result<Vec<Submap>, Error> {
    let Ok(content) = fs::read_to_string(format!("/etc/sub{kind}")) else {
        ensure_sub_count(kind, id, &[])?;
        return Ok(vec![]);
    };

    let mut mappings = vec![];

    let lines = content.lines();

    for line in lines {
        let mut split = line.split(':');

        let user = split.next();
        let sub_id = split.next().and_then(|s| s.parse::<u32>().ok());
        let count = split.next().and_then(|s| s.parse::<u32>().ok());

        if let (Some(user), Some(sub_id), Some(count)) = (user, sub_id, count) {
            if user.parse::<u32>() == Ok(id) || user == username {
                mappings.push(Submap { sub_id, count });
            }
        }
    }

    ensure_sub_count(kind, id, &mappings)?;

    Ok(mappings)
}

fn ensure_sub_count(kind: Kind, id: u32, mappings: &[Submap]) -> Result<(), Error> {
    let count = mappings.iter().map(|map| map.count).sum::<u32>();
    ensure!(count >= 1000, SubMappingCountSnafu { id, kind, count });
    Ok(())
}

fn format_id_mappings(sub_mappings: &[Submap]) -> Vec<Idmap> {
    // Start mapping at 1 (root mapped to user)
    let mut ns_id = 1;

    let mut id_mappings = vec![];

    for submap in sub_mappings {
        id_mappings.push(Idmap {
            ns_id,
            host_id: submap.sub_id,
            count: submap.count,
        });

        ns_id += submap.count;
    }

    id_mappings
}

fn add_id_mappings(pid: Pid, kind: Kind, id: u32, mappings: &[Idmap]) -> Result<(), Error> {
    let cmd = match kind {
        Kind::User => "newuidmap",
        Kind::Group => "newgidmap",
    };
    let out = Command::new(cmd)
        .arg(pid.as_raw().to_string())
        // Root mapping
        .arg(0.to_string())
        .arg(id.to_string())
        .arg(1.to_string())
        // Sub mappings
        .args(mappings.iter().flat_map(|mapping| {
            [
                mapping.ns_id.to_string(),
                mapping.host_id.to_string(),
                mapping.count.to_string(),
            ]
        }))
        .output()
        .boxed()
        .context(CommandSnafu { kind })?;

    if !out.status.success() {
        return Err(Error::Command {
            kind,
            source: out.status.to_string().into(),
        });
    }

    Ok(())
}

#[derive(Debug)]
struct Submap {
    sub_id: u32,
    count: u32,
}

#[derive(Debug)]
struct Idmap {
    ns_id: u32,
    host_id: u32,
    count: u32,
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display(
        "\n\nAt least 1,000 sub{kind} mappings are required for {kind} {id}, found {count}\n\nMappings can be added to /etc/sub{kind}"
    ))]
    SubMappingCount { id: u32, kind: Kind, count: u32 },
    #[snafu(display("new{kind}map command failed"))]
    Command {
        kind: Kind,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[snafu(display("get user by UID"))]
    GetUserByUid { source: nix::Error },
}
