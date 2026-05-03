// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;
use std::iter;
use std::rc::Rc;

use indoc::indoc;
use itertools::Itertools;
use rusqlite::types::ToSqlOutput;
use rusqlite::{
    limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER,
    types::{Type, Value},
};
use stone::StonePayloadLayoutRecord;

use crate::{
    db::{Connection, Error, migrations::Migrations},
    package,
};
use types::{Layout, encode_file};

mod types;

const SCHEMAS: &[&str] = &[include_str!("schemas/v1_up.sql")];
const MIGRATIONS: Migrations = Migrations::new(SCHEMAS);

#[derive(Clone, Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(url: &str) -> Result<Self, Error> {
        let mut conn = rusqlite::Connection::open(url)?;
        rusqlite::vtab::array::load_module(&conn)?;
        MIGRATIONS.migrate(&mut conn, MIGRATIONS.latest())?;
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
            let ids: Rc<Vec<Value>> = Rc::new(packages.into_iter().map(|id| Value::from(id.to_string())).collect());
            let mut stmt = conn.prepare("SELECT * FROM layout WHERE package_id IN rarray(?)")?;
            Ok(stmt
                .query_map([ids], |row| {
                    let id = row.get::<_, String>("package_id")?.into();
                    let layout = TryInto::<Layout>::try_into(row)?.into_inner();
                    Ok((id, layout))
                })?
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn all(&self) -> Result<Vec<(package::Id, StonePayloadLayoutRecord)>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM layout")?;
            Ok(stmt
                .query_map([], |row| {
                    let id = row.get::<_, String>("package_id")?.into();
                    let layout = TryInto::<Layout>::try_into(row)?.into_inner();
                    Ok((id, layout))
                })?
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn package_ids(&self) -> Result<BTreeSet<package::Id>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare("SELECT package_id FROM layout")?;
            Ok(stmt
                .query_map([], |row| {
                    let id = row.get::<_, String>(0)?.into();
                    Ok(id)
                })?
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn file_hashes(&self) -> Result<BTreeSet<String>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare("SELECT entry_value1 FROM layout WHERE entry_type = 'regular'")?;
            Ok(stmt
                .query_map([], |row| {
                    let hash_str = row.get::<_, String>(0)?;
                    let hash = hash_str
                        .parse::<u128>()
                        .map(|hash| format!("{hash:02x}"))
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?;
                    Ok(hash)
                })?
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn add(&mut self, package: &package::Id, layout: &StonePayloadLayoutRecord) -> Result<(), Error> {
        self.batch_add(iter::once((package, layout)))
    }

    pub fn batch_add<'a>(
        &mut self,
        layouts: impl IntoIterator<Item = (&'a package::Id, &'a StonePayloadLayoutRecord)>,
    ) -> Result<(), Error> {
        let layouts_iter = layouts.into_iter();
        let mut ids = Vec::with_capacity(layouts_iter.size_hint().1.unwrap_or(0));
        let mut records = Vec::with_capacity(ids.capacity());
        for (id, rec) in layouts_iter {
            ids.push(id);
            let file = encode_file(&rec.file);
            records.push([
                ToSqlOutput::from(id.as_str()),
                ToSqlOutput::from(rec.uid),
                ToSqlOutput::from(rec.gid),
                ToSqlOutput::from(rec.mode),
                ToSqlOutput::from(rec.tag),
                ToSqlOutput::from(file.type_),
                ToSqlOutput::Owned(file.value1.map_or(Value::Null, |v| Value::from(v.into_owned()))),
                ToSqlOutput::Owned(file.value2.map_or(Value::Null, |v| Value::from(v.to_owned()))),
            ]);
        }
        ids.sort();
        ids.dedup();

        self.conn.exec_mut(|conn| {
            let tx = conn.transaction()?;

            // We could just ON CONFLICT DO NOTHING, but package_id is currently
            // not UNIQUE in the schema.
            Self::batch_remove_(&tx, ids)?;

            let num_placeholders = tx.limit(SQLITE_LIMIT_VARIABLE_NUMBER).unwrap() as usize;
            for chunk in records.chunks(num_placeholders / 8) {
                let mut stmt = tx.prepare_cached(&format!(
                    indoc! {"
                        INSERT INTO layout
                            (package_id, uid, gid, mode, tag, entry_type, entry_value1, entry_value2)
                        VALUES
                            {}
                        "},
                    iter::repeat_n("(?,?,?,?,?,?,?,?)", chunk.len()).join(",")
                ))?;
                stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }

            Ok(tx.commit().map(|_| ())?)
        })
    }

    pub fn remove(&mut self, package: &package::Id) -> Result<(), Error> {
        self.batch_remove(iter::once(package))
    }

    pub fn batch_remove<'a>(&mut self, packages: impl IntoIterator<Item = &'a package::Id>) -> Result<(), Error> {
        self.conn.exec_mut(|conn| Self::batch_remove_(conn, packages))
    }

    fn batch_remove_<'a>(
        conn: &rusqlite::Connection,
        packages: impl IntoIterator<Item = &'a package::Id>,
    ) -> Result<(), Error> {
        let ids: Rc<Vec<Value>> = Rc::new(packages.into_iter().map(|id| Value::from(id.to_string())).collect());
        Ok(conn.execute("DELETE FROM layout WHERE package_id IN rarray(?)", [ids])?).map(|_| ())
    }
}

#[cfg(test)]
mod test {
    use std::iter;

    use crate::db::Error;
    use astr::AStr;
    use itertools::Itertools;
    use stone::StonePayloadLayoutFile;

    use super::*;

    #[test]
    fn creates_in_memory_db_connection() -> Result<(), Error> {
        Database::new(":memory:").map(|_| ())
    }

    #[test]
    fn db_queries_package_id() -> Result<(), Error> {
        let non_unique_id = package::Id::from(NUM_ENTRIES.to_string());
        let entries = create_db()?.query(iter::once(&non_unique_id))?;
        let expected_entries = all_entries().filter(|(id, _)| id == &non_unique_id);
        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_returns_all_entries() -> Result<(), Error> {
        let entries = create_db()?.all()?;
        itertools::assert_equal(entries.into_iter(), all_entries());
        Ok(())
    }

    #[test]
    fn db_returns_package_ids() -> Result<(), Error> {
        let ids = create_db()?.package_ids()?;
        let expected_ids = all_entries().map(|(id, _)| id);
        // FIXME? BTreeSet sorts its elements according to the Ord trait
        // of the elements. But maybe the intention was to return package IDs
        // in order they are inside the database?
        itertools::assert_equal(ids.into_iter(), expected_ids.sorted());
        Ok(())
    }

    #[test]
    fn db_returns_file_hashes() -> Result<(), Error> {
        let hashes = create_db()?.file_hashes()?;
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
        let mut db = Database::new(":memory:")?;
        db.add(&expected_id, &expected_record)?;
        assert_eq!(db.all()?, vec![(expected_id, expected_record)]);
        Ok(())
    }

    #[test]
    fn db_adds_multiple_entries() -> Result<(), Error> {
        let expected_entries = all_entries().take(10).collect::<Vec<_>>();
        let expected_entries_ref = expected_entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();
        let mut db = Database::new(":memory:")?;
        db.batch_add(expected_entries_ref)?;
        assert_eq!(db.all()?, expected_entries);
        Ok(())
    }

    #[test]
    fn db_removes_one_entry() -> Result<(), Error> {
        let entries = all_entries().take(10).collect::<Vec<_>>();
        let entries_ref = entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();
        let mut db = Database::new(":memory:")?;
        db.batch_add(entries_ref)?;

        db.remove(&entries.last().unwrap().0)?;
        itertools::assert_equal(db.all()?.iter(), entries.iter().take(9));
        Ok(())
    }

    #[test]
    fn db_removes_multiple_entries() -> Result<(), Error> {
        let entries = all_entries().take(10).collect::<Vec<_>>();
        let entries_ref = entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();
        let mut db = Database::new(":memory:")?;
        db.batch_add(entries_ref)?;

        db.batch_remove(entries.iter().take(5).map(|(id, _)| id))?;
        itertools::assert_equal(db.all()?.iter(), entries[5..].iter());
        Ok(())
    }

    fn create_db() -> Result<Database, Error> {
        let entries = all_entries().collect::<Vec<_>>();
        let entries_ref = entries.iter().map(|(id, rec)| (id, rec)).collect::<Vec<_>>();

        let mut db = Database::new(":memory:").unwrap();
        db.batch_add(entries_ref).unwrap();
        Ok(db)
    }

    const NUM_ENTRIES: u32 = 512;

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
