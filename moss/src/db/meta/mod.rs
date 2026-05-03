// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeSet;
use std::iter;

use indoc::indoc;

use crate::db::{Connection, migrations::Migrations};
use crate::package::{self, Meta};
use crate::{Dependency, Provider};

pub use super::Error;

mod types;

const SCHEMAS: &[&str] = &[include_str!("schemas/v1_up.sql")];
const MIGRATIONS: Migrations = Migrations::new(SCHEMAS);

#[derive(Debug)]
pub enum Filter<'a> {
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
        self.conn.exec(|conn| {
            let query = format!("{META_QUERY} WHERE m.package = ? GROUP BY m.package");
            let mut stmt = conn.prepare(&query)?;
            stmt.query_one([package.as_str()], |row| row.try_into())
                .map_err(Error::from)
        })
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
        self.conn.exec(|conn| {
            let (where_, having, params) = match &filter {
                // These filters operate on the one-to-many relationships of of the meta table,
                // so they use the HAVING clause.
                Filter::Provider(provider) => ("", "HAVING SUM(mp.provider = ?) > 0", [provider.to_string()]),
                Filter::Dependency(dependency) => ("", "HAVING SUM(md.dependency = ?) > 0", [dependency.to_string()]),

                // These filters operate directly on the meta table,
                // so they use the WHERE clause.
                Filter::Name(name) => ("WHERE name = ?", "", [name.to_string()]),
                Filter::Keyword(keyword) => (
                    "WHERE name LIKE concat('%', ?1, '%') OR summary LIKE concat('%', ?1, '%')",
                    "",
                    [keyword.to_string()],
                ),

                Filter::All => ("", "", ["".to_string()]),
            };

            let query = format!("{META_QUERY} {where_} GROUP BY m.package {having}");

            let mut stmt = conn.prepare(&query)?;
            let mut rows = if let Filter::All = filter {
                stmt.query([])
            } else {
                stmt.query(params)
            }?;

            let mut metas = Vec::new();
            while let Some(row) = rows.next()? {
                metas.push((row.get::<_, String>("package")?.into(), row.try_into()?));
            }
            Ok(metas)
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
            let mut stmt = conn.prepare("SELECT hash FROM meta WHERE hash IS NOT NULL")?;
            stmt.query_and_then([], |row| Ok(row.get::<_, String>(0)?))?.collect()
        })
    }

    pub fn add(&mut self, id: package::Id, meta: Meta) -> Result<(), Error> {
        self.batch_add(vec![(id, meta)])
    }

    pub fn batch_add(&mut self, packages: Vec<(package::Id, Meta)>) -> Result<(), Error> {
        self.conn.exec_mut(|conn| {
            let tx = conn.transaction()?;
            {
                let mut meta = tx.prepare(indoc! {"
                    INSERT OR REPLACE
                    INTO meta
                        (package, name, version_identifier, source_release, build_release,
                        architecture, summary, description, source_id, homepage, uri,
                        hash, download_size)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "})?;
                let mut licenses = tx.prepare("INSERT INTO meta_licenses (package, license) VALUES (?, ?)")?;
                let mut dependencies =
                    tx.prepare("INSERT INTO meta_dependencies (package, dependency) VALUES (?, ?)")?;
                let mut providers = tx.prepare("INSERT INTO meta_providers (package, provider) VALUES (?, ?)")?;
                let mut conflicts = tx.prepare("INSERT INTO meta_conflicts (package, conflict) VALUES (?, ?)")?;

                for (id, val) in packages.iter() {
                    meta.execute((
                        id.to_string(),
                        val.name.to_string(),
                        &val.version_identifier,
                        val.source_release as i32,
                        val.build_release as i32,
                        &val.architecture,
                        &val.summary,
                        &val.description,
                        &val.source_id,
                        &val.homepage,
                        val.uri.as_deref(),
                        val.hash.as_deref(),
                        val.download_size.map(|size| size as i64),
                    ))?;
                    for license in &val.licenses {
                        licenses.execute((id.to_string(), license))?;
                    }
                    for dep in &val.dependencies {
                        dependencies.execute((id.to_string(), dep.to_string()))?;
                    }
                    for prov in &val.providers {
                        providers.execute((id.to_string(), prov.to_string()))?;
                    }
                    for conf in &val.conflicts {
                        conflicts.execute((id.to_string(), conf.to_string()))?;
                    }
                }
            }
            Ok(tx.commit()?)
        })
    }

    pub fn remove(&mut self, package: &package::Id) -> Result<(), Error> {
        self.batch_remove(iter::once(package))
    }

    pub fn batch_remove<'a>(&mut self, packages: impl IntoIterator<Item = &'a package::Id>) -> Result<(), Error> {
        self.conn.exec_mut(|conn| {
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare("DELETE FROM meta WHERE package = ?")?;
                for package in packages {
                    stmt.execute((package.to_string(),))?;
                }
            }
            Ok(tx.commit()?)
        })
    }
}

