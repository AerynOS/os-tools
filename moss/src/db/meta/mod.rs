// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::{BTreeMap, BTreeSet};
use std::iter;
use std::rc::Rc;

use indoc::indoc;
use itertools::Itertools;
use rusqlite::Rows;
use rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER;
use rusqlite::types::{ToSqlOutput, Type, Value, ValueRef};

use crate::db::{Connection, migrations::Migrations};
use crate::package::{self, Meta};
use crate::{Dependency, Provider};

pub use super::Error;

mod types;

const SCHEMAS: &[&str] = &[include_str!("schemas/v1_up.sql")];
const MIGRATIONS: Migrations = Migrations::new(SCHEMAS);

#[derive(Debug)]
pub enum Filter<'a> {
    Id(package::Id),
    Provider(Provider),
    Dependency(Dependency),
    Name(package::Name),
    Keyword(&'a str),
    All,
}

#[derive(Clone, Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(url: &str) -> Result<Self, Error> {
        let mut conn = rusqlite::Connection::open(url)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        rusqlite::vtab::array::load_module(&conn)?;
        MIGRATIONS.migrate(&mut conn, MIGRATIONS.latest())?;
        Ok(Database {
            conn: Connection::new(conn),
        })
    }

    pub fn wipe(&self) -> Result<(), Error> {
        self.conn
            .exec(|conn| Ok(conn.execute("DELETE FROM meta", []).map(|_| ())?))
    }

    pub fn get(&self, package: &package::Id) -> Result<Meta, Error> {
        let entries = self.query(Filter::Id(package.clone()))?;
        if entries.len() > 1 {
            return Err(Error::Dbms(rusqlite::Error::QueryReturnedMoreThanOneRow));
        }
        if let Some((_, meta)) = entries.into_iter().next() {
            Ok(meta)
        } else {
            return Err(Error::Dbms(rusqlite::Error::QueryReturnedNoRows));
        }
    }

    pub fn provider_packages(&self, provider: &Provider) -> Result<Vec<package::Id>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare("SELECT package FROM meta_providers WHERE provider = ?")?;
            stmt.query_and_then([provider.to_string()], |row| {
                Ok(package::Id::from(row.get::<_, String>(0)?))
            })?
            .collect()
        })
    }

    pub fn query(&self, filter: Filter<'_>) -> Result<Vec<(package::Id, Meta)>, Error> {
        self.conn.exec_mut(|conn| {
            let mut stmt;
            let meta_base_query = match filter {
                Filter::Id(id) => {
                    stmt = conn.prepare("SELECT * FROM meta WHERE package = ?")?;
                    stmt.query([id.as_str()])
                }
                Filter::Provider(provider) => {
                    stmt = conn.prepare(indoc! {"
                        SELECT m.*
                        FROM meta m
                        INNER JOIN meta_providers mp ON m.package = mp.package
                        WHERE mp.provider = ?"})?;
                    stmt.query([provider.to_string()])
                }
                Filter::Dependency(dependency) => {
                    stmt = conn.prepare(indoc! {"
                        SELECT m.*
                        FROM meta m
                        INNER JOIN meta_dependencies md ON m.package = md.package
                        WHERE md.dependency = ?"})?;
                    stmt.query([dependency.to_string()])
                }
                Filter::Name(name) => {
                    stmt = conn.prepare("SELECT * FROM meta WHERE name = ?")?;
                    stmt.query([name.to_string()])
                }
                Filter::Keyword(kw) => {
                    stmt = conn.prepare(
                        "SELECT * FROM meta WHERE name LIKE concat('%', ?1, '%') OR summary LIKE concat('%', ?1, '%')",
                    )?;
                    stmt.query([kw.to_string()])
                }
                Filter::All => {
                    stmt = conn.prepare("SELECT * FROM meta")?;
                    stmt.query([])
                }
            }?;

            let mut entries: BTreeMap<package::Id, Meta> = meta_base_query
                .mapped(|row| {
                    let id = package::Id::from(row.get::<_, String>("package")?);
                    let meta = row.try_into()?;
                    Ok((id, meta))
                })
                .collect::<Result<_, _>>()?;

            let ids: Rc<Vec<Value>> = Rc::new(entries.keys().map(|id| Value::from(id.to_string())).collect());
            append_licenses(
                &mut entries,
                conn.prepare("SELECT * from meta_licenses WHERE package in rarray(?)")?
                    .query([ids.clone()])?,
            )?;
            append_dependencies(
                &mut entries,
                conn.prepare("SELECT * from meta_dependencies WHERE package in rarray(?)")?
                    .query([ids.clone()])?,
            )?;
            append_providers(
                &mut entries,
                conn.prepare("SELECT * from meta_providers WHERE package in rarray(?)")?
                    .query([ids.clone()])?,
            )?;
            append_conflicts(
                &mut entries,
                conn.prepare("SELECT * from meta_conflicts WHERE package in rarray(?)")?
                    .query([ids])?,
            )?;

            Ok(entries.into_iter().collect())
        })
    }

    pub fn package_ids(&self) -> Result<BTreeSet<package::Id>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare("SELECT package FROM meta")?;
            stmt.query_and_then([], |row| {
                let id = row.get::<_, String>(0)?.into();
                Ok(id)
            })?
            .collect()
        })
    }

    pub fn file_hashes(&self) -> Result<BTreeSet<String>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare("SELECT DISTINCT hash FROM meta WHERE hash IS NOT NULL")?;
            stmt.query_and_then([], |row| Ok(row.get::<_, String>(0)?))?.collect()
        })
    }

    pub fn add(&mut self, id: package::Id, meta: Meta) -> Result<(), Error> {
        self.batch_add(vec![(id, meta)])
    }

    pub fn batch_add(&mut self, packages: Vec<(package::Id, Meta)>) -> Result<(), Error> {
        let mut ids = Vec::with_capacity(packages.len());
        let mut metas = Vec::with_capacity(packages.len());
        let mut licenses = Vec::with_capacity(packages.len());
        let mut dependencies = Vec::with_capacity(packages.len());
        let mut providers = Vec::with_capacity(packages.len());
        let mut conflicts = Vec::with_capacity(packages.len());
        for (id, meta) in packages.iter() {
            ids.push(id);
            metas.push([
                ToSqlOutput::from(id.as_str()),
                ToSqlOutput::from(meta.name.as_str()),
                ToSqlOutput::from(meta.version_identifier.as_str()),
                ToSqlOutput::from(meta.source_release as i64),
                ToSqlOutput::from(meta.build_release as i64),
                ToSqlOutput::from(meta.architecture.as_str()),
                ToSqlOutput::from(meta.summary.as_str()),
                ToSqlOutput::from(meta.description.as_str()),
                ToSqlOutput::from(meta.source_id.as_str()),
                ToSqlOutput::from(meta.homepage.as_str()),
                ToSqlOutput::Borrowed(ValueRef::from(meta.uri.as_deref())),
                ToSqlOutput::Borrowed(ValueRef::from(meta.hash.as_deref())),
                ToSqlOutput::Borrowed(
                    meta.download_size
                        .map_or(ValueRef::Null, |ds| ValueRef::Integer(ds as i64)),
                ),
            ]);
            for lic in meta.licenses.iter() {
                licenses.push([ToSqlOutput::from(id.as_str()), ToSqlOutput::from(lic.as_str())]);
            }
            for dep in meta.dependencies.iter() {
                dependencies.push([ToSqlOutput::from(id.as_str()), ToSqlOutput::from(dep.to_string())]);
            }
            for prov in meta.providers.iter() {
                providers.push([ToSqlOutput::from(id.as_str()), ToSqlOutput::from(prov.to_string())]);
            }
            for conf in meta.conflicts.iter() {
                conflicts.push([ToSqlOutput::from(id.as_str()), ToSqlOutput::from(conf.to_string())]);
            }
        }

        self.conn.exec_mut(|conn| {
            let tx = conn.transaction()?;

            // Our schema does not have an ON UPDATE clause yet,
            // so we must manually remove and re-insert.
            Self::batch_remove_(&tx, ids)?;

            let num_placeholders = tx.limit(SQLITE_LIMIT_VARIABLE_NUMBER).unwrap() as usize;
            for chunk in metas.chunks(num_placeholders / 13) {
                let mut stmt = tx.prepare_cached(&format!(
                    indoc! {"
                    INSERT OR REPLACE
                    INTO meta
                        (package, name, version_identifier, source_release, build_release,
                        architecture, summary, description, source_id, homepage, uri,
                        hash, download_size)
                    VALUES {}"},
                    iter::repeat_n("(?,?,?,?,?,?,?,?,?,?,?,?,?)", chunk.len()).join(",")
                ))?;
                stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }
            for chunk in licenses.chunks(num_placeholders / 2) {
                let mut stmt = tx.prepare_cached(&format!(
                    "INSERT INTO meta_licenses (package, license) VALUES {}",
                    iter::repeat_n("(?,?)", chunk.len()).join(",")
                ))?;
                stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }
            for chunk in dependencies.chunks(num_placeholders / 2) {
                let mut stmt = tx.prepare_cached(&format!(
                    "INSERT INTO meta_dependencies (package, dependency) VALUES {}",
                    iter::repeat_n("(?,?)", chunk.len()).join(",")
                ))?;
                stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }
            for chunk in providers.chunks(num_placeholders / 2) {
                let mut stmt = tx.prepare_cached(&format!(
                    "INSERT INTO meta_providers (package, provider) VALUES {}",
                    iter::repeat_n("(?,?)", chunk.len()).join(",")
                ))?;
                stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }
            for chunk in conflicts.chunks(num_placeholders / 2) {
                let mut stmt = tx.prepare_cached(&format!(
                    "INSERT INTO meta_conflicts (package, conflict) VALUES {}",
                    iter::repeat_n("(?,?)", chunk.len()).join(",")
                ))?;
                stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }

            Ok(tx.commit()?)
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
        Ok(conn.execute("DELETE FROM meta WHERE package IN rarray(?)", [ids])?).map(|_| ())
    }
}

fn append_licenses(metas: &mut BTreeMap<package::Id, Meta>, rows: Rows<'_>) -> Result<(), Error> {
    let rows = rows.mapped(|row| {
        Ok((
            package::Id::from(row.get::<_, String>("package")?),
            row.get::<_, String>("license")?,
        ))
    });
    for row in rows {
        let (id, license) = row?;
        metas.entry(id).or_default().licenses.push(license);
    }
    Ok(())
}

fn append_dependencies(metas: &mut BTreeMap<package::Id, Meta>, rows: Rows<'_>) -> Result<(), Error> {
    let rows = rows.mapped(|row| {
        Ok((
            package::Id::from(row.get::<_, String>("package")?),
            Dependency::try_from(row.get::<_, String>("dependency")?)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?,
        ))
    });
    for row in rows {
        let (id, dep) = row?;
        metas.entry(id).or_default().dependencies.insert(dep);
    }
    Ok(())
}

fn append_providers(metas: &mut BTreeMap<package::Id, Meta>, rows: Rows<'_>) -> Result<(), Error> {
    let rows = rows.mapped(|row| {
        Ok((
            package::Id::from(row.get::<_, String>("package")?),
            Provider::try_from(row.get::<_, String>("provider")?)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?,
        ))
    });
    for row in rows {
        let (id, prov) = row?;
        metas.entry(id).or_default().providers.insert(prov);
    }
    Ok(())
}

fn append_conflicts(metas: &mut BTreeMap<package::Id, Meta>, rows: Rows<'_>) -> Result<(), Error> {
    let rows = rows.mapped(|row| {
        Ok((
            package::Id::from(row.get::<_, String>("package")?),
            Provider::try_from(row.get::<_, String>("conflict")?)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?,
        ))
    });
    for row in rows {
        let (id, conflict) = row?;
        metas.entry(id).or_default().conflicts.insert(conflict);
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use std::{collections::BTreeSet, iter};

    use crate::dependency::Kind;
    use itertools::Itertools;

    use super::*;

    #[test]
    fn creates_in_memory_db_connection() -> Result<(), Error> {
        Database::new(":memory:").map(|_| ())
    }

    #[test]
    fn db_wipes_all_entries() -> Result<(), Error> {
        let entries = all_entries().take(10).collect::<Vec<_>>();
        let mut db = Database::new(":memory:")?;
        db.batch_add(entries.clone())?;

        assert!(!db.query(Filter::All)?.is_empty());
        db.wipe()?;
        assert!(db.query(Filter::All)?.is_empty());
        Ok(())
    }

    #[test]
    fn db_gets_meta_of_package_id() -> Result<(), Error> {
        let (expected_id, mut expected_meta) = all_entries().nth((NUM_ENTRIES / 2) as usize).unwrap();
        // FIXME: licenses are a one-to-many relationship and Database seems to
        // return the list of licenses, sorted.
        // The license list should likely be a HashSet, since we don't care about the order of licenses
        // *and* they should be unique.
        expected_meta.licenses.sort();

        let meta = create_db()?.get(&expected_id)?;
        assert_eq!(meta, expected_meta);
        Ok(())
    }

    #[test]
    fn db_returns_provider_packages() -> Result<(), Error> {
        let expected_ids =
            all_entries().filter_map(|(id, meta)| meta.providers.contains(&common_provider()).then_some(id));
        let ids = create_db()?.provider_packages(&common_provider())?;
        itertools::assert_equal(ids, expected_ids);
        Ok(())
    }

    #[test]
    fn db_queries_by_provider() -> Result<(), Error> {
        let expected_entries = all_entries()
            .filter_map(|(id, mut meta)| {
                if meta.providers.contains(&common_provider()) {
                    meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                    Some((id, meta))
                } else {
                    None
                }
            })
            .sorted_by(|(id1, _), (id2, _)| id1.cmp(&id2));

        let mut entries = create_db()?.query(Filter::Provider(common_provider()))?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(&id2));

        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_queries_by_dependency() -> Result<(), Error> {
        let dependency = Dependency {
            kind: Kind::PackageName,
            name: "6".to_owned(),
        };
        let expected_entries = all_entries()
            .filter_map(|(id, mut meta)| {
                if meta.dependencies.contains(&dependency) {
                    meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                    Some((id, meta))
                } else {
                    None
                }
            })
            .sorted_by(|(id1, _), (id2, _)| id1.cmp(&id2));

        let mut entries = create_db()?.query(Filter::Dependency(dependency))?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(&id2));

        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_queries_by_name() -> Result<(), Error> {
        let mut expected_entries = all_entries().nth((NUM_ENTRIES / 2) as usize).unwrap();
        expected_entries.1.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
        let meta = create_db()?.query(Filter::Name(package::Name::from(format!("Name {}", NUM_ENTRIES / 2))))?;
        itertools::assert_equal(meta.into_iter(), iter::once(expected_entries));
        Ok(())
    }

    #[test]
    fn db_queries_by_keyword() -> Result<(), Error> {
        {
            // Filter by summary.
            // All packages have "Summary" in their summary, so this should return all packages.
            let expected_entries = all_entries()
                .map(|(id, mut meta)| {
                    meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                    (id, meta)
                })
                .sorted_by(|(id1, _), (id2, _)| id1.cmp(&id2));
            let mut entries = create_db()?.query(Filter::Keyword("Summary"))?;
            entries.sort_by(|(id1, _), (id2, _)| id1.cmp(&id2));
            itertools::assert_equal(entries.into_iter(), expected_entries);
        }
        {
            // Filter by name this time.
            let expected_entries = all_entries()
                .filter_map(|(id, mut meta)| {
                    if meta.name.contains("Name 10") {
                        meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                        Some((id, meta))
                    } else {
                        None
                    }
                })
                .sorted_by(|(id1, _), (id2, _)| id1.cmp(&id2));
            let mut entries = create_db()?.query(Filter::Keyword("Name 10"))?;
            entries.sort_by(|(id1, _), (id2, _)| id1.cmp(&id2));
            itertools::assert_equal(entries.into_iter(), expected_entries);
        }

        Ok(())
    }

    #[test]
    fn db_queries_all() -> Result<(), Error> {
        let expected_entries = all_entries()
            .map(|(id, mut meta)| {
                meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                (id, meta)
            })
            .sorted_by(|(id1, _), (id2, _)| id1.cmp(id2));
        let mut entries = create_db()?.query(Filter::All)?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(id2));
        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_returns_package_ids() -> Result<(), Error> {
        let ids = create_db()?.package_ids()?;
        let expected_ids: std::collections::BTreeSet<_> = all_entries().map(|(id, _)| id.clone()).collect();
        itertools::assert_equal(ids, expected_ids);
        Ok(())
    }

    #[test]
    fn db_returns_file_hashes() -> Result<(), Error> {
        let hashes = create_db()?.file_hashes()?;
        let expected_hashes = all_entries()
            .filter_map(|(_, meta)| meta.hash.clone())
            .collect::<BTreeSet<_>>();
        itertools::assert_equal(hashes, expected_hashes);
        Ok(())
    }

    #[test]
    fn db_adds_one_entry() -> Result<(), Error> {
        let (expected_id, mut expected_meta) = all_entries().next().unwrap();
        expected_meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
        let mut db = Database::new(":memory:")?;
        db.add(expected_id.clone(), expected_meta.clone())?;

        let entries = db.query(Filter::All)?;
        itertools::assert_equal(entries.into_iter(), iter::once((expected_id, expected_meta)));
        Ok(())
    }

    #[test]
    fn db_adds_multiple_entries() -> Result<(), Error> {
        let mut db = Database::new(":memory:")?;
        db.batch_add(all_entries().take((NUM_ENTRIES / 2) as usize).collect())?;

        let mut entries = db.query(Filter::All)?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(id2));

        let expected_entries = all_entries()
            .take((NUM_ENTRIES / 2) as usize)
            .map(|(id, mut meta)| {
                meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                (id, meta)
            })
            .sorted_by(|(id1, _), (id2, _)| id1.cmp(id2));
        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_removes_one_entry() -> Result<(), Error> {
        let all_entries = all_entries()
            .take(10)
            .map(|(id, mut meta)| {
                meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                (id, meta)
            })
            .sorted_by(|(id1, _), (id2, _)| id1.cmp(id2))
            .collect::<Vec<_>>();
        let mut db = Database::new(":memory:")?;
        db.batch_add(all_entries.clone())?;

        db.remove(&all_entries.last().unwrap().0)?;

        let mut entries = db.query(Filter::All)?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(id2));
        itertools::assert_equal(entries.into_iter(), all_entries.into_iter().take(9));
        Ok(())
    }

    #[test]
    fn db_removes_multiple_entries() -> Result<(), Error> {
        let all_entries = all_entries()
            .take(10)
            .map(|(id, mut meta)| {
                meta.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
                (id, meta)
            })
            .sorted_by(|(id1, _), (id2, _)| id1.cmp(id2))
            .collect::<Vec<_>>();
        let mut db = Database::new(":memory:")?;
        db.batch_add(all_entries.clone())?;

        db.batch_remove(&all_entries.iter().take(5).map(|(id, _)| id.clone()).collect::<Vec<_>>())?;

        let mut entries = db.query(Filter::All)?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(id2));
        itertools::assert_equal(entries.into_iter(), all_entries.into_iter().skip(5));
        Ok(())
    }

    fn create_db() -> Result<Database, Error> {
        let entries = all_entries().collect::<Vec<_>>();
        let mut db = Database::new(":memory:").unwrap();
        db.batch_add(entries).unwrap();
        Ok(db)
    }

    const NUM_ENTRIES: u32 = 101;

    fn all_entries() -> impl Iterator<Item = (package::Id, Meta)> {
        // We use the modulo operator to create some variety
        // in the generated entries, while ensuring that
        // we have a deterministic set of entries for testing.
        (0..NUM_ENTRIES).map(move |i| {
            let mut dependencies = BTreeSet::new();
            if i % 5 == 0 {
                dependencies.insert(Dependency {
                    kind: Kind::PackageName,
                    name: format!("{}", i + 1),
                });
            }
            if i % 7 == 0 {
                dependencies.insert(Dependency {
                    kind: Kind::SharedLibrary,
                    name: format!("libc.so.{}", i % 3),
                });
            }

            let mut providers = BTreeSet::from_iter(iter::once(Provider {
                kind: Kind::PackageName,
                name: i.to_string(),
            }));
            if i % 3 == 0 {
                providers.insert(Provider {
                    kind: Kind::PkgConfig,
                    name: format!("pkg-{i}"),
                });
                providers.insert(common_provider());
            }

            let mut conflicts = BTreeSet::new();
            if i % 11 == 0 {
                conflicts.insert(Provider {
                    kind: Kind::PackageName,
                    name: format!("conflicting-package-{i}"),
                });
            }

            let mut licenses = vec!["MPL-2.0".to_string()];
            if i % 2 == 0 {
                licenses.push("MIT".to_string());
            }

            (
                package::Id::from(i.to_string()),
                Meta {
                    name: package::Name::from(format!("Name {}", i)),
                    version_identifier: format!("{i}.0.0"),
                    source_release: i as u64,
                    build_release: i as u64,
                    architecture: "x86_64".to_string(),
                    summary: format!("Summary {i}"),
                    description: format!("Description {i}"),
                    source_id: i.to_string(),
                    homepage: format!("https://example.com/{i}"),
                    licenses,
                    dependencies,
                    providers,
                    conflicts,
                    uri: (i % 2 == 0).then_some(format!("https://repo.example.com/{i}.stone")),
                    hash: (i % 2 != 0).then_some(format!("{i:032x}")),
                    download_size: (i % 2 == 0).then_some(1024 * (i as u64 + 1)),
                },
            )
        })
    }

    /// Returns a provider that is shared across
    /// multiple packages, to test that multiple
    /// package IDs can be associated to the same provider.
    fn common_provider() -> Provider {
        Provider {
            kind: Kind::SharedLibrary,
            name: "libcommon.so.6".to_owned(),
        }
    }
}
