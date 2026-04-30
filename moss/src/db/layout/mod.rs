// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use astr::AStr;
use diesel::prelude::*;
use diesel::{Connection as _, SqliteConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use std::{borrow::Cow, collections::BTreeSet};

use stone::{StonePayloadLayoutFile, StonePayloadLayoutRecord};

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
    ) -> Result<Vec<(package::Id, StonePayloadLayoutRecord)>, Error> {
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

    pub fn all(&self) -> Result<Vec<(package::Id, StonePayloadLayoutRecord)>, Error> {
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

    pub fn add(&self, package: &package::Id, layout: &StonePayloadLayoutRecord) -> Result<(), Error> {
        self.batch_add(vec![(package, layout)])
    }

    pub fn batch_add<'a>(
        &self,
        layouts: impl IntoIterator<Item = (&'a package::Id, &'a StonePayloadLayoutRecord)>,
    ) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let mut ids = vec![];

            let values = layouts
                .into_iter()
                .map(|(package_id, layout)| {
                    ids.push(package_id.as_str());

                    let (entry_type, entry_value1, entry_value2) = encode_entry(&layout.file);

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

fn map_layout(result: QueryResult<model::Layout>) -> Result<(package::Id, StonePayloadLayoutRecord), Error> {
    let row = result?;

    let entry = decode_entry(row.entry_type, row.entry_value1, row.entry_value2).ok_or(Error::LayoutEntryDecode)?;

    let layout = StonePayloadLayoutRecord {
        uid: row.uid as u32,
        gid: row.gid as u32,
        mode: row.mode as u32,
        tag: row.tag as u32,
        file: entry,
    };

    Ok((row.package_id, layout))
}

fn decode_entry(
    entry_type: String,
    entry_value1: Option<AStr>,
    entry_value2: Option<AStr>,
) -> Option<StonePayloadLayoutFile> {
    match entry_type.as_str() {
        "regular" => {
            let hash = entry_value1?.parse::<u128>().ok()?;
            let name = entry_value2?;

            Some(StonePayloadLayoutFile::Regular(hash, name))
        }
        "symlink" => Some(StonePayloadLayoutFile::Symlink(entry_value1?, entry_value2?)),
        "directory" => Some(StonePayloadLayoutFile::Directory(entry_value1?)),
        "character-device" => Some(StonePayloadLayoutFile::CharacterDevice(entry_value1?)),
        "block-device" => Some(StonePayloadLayoutFile::BlockDevice(entry_value1?)),
        "fifo" => Some(StonePayloadLayoutFile::Fifo(entry_value1?)),
        "socket" => Some(StonePayloadLayoutFile::Socket(entry_value1?)),
        "unknown" => Some(StonePayloadLayoutFile::Unknown(entry_value1?, entry_value2?)),
        _ => None,
    }
}

fn encode_entry(entry: &StonePayloadLayoutFile) -> (&'static str, Option<Cow<'_, str>>, Option<&str>) {
    match entry {
        StonePayloadLayoutFile::Regular(hash, name) => ("regular", Some(hash.to_string().into()), Some(name)),
        StonePayloadLayoutFile::Symlink(a, b) => ("symlink", Some(a.into()), Some(b)),
        StonePayloadLayoutFile::Directory(name) => ("directory", Some(name.into()), None),
        StonePayloadLayoutFile::CharacterDevice(name) => ("character-device", Some(name.into()), None),
        StonePayloadLayoutFile::BlockDevice(name) => ("block-device", Some(name.into()), None),
        StonePayloadLayoutFile::Fifo(name) => ("fifo", Some(name.into()), None),
        StonePayloadLayoutFile::Socket(name) => ("socket", Some(name.into()), None),
        StonePayloadLayoutFile::Unknown(a, b) => ("unknown", Some(a.into()), Some(b)),
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
    use std::{iter, sync::LazyLock};

    use crate::db::Error;
    use itertools::Itertools;

    use super::*;

    #[test]
    fn creates_in_memory_db_connection() -> Result<(), Error> {
        Database::new(":memory:").map(|_| ())
    }

    #[test]
    fn db_queries_package_id() -> Result<(), Error> {
        let non_unique_id = package::Id::from(NUM_ENTRIES.to_string());
        let entries = SHARED_DB.query(iter::once(&non_unique_id))?;
        let expected_entries = all_entries().filter(|(id, _)| id == &non_unique_id);
        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_returns_all_entries() -> Result<(), Error> {
        let entries = SHARED_DB.all()?;
        itertools::assert_equal(entries.into_iter(), all_entries());
        Ok(())
    }

    #[test]
    fn db_returns_package_ids() -> Result<(), Error> {
        let ids = SHARED_DB.package_ids()?;
        let expected_ids = all_entries().map(|(id, _)| id);
        // FIXME? BTreeSet sorts its elements according to the Ord trait
        // of the elements. But maybe the intention was to return package IDs
        // in order they are inside the database?
        itertools::assert_equal(ids.into_iter(), expected_ids.sorted());
        Ok(())
    }

    #[test]
    fn db_returns_file_hashes() -> Result<(), Error> {
        let hashes = SHARED_DB.file_hashes()?;
        let expected_hashes = all_entries().filter_map(|(_, record)| {
            if let StonePayloadLayoutFile::Regular(hash, _) = record.file {
                Some(format!("{hash:02x}"))
            } else {
                None
            }
        });
        // FIXME? Same as the test above.
        itertools::assert_equal(hashes.into_iter(), expected_hashes.sorted());
        Ok(())
    }

    #[test]
    fn db_adds_one_entry() -> Result<(), Error> {
        let (expected_id, expected_record) = all_entries().next().unwrap();
        let db = Database::new(":memory:")?;
        db.add(&expected_id, &expected_record)?;
        assert_eq!(db.all()?, vec![(expected_id, expected_record)]);
        Ok(())
    }

    #[test]
    fn db_adds_multiple_entries() -> Result<(), Error> {
        // FIXME: Same observation as in SHARED_DB.
        let expected_entries = all_entries().take(10).collect::<Vec<_>>();
        let expected_entries_ref = expected_entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();
        let db = Database::new(":memory:")?;
        db.batch_add(expected_entries_ref)?;
        assert_eq!(db.all()?, expected_entries);
        Ok(())
    }

    #[test]
    fn db_removes_one_entry() -> Result<(), Error> {
        // FIXME: Same observation as in SHARED_DB.
        let entries = all_entries().take(10).collect::<Vec<_>>();
        let entries_ref = entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();
        let db = Database::new(":memory:")?;
        db.batch_add(entries_ref)?;

        db.remove(&entries.last().unwrap().0)?;
        itertools::assert_equal(db.all()?.iter(), entries.iter().take(9));
        Ok(())
    }

    #[test]
    fn db_removes_multiple_entries() -> Result<(), Error> {
        // FIXME: Same observation as in SHARED_DB.
        let entries = all_entries().take(10).collect::<Vec<_>>();
        let entries_ref = entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();
        let db = Database::new(":memory:")?;
        db.batch_add(entries_ref)?;

        db.batch_remove(entries.iter().take(5).map(|(id, _)| id))?;
        itertools::assert_equal(db.all()?.iter(), entries[5..].iter());
        Ok(())
    }

    const NUM_ENTRIES: u32 = 512;

    static SHARED_DB: LazyLock<Database> = LazyLock::new(|| {
        // FIXME: This allocates a lot. Maybe we can make batched functions
        // in mod.rs accept Borrow, so that it works with both owned and referenced values?
        let entries = all_entries().collect::<Vec<_>>();
        let entries_ref = entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();

        let db = Database::new(":memory:").unwrap();
        db.batch_add(entries_ref).unwrap();
        db
    });

    fn all_entries() -> impl Iterator<Item = (package::Id, StonePayloadLayoutRecord)> {
        let file_from_index = |index: u32| -> StonePayloadLayoutFile {
            let i_str = index.to_string();
            match index % 8 {
                0 => StonePayloadLayoutFile::Regular(index as u128, AStr::from(i_str)),
                1 => StonePayloadLayoutFile::Symlink(AStr::from(i_str.clone()), AStr::from(i_str)),
                2 => StonePayloadLayoutFile::Directory(AStr::from(i_str)),
                3 => StonePayloadLayoutFile::CharacterDevice(AStr::from(i_str)),
                4 => StonePayloadLayoutFile::BlockDevice(AStr::from(i_str)),
                5 => StonePayloadLayoutFile::Fifo(AStr::from(i_str)),
                6 => StonePayloadLayoutFile::Socket(AStr::from(i_str)),
                _ => StonePayloadLayoutFile::Unknown(AStr::from(i_str.clone()), AStr::from(i_str)),
            }
        };
        let record_from_index = move |index: u32| -> StonePayloadLayoutRecord {
            StonePayloadLayoutRecord {
                uid: index,
                gid: index,
                mode: index,
                tag: index,
                file: file_from_index(index),
            }
        };

        (0..NUM_ENTRIES)
            .map(move |i| (package::Id::from(i.to_string()), record_from_index(i)))
            // Package IDs are not unique. Ensure there is at least
            // one ID listed twice to test this possibility.
            .chain(iter::once((
                package::Id::from(NUM_ENTRIES.to_string()),
                record_from_index(NUM_ENTRIES),
            )))
    }
}
