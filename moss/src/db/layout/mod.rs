// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use astr::AStr;
use diesel::prelude::*;
use diesel::{Connection as _, SqliteConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use std::{borrow::Cow, collections::BTreeSet};

use stone::payload;

use crate::package;

pub use super::Error;
use super::{Connection, MAX_VARIABLE_NUMBER};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("src/db/layout/migrations");

mod schema;

#[derive(Debug, Clone)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(url: &str) -> Result<Self, Error> {
        let mut conn = SqliteConnection::establish(url)?;

        conn.run_pending_migrations(MIGRATIONS).map_err(Error::Migration)?;

        Ok(Database {
            conn: Connection::new(conn),
        })
    }

    /// Retrieve all entries for a given package by ID
    pub fn query<'a>(
        &self,
        packages: impl IntoIterator<Item = &'a package::Id>,
    ) -> Result<Vec<(package::Id, payload::Layout)>, Error> {
        self.conn.exec(|conn| {
            let packages = packages.into_iter().map(package::Id::as_str).collect::<Vec<_>>();

            let mut output = vec![];

            for chunk in packages.chunks(MAX_VARIABLE_NUMBER) {
                output.extend(
                    model::layout::table
                        .select(model::Layout::as_select())
                        .filter(model::layout::package_id.eq_any(chunk))
                        .load_iter(conn)?
                        .map(map_layout)
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }

            Ok(output)
        })
    }

    pub fn all(&self) -> Result<Vec<(package::Id, payload::Layout)>, Error> {
        self.conn.exec(|conn| {
            model::layout::table
                .select(model::Layout::as_select())
                .load_iter(conn)?
                .map(map_layout)
                .collect()
        })
    }

    pub fn package_ids(&self) -> Result<BTreeSet<package::Id>, Error> {
        self.conn.exec(|conn| {
            Ok(model::layout::table
                .select(model::layout::package_id)
                .distinct()
                .load_iter::<AStr, _>(conn)?
                .map(|result| result.map(package::Id::from))
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn file_hashes(&self) -> Result<BTreeSet<String>, Error> {
        self.conn.exec(|conn| {
            let hashes = model::layout::table
                .select(model::layout::entry_value1.assume_not_null())
                .distinct()
                .filter(model::layout::entry_type.eq("regular"))
                .load::<String>(conn)?;

            Ok(hashes
                .into_iter()
                .filter_map(|hash| hash.parse::<u128>().ok().map(|hash| format!("{hash:02x}")))
                .collect())
        })
    }

    pub fn add(&self, package: &package::Id, layout: &payload::Layout) -> Result<(), Error> {
        self.batch_add(vec![(package, layout)])
    }

    pub fn batch_add<'a>(
        &self,
        layouts: impl IntoIterator<Item = (&'a package::Id, &'a payload::Layout)>,
    ) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let mut ids = vec![];

            let values = layouts
                .into_iter()
                .map(|(package_id, layout)| {
                    ids.push(package_id.as_str());

                    let (entry_type, entry_value1, entry_value2) = encode_entry(&layout.entry);

                    model::NewLayout {
                        package_id: package_id.to_string(),
                        uid: layout.uid as i32,
                        gid: layout.gid as i32,
                        mode: layout.mode as i32,
                        tag: layout.tag as i32,
                        entry_type,
                        entry_value1,
                        entry_value2,
                    }
                })
                .collect::<Vec<_>>();

            ids.sort();
            ids.dedup();
            batch_remove_impl(&ids, tx)?;

            for chunk in values.chunks(MAX_VARIABLE_NUMBER / 8) {
                diesel::insert_into(model::layout::table).values(chunk).execute(tx)?;
            }

            Ok(())
        })
    }

    pub fn remove(&self, package: &package::Id) -> Result<(), Error> {
        self.batch_remove(Some(package))
    }

    pub fn batch_remove<'a>(&self, packages: impl IntoIterator<Item = &'a package::Id>) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let packages = packages.into_iter().map(package::Id::as_str).collect::<Vec<_>>();

            batch_remove_impl(&packages, tx)?;

            Ok(())
        })
    }
}

