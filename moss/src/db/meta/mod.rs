// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::{BTreeMap, BTreeSet};

use astr::AStr;
use diesel::prelude::*;
use diesel::{Connection as _, SqliteConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use crate::db::Connection;
use crate::package::{self, Meta};
use crate::{Dependency, Provider};

pub use super::Error;
use super::MAX_VARIABLE_NUMBER;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("src/db/meta/migrations");

mod schema;

#[derive(Debug)]
pub enum Filter<'a> {
    Provider(Provider),
    Dependency(Dependency),
    Name(package::Name),
    Keyword(&'a str),
}

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

    pub fn wipe(&self) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            // Cascading wipes other tables
            diesel::delete(model::meta::table).execute(tx)?;
            Ok(())
        })
    }

    pub fn get(&self, package: &package::Id) -> Result<Meta, Error> {
        self.conn.exec(|conn| {
            let meta = model::meta::table
                .select(model::Meta::as_select())
                .find(package.to_string())
                .first::<model::Meta>(conn)?;
            let licenses = model::License::belonging_to(&meta)
                .select(model::meta_licenses::license)
                .load::<String>(conn)?;
            let dependencies = model::Dependency::belonging_to(&meta)
                .select(model::Dependency::as_select())
                .load_iter(conn)?
                .map(|d| Ok(d?.dependency))
                .collect::<Result<_, Error>>()?;
            let providers = model::Provider::belonging_to(&meta)
                .select(model::Provider::as_select())
                .load_iter(conn)?
                .map(|p| Ok(p?.provider))
                .collect::<Result<_, Error>>()?;
            let conflicts = model::Conflict::belonging_to(&meta)
                .select(model::Conflict::as_select())
                .load_iter(conn)?
                .map(|p| Ok(p?.conflict))
                .collect::<Result<_, Error>>()?;

            Ok(Meta {
                name: meta.name,
                version_identifier: meta.version_identifier,
                source_release: meta.source_release as u64,
                build_release: meta.build_release as u64,
                architecture: meta.architecture,
                summary: meta.summary,
                description: meta.description,
                source_id: meta.source_id,
                homepage: meta.homepage,
                licenses,
                dependencies,
                providers,
                conflicts,
                uri: meta.uri,
                hash: meta.hash,
                download_size: meta.download_size.map(|size| size as u64),
            })
        })
    }

    pub fn provider_packages(&self, provider: &Provider) -> Result<Vec<package::Id>, Error> {
        self.conn.exec(|conn| {
            model::meta_providers::table
                .select(model::meta_providers::package)
                .distinct()
                .filter(model::meta_providers::provider.eq(provider.to_string()))
                .load_iter::<AStr, _>(conn)?
                .map(|result| {
                    let id = result?;
                    Ok(id.into())
                })
                .collect()
        })
    }

    pub fn query(&self, filter: Option<Filter<'_>>) -> Result<Vec<(package::Id, Meta)>, Error> {
        self.conn.exec(|conn| {
            let map_row = |result| {
                let meta: model::Meta = result?;

                Ok((
                    package::Id::from(AStr::from(meta.package)),
                    Meta {
                        name: meta.name,
                        version_identifier: meta.version_identifier,
                        source_release: meta.source_release as u64,
                        build_release: meta.build_release as u64,
                        architecture: meta.architecture,
                        summary: meta.summary,
                        description: meta.description,
                        source_id: meta.source_id,
                        homepage: meta.homepage,
                        licenses: Default::default(),
                        dependencies: Default::default(),
                        providers: Default::default(),
                        conflicts: Default::default(),
                        uri: meta.uri,
                        hash: meta.hash,
                        download_size: meta.download_size.map(|size| size as u64),
                    },
                ))
            };

            let mut entries: BTreeMap<package::Id, Meta> = match &filter {
                Some(Filter::Provider(provider)) => model::meta::table
                    .select(model::Meta::as_select())
                    .inner_join(model::meta_providers::table)
                    .filter(model::meta_providers::provider.eq(provider.to_string()))
                    .load_iter::<model::Meta, _>(conn)?,
                Some(Filter::Dependency(dependency)) => model::meta::table
                    .select(model::Meta::as_select())
                    .inner_join(model::meta_dependencies::table)
                    .filter(model::meta_dependencies::dependency.eq(dependency.to_string()))
                    .load_iter::<model::Meta, _>(conn)?,
                Some(Filter::Name(name)) => model::meta::table
                    .select(model::Meta::as_select())
                    .filter(model::meta::name.eq(name.to_string()))
                    .load_iter::<model::Meta, _>(conn)?,
                Some(Filter::Keyword(keyword)) => {
                    let pattern = format!("%{keyword}%");
                    model::meta::table
                        .select(model::Meta::as_select())
                        .filter(
                            model::meta::name
                                .like(pattern.clone())
                                .or(model::meta::summary.like(pattern)),
                        )
                        .load_iter::<model::Meta, _>(conn)?
                }
                None => model::meta::table
                    .select(model::Meta::as_select())
                    .load_iter::<model::Meta, _>(conn)?,
            }
            .map(map_row)
            .collect::<Result<_, Error>>()?;

            let package_ids = entries
                .keys()
                .map(|id| model::PackageId { id: id.to_string() })
                .collect::<Vec<_>>();

            for chunk in package_ids.chunks(MAX_VARIABLE_NUMBER) {
                // Add licenses
                model::License::belonging_to(chunk)
                    .load_iter::<model::License, _>(conn)?
                    .try_for_each::<_, Result<_, Error>>(|result| {
                        let row = result?;
                        if let Some(meta) = entries.get_mut(row.package.as_str()) {
                            meta.licenses.push(row.license);
                        }
                        Ok(())
                    })?;

                // Add dependencies
                model::Dependency::belonging_to(chunk)
                    .load_iter::<model::Dependency, _>(conn)?
                    .try_for_each::<_, Result<_, Error>>(|result| {
                        let row = result?;
                        if let Some(meta) = entries.get_mut(row.package.as_str()) {
                            meta.dependencies.insert(row.dependency);
                        }
                        Ok(())
                    })?;

                // Add providers
                model::Provider::belonging_to(chunk)
                    .load_iter::<model::Provider, _>(conn)?
                    .try_for_each::<_, Result<_, Error>>(|result| {
                        let row = result?;
                        if let Some(meta) = entries.get_mut(row.package.as_str()) {
                            meta.providers.insert(row.provider);
                        }
                        Ok(())
                    })?;

                // Add conflicts
                model::Conflict::belonging_to(chunk)
                    .load_iter::<model::Conflict, _>(conn)?
                    .try_for_each::<_, Result<_, Error>>(|result| {
                        let row = result?;
                        if let Some(meta) = entries.get_mut(row.package.as_str()) {
                            meta.conflicts.insert(row.conflict);
                        }
                        Ok(())
                    })?;
            }

            Ok(entries.into_iter().collect())
        })
    }

    pub fn package_ids(&self) -> Result<BTreeSet<package::Id>, Error> {
        self.conn.exec(|conn| {
            Ok(model::meta::table
                .select(model::meta::package)
                .distinct()
                .load_iter::<AStr, _>(conn)?
                .map(|result| result.map(package::Id::from))
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn file_hashes(&self) -> Result<BTreeSet<String>, Error> {
        self.conn.exec(|conn| {
            Ok(model::meta::table
                .select(model::meta::hash.assume_not_null())
                .filter(model::meta::hash.is_not_null())
                .distinct()
                .load_iter::<String, _>(conn)?
                .collect::<Result<_, _>>()?)
        })
    }

    pub fn add(&self, id: package::Id, meta: Meta) -> Result<(), Error> {
        self.batch_add(vec![(id, meta)])
    }

    pub fn batch_add(&self, packages: Vec<(package::Id, Meta)>) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let ids = packages.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
            let entries = packages
                .iter()
                .map(|(package, meta)| model::NewMeta {
                    package: package.as_str(),
                    name: meta.name.as_str(),
                    version_identifier: &meta.version_identifier,
                    source_release: meta.source_release as i32,
                    build_release: meta.build_release as i32,
                    architecture: &meta.architecture,
                    summary: &meta.summary,
                    description: &meta.description,
                    source_id: &meta.source_id,
                    homepage: &meta.homepage,
                    uri: meta.uri.as_deref(),
                    hash: meta.hash.as_deref(),
                    download_size: meta.download_size.map(|size| size as i64),
                })
                .collect::<Vec<_>>();
            let licenses = packages
                .iter()
                .flat_map(|(package, meta)| {
                    meta.licenses.iter().map(|license| {
                        (
                            model::meta_licenses::package.eq(package.as_str()),
                            model::meta_licenses::license.eq(license),
                        )
                    })
                })
                .collect::<Vec<_>>();
            let dependencies = packages
                .iter()
                .flat_map(|(package, meta)| {
                    meta.dependencies.iter().map(|dependency| {
                        (
                            model::meta_dependencies::package.eq(package.as_str()),
                            model::meta_dependencies::dependency.eq(dependency.to_string()),
                        )
                    })
                })
                .collect::<Vec<_>>();
            let providers = packages
                .iter()
                .flat_map(|(package, meta)| {
                    meta.providers.iter().map(|provider| {
                        (
                            model::meta_providers::package.eq(package.as_str()),
                            model::meta_providers::provider.eq(provider.to_string()),
                        )
                    })
                })
                .collect::<Vec<_>>();
            let conflicts = packages
                .iter()
                .flat_map(|(package, meta)| {
                    meta.conflicts.iter().map(|conflict| {
                        (
                            model::meta_conflicts::package.eq(package.as_str()),
                            model::meta_conflicts::conflict.eq(conflict.to_string()),
                        )
                    })
                })
                .collect::<Vec<_>>();

            batch_remove_impl(&ids, tx)?;

            for chunk in entries.chunks(MAX_VARIABLE_NUMBER / 13) {
                diesel::insert_into(model::meta::table).values(chunk).execute(tx)?;
            }
            for chunk in licenses.chunks(MAX_VARIABLE_NUMBER / 2) {
                diesel::insert_or_ignore_into(model::meta_licenses::table)
                    .values(chunk)
                    .execute(tx)?;
            }
            for chunk in dependencies.chunks(MAX_VARIABLE_NUMBER / 2) {
                diesel::insert_or_ignore_into(model::meta_dependencies::table)
                    .values(chunk)
                    .execute(tx)?;
            }
            for chunk in providers.chunks(MAX_VARIABLE_NUMBER / 2) {
                diesel::insert_or_ignore_into(model::meta_providers::table)
                    .values(chunk)
                    .execute(tx)?;
            }
            for chunk in conflicts.chunks(MAX_VARIABLE_NUMBER / 2) {
                diesel::insert_or_ignore_into(model::meta_conflicts::table)
                    .values(chunk)
                    .execute(tx)?;
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
        diesel::delete(model::meta::table.filter(model::meta::package.eq_any(chunk))).execute(tx)?;
    }
    Ok(())
}

mod model {
    use diesel::{
        Selectable,
        associations::{Associations, Identifiable},
        deserialize::Queryable,
        prelude::Insertable,
    };

    pub use crate::db::meta::schema::{meta, meta_conflicts, meta_dependencies, meta_licenses, meta_providers};
    use crate::package;

    #[derive(Queryable, Selectable, Identifiable)]
    #[diesel(table_name = meta)]
    #[diesel(primary_key(package))]
    pub struct Meta {
        pub package: String,
        #[diesel(deserialize_as = String)]
        pub name: package::Name,
        pub version_identifier: String,
        pub source_release: i32,
        pub build_release: i32,
        pub architecture: String,
        pub summary: String,
        pub description: String,
        pub source_id: String,
        pub homepage: String,
        pub uri: Option<String>,
        pub hash: Option<String>,
        pub download_size: Option<i64>,
    }

    #[derive(Queryable, Selectable, Identifiable)]
    #[diesel(table_name = meta)]
    #[diesel(primary_key(package))]
    pub struct PackageId {
        #[diesel(column_name = "package")]
        pub id: String,
    }

    #[derive(Queryable, Selectable, Identifiable, Associations)]
    #[diesel(table_name = meta_licenses)]
    #[diesel(primary_key(package, license))]
    #[diesel(belongs_to(Meta, foreign_key = package))]
    #[diesel(belongs_to(PackageId, foreign_key = package))]
    pub struct License {
        pub package: String,
        pub license: String,
    }

    #[derive(Queryable, Selectable, Identifiable, Associations)]
    #[diesel(table_name = meta_dependencies)]
    #[diesel(primary_key(package, dependency))]
    #[diesel(belongs_to(Meta, foreign_key = package))]
    #[diesel(belongs_to(PackageId, foreign_key = package))]
    pub struct Dependency {
        pub package: String,
        #[diesel(deserialize_as = String)]
        pub dependency: crate::Dependency,
    }

    #[derive(Queryable, Selectable, Identifiable, Associations)]
    #[diesel(table_name = meta_providers)]
    #[diesel(primary_key(package, provider))]
    #[diesel(belongs_to(Meta, foreign_key = package))]
    #[diesel(belongs_to(PackageId, foreign_key = package))]
    pub struct Provider {
        pub package: String,
        #[diesel(deserialize_as = String)]
        pub provider: crate::Provider,
    }

    #[derive(Queryable, Selectable, Identifiable, Associations)]
    #[diesel(table_name = meta_conflicts)]
    #[diesel(primary_key(package, conflict))]
    #[diesel(belongs_to(Meta, foreign_key = package))]
    #[diesel(belongs_to(PackageId, foreign_key = package))]
    pub struct Conflict {
        pub package: String,
        #[diesel(deserialize_as = String)]
        pub conflict: crate::Provider,
    }

    #[derive(Insertable)]
    #[diesel(table_name = meta)]
    pub struct NewMeta<'a> {
        pub package: &'a str,
        pub name: &'a str,
        pub version_identifier: &'a str,
        pub source_release: i32,
        pub build_release: i32,
        pub architecture: &'a str,
        pub summary: &'a str,
        pub description: &'a str,
        pub source_id: &'a str,
        pub homepage: &'a str,
        pub uri: Option<&'a str>,
        pub hash: Option<&'a str>,
        pub download_size: Option<i64>,
    }
}

#[cfg(test)]
mod test {
    use stone::StoneDecodedPayload;

    use crate::dependency::Kind;

    use super::*;

    #[test]
    fn create_insert_select() {
        let db = Database::new(":memory:").unwrap();

        let bash_completion = include_bytes!("../../../../test/bash-completion-2.11-1-1-x86_64.stone");

        let mut stone = stone::read_bytes(bash_completion).unwrap();

        let payloads = stone.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let meta_payload = payloads.iter().find_map(StoneDecodedPayload::meta).unwrap();
        let meta = Meta::from_stone_payload(&meta_payload.body).unwrap();

        let id = package::Id::from("test");

        db.add(id.clone(), meta.clone()).unwrap();

        assert_eq!(&meta.name, &"bash-completion".to_owned().into());

        // Now retrieve by provider.
        let lookup = Filter::Provider(Provider {
            kind: Kind::PackageName,
            name: "bash-completion".to_owned(),
        });
        let fetched = db.query(Some(lookup)).unwrap();
        assert_eq!(fetched.len(), 1);

        db.remove(&id).unwrap();

        let result = db.get(&id);

        assert!(result.is_err());

        // Test wipe
        db.add(id.clone(), meta).unwrap();
        db.wipe().unwrap();
        let result = db.get(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_conflict_is_recognized() {
        let db = Database::new(":memory:").unwrap();

        // See `test/conflicts/italian-pizza.yml` for the recipe file that produced this stone.
        // It should be obvious that this package conflicts with `name(pineapple)`.
        let italian_pizza = include_bytes!("../../../../test/conflicts/italian-pizza-1-1-1-x86_64.stone");
        let pineapple_provider = Provider {
            kind: Kind::PackageName,
            name: "pineapple".to_owned(),
        };

        let mut stone = stone::read_bytes(italian_pizza).unwrap();

        let payloads = stone.payloads().unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let meta_payload = payloads.iter().find_map(StoneDecodedPayload::meta).unwrap();
        let meta = Meta::from_stone_payload(&meta_payload.body).unwrap();
        db.add(package::Id::from(meta.id()), meta.clone()).unwrap();

        // Ensure we're parsing the correct package!
        assert_eq!(&meta.name, &"italian-pizza".to_owned().into());
        // Ensure that the conflict info already exists in the binary package.
        assert_eq!(
            meta.conflicts.iter().collect::<Vec<&Provider>>(),
            vec![&pineapple_provider]
        );

        // Now retrieve by provider.
        let lookup = Filter::Provider(Provider {
            kind: Kind::PackageName,
            name: "italian-pizza".to_owned(),
        });
        let fetched = db.query(Some(lookup)).unwrap();
        assert_eq!(fetched.len(), 1);

        let (_, retrieved_pkg) = fetched.first().unwrap();
        let retrieved_conflicts: Vec<&Provider> = retrieved_pkg.conflicts.iter().collect();
        // Ensure that the conflicts field is inserted into and can be queried from our database
        // correctly.
        assert_eq!(retrieved_conflicts, vec![&pineapple_provider]);
    }
}