const META_QUERY: &str = indoc! {"
    SELECT
        m.*,
        json_group_array(DISTINCT ml.license)        AS licenses,
        json_group_array(DISTINCT md.dependency)
            FILTER (WHERE md.dependency IS NOT NULL) AS dependencies,
        json_group_array(DISTINCT mp.provider)       AS providers,
        json_group_array(DISTINCT mc.conflict)
            FILTER (WHERE mc.conflict IS NOT NULL)   AS conflicts
    FROM meta m
    JOIN meta_licenses ml ON m.package = ml.package
    LEFT JOIN meta_dependencies md ON m.package = md.package
    JOIN meta_providers mp ON m.package = mp.package
    LEFT JOIN meta_conflicts mc ON m.package = mc.package
"};

#[cfg(test)]
mod test {
    use std::{collections::BTreeSet, iter, sync::LazyLock};

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

        let meta = SHARED_DB.get(&expected_id)?;
        assert_eq!(meta, expected_meta);
        Ok(())
    }

    #[test]
    fn db_returns_provider_packages() -> Result<(), Error> {
        let expected_ids =
            all_entries().filter_map(|(id, meta)| meta.providers.contains(&common_provider()).then_some(id));
        let ids = SHARED_DB.provider_packages(&common_provider())?;
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

        let mut entries = SHARED_DB.query(Filter::Provider(common_provider()))?;
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

        let mut entries = SHARED_DB.query(Filter::Dependency(dependency))?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(&id2));

        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_queries_by_name() -> Result<(), Error> {
        let mut expected_entries = all_entries().nth((NUM_ENTRIES / 2) as usize).unwrap();
        expected_entries.1.licenses.sort(); // FIXME: See db_gets_meta_of_package_id().
        let meta = SHARED_DB.query(Filter::Name(package::Name::from(format!("Name {}", NUM_ENTRIES / 2))))?;
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
            let mut entries = SHARED_DB.query(Filter::Keyword("Summary"))?;
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
            let mut entries = SHARED_DB.query(Filter::Keyword("Name 10"))?;
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
        let mut entries = SHARED_DB.query(Filter::All)?;
        entries.sort_by(|(id1, _), (id2, _)| id1.cmp(id2));
        itertools::assert_equal(entries.into_iter(), expected_entries);
        Ok(())
    }

    #[test]
    fn db_returns_package_ids() -> Result<(), Error> {
        let ids = SHARED_DB.package_ids()?;
        let expected_ids: std::collections::BTreeSet<_> = all_entries().map(|(id, _)| id.clone()).collect();
        itertools::assert_equal(ids, expected_ids);
        Ok(())
    }

    #[test]
    fn db_returns_file_hashes() -> Result<(), Error> {
        let hashes = SHARED_DB.file_hashes()?;
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

    const NUM_ENTRIES: u32 = 101;

    static SHARED_DB: LazyLock<Database> = LazyLock::new(|| {
        let entries = all_entries().collect::<Vec<_>>();
        let mut db = Database::new(":memory:").unwrap();
        db.batch_add(entries).unwrap();
        db
    });

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