fn batch_remove_impl(packages: &[&str], tx: &mut SqliteConnection) -> Result<(), Error> {
    for chunk in packages.chunks(MAX_VARIABLE_NUMBER) {
        diesel::delete(model::layout::table.filter(model::layout::package_id.eq_any(chunk))).execute(tx)?;
    }
    Ok(())
}

fn map_layout(result: QueryResult<model::Layout>) -> Result<(package::Id, payload::Layout), Error> {
    let row = result?;

    let entry = decode_entry(row.entry_type, row.entry_value1, row.entry_value2).ok_or(Error::LayoutEntryDecode)?;

    let layout = payload::Layout {
        uid: row.uid as u32,
        gid: row.gid as u32,
        mode: row.mode as u32,
        tag: row.tag as u32,
        entry,
    };

    Ok((row.package_id, layout))
}

fn decode_entry(
    entry_type: String,
    entry_value1: Option<AStr>,
    entry_value2: Option<AStr>,
) -> Option<payload::layout::Entry> {
    use payload::layout::Entry;

    match entry_type.as_str() {
        "regular" => {
            let hash = entry_value1?.parse::<u128>().ok()?;
            let name = entry_value2?;

            Some(Entry::Regular(hash, name))
        }
        "symlink" => Some(Entry::Symlink(entry_value1?, entry_value2?)),
        "directory" => Some(Entry::Directory(entry_value1?)),
        "character-device" => Some(Entry::CharacterDevice(entry_value1?)),
        "block-device" => Some(Entry::BlockDevice(entry_value1?)),
        "fifo" => Some(Entry::Fifo(entry_value1?)),
        "socket" => Some(Entry::Socket(entry_value1?)),
        _ => None,
    }
}

fn encode_entry(entry: &payload::layout::Entry) -> (&'static str, Option<Cow<'_, str>>, Option<&str>) {
    use payload::layout::Entry;

    match entry {
        Entry::Regular(hash, name) => ("regular", Some(hash.to_string().into()), Some(name)),
        Entry::Symlink(a, b) => ("symlink", Some(a.into()), Some(b)),
        Entry::Directory(name) => ("directory", Some(name.into()), None),
        Entry::CharacterDevice(name) => ("character-device", Some(name.into()), None),
        Entry::BlockDevice(name) => ("block-device", Some(name.into()), None),
        Entry::Fifo(name) => ("fifo", Some(name.into()), None),
        Entry::Socket(name) => ("socket", Some(name.into()), None),
    }
}

mod model {
    use std::borrow::Cow;

    use astr::AStr;
    use diesel::{Selectable, associations::Identifiable, deserialize::Queryable, prelude::Insertable};

    use crate::package;

    pub use super::schema::layout;

    #[derive(Queryable, Selectable, Identifiable)]
    #[diesel(table_name = layout)]
    pub struct Layout {
        pub id: i32,
        #[diesel(deserialize_as = AStr)]
        pub package_id: package::Id,
        pub uid: i32,
        pub gid: i32,
        pub mode: i32,
        pub tag: i32,
        pub entry_type: String,
        pub entry_value1: Option<AStr>,
        pub entry_value2: Option<AStr>,
    }

    #[derive(Insertable)]
    #[diesel(table_name = layout)]
    pub struct NewLayout<'a> {
        pub package_id: String,
        pub uid: i32,
        pub gid: i32,
        pub mode: i32,
        pub tag: i32,
        pub entry_type: &'a str,
        pub entry_value1: Option<Cow<'a, str>>,
        pub entry_value2: Option<&'a str>,
    }
}

#[cfg(test)]
mod test {
    use stone::read::PayloadKind;

    use super::*;

    #[test]
    fn create_insert_select() {
        let database = Database::new(":memory:").unwrap();

        let bash_completion = include_bytes!("../../../../test/bash-completion-2.11-1-1-x86_64.stone");

        let mut stone = stone::read_bytes(bash_completion).unwrap();

        let payloads = stone.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let layouts = payloads
            .iter()
            .filter_map(PayloadKind::layout)
            .flat_map(|p| &p.body)
            .map(|layout| (package::Id::from("test"), layout))
            .collect::<Vec<_>>();

        let count = layouts.len();

        database.batch_add(layouts.iter().map(|(p, l)| (p, *l))).unwrap();

        let all = database.all().unwrap();

        assert_eq!(count, all.len());
    }
}
