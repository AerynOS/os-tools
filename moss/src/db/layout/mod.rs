// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use diesel::prelude::*;
use diesel::{Connection as _, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std::collections::BTreeSet;

use stone::payload;

use crate::package;

use super::Connection;
pub use super::Error;

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
            let packages = packages.into_iter().map(AsRef::<str>::as_ref).collect::<Vec<_>>();

            model::layout::table
                .select(model::Layout::as_select())
                .filter(model::layout::package_id.eq_any(packages))
                .load_iter(conn)?
                .map(map_layout)
                .collect()
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

    pub fn add(&self, package: package::Id, layout: payload::Layout) -> Result<(), Error> {
        self.batch_add(vec![(package, layout)])
    }

    pub fn batch_add(&self, layouts: Vec<(package::Id, payload::Layout)>) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let values = layouts
                .into_iter()
                .map(|(package_id, layout)| {
                    let (entry_type, entry_value1, entry_value2) = encode_entry(layout.entry);

                    model::NewLayout {
                        package_id: package_id.into(),
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

            diesel::insert_into(model::layout::table).values(values).execute(tx)?;

            Ok(())
        })
    }

    pub fn remove(&self, package: &package::Id) -> Result<(), Error> {
        self.batch_remove(Some(package))
    }

    pub fn batch_remove<'a>(&self, packages: impl IntoIterator<Item = &'a package::Id>) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let packages = packages.into_iter().map(AsRef::<str>::as_ref).collect::<Vec<_>>();

            diesel::delete(model::layout::table.filter(model::layout::package_id.eq_any(packages))).execute(tx)?;

            Ok(())
        })
    }
}

fn map_layout(result: diesel::QueryResult<model::Layout>) -> Result<(package::Id, payload::Layout), Error> {
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
    entry_value1: Option<String>,
    entry_value2: Option<String>,
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

fn encode_entry(entry: payload::layout::Entry) -> (&'static str, Option<String>, Option<String>) {
    use payload::layout::Entry;

    match entry {
        Entry::Regular(hash, name) => ("regular", Some(hash.to_string()), Some(name)),
        Entry::Symlink(a, b) => ("symlink", Some(a), Some(b)),
        Entry::Directory(name) => ("directory", Some(name), None),
        Entry::CharacterDevice(name) => ("character-device", Some(name), None),
        Entry::BlockDevice(name) => ("block-device", Some(name), None),
        Entry::Fifo(name) => ("fifo", Some(name), None),
        Entry::Socket(name) => ("socket", Some(name), None),
    }
}

mod model {
    use diesel::{associations::Identifiable, deserialize::Queryable, prelude::Insertable, Selectable};

    use crate::package;

    pub use super::schema::layout;

    #[derive(Queryable, Selectable, Identifiable)]
    #[diesel(table_name = layout)]
    pub struct Layout {
        pub id: i32,
        #[diesel(deserialize_as = String)]
        pub package_id: package::Id,
        pub uid: i32,
        pub gid: i32,
        pub mode: i32,
        pub tag: i32,
        pub entry_type: String,
        pub entry_value1: Option<String>,
        pub entry_value2: Option<String>,
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
        pub entry_value1: Option<String>,
        pub entry_value2: Option<String>,
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
            .cloned()
            .map(|layout| (package::Id::from("test".to_string()), layout))
            .collect::<Vec<_>>();

        let count = layouts.len();

        database.batch_add(layouts).unwrap();

        let all = database.all().unwrap();

        assert_eq!(count, all.len());
    }
}
