// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use rusqlite::Error;
use rusqlite::{Connection, Transaction};

/// Schema migration managed based on a list of schema definitions.
/// Each schema corresponds to a specific version.
/// Schema versions are 1-indexed, so the first schema corresponds
/// to version 1, the second to version 2, and so on.
///
/// The migration manager tracks applied migrations in
/// [`user_version` PRAGMA](https://sqlite.org/pragma.html#pragma_user_version).
/// This prevents reapplying the same migration multiple times.
pub struct Migrations {
    schemas: &'static [&'static str],
}

impl Migrations {
    /// Creates a new Migrations instance with the provided schema definitions.
    pub const fn new(schemas: &'static [&'static str]) -> Self {
        Self { schemas }
    }

    /// Returns the latest available version.
    pub const fn latest(&self) -> usize {
        self.schemas.len()
    }

    /// Returns the current schema version applied to the database,
    /// or 0 if no migration has been applied yet.
    pub fn current_version(conn: &Connection) -> Result<usize, Error> {
        let mut stmt = conn.prepare("PRAGMA user_version")?;
        let version: Option<isize> = stmt.query_row([], |row| row.get(0))?;
        Ok(version.unwrap_or(0) as usize)
    }

    /// Applies migrations up to the specified version.
    /// Migrations are only applied if the current version is lower than the requested version.
    /// It panics if the requested version is higher than the latest available version.
    pub fn migrate(&self, conn: &mut Connection, version: usize) -> Result<(), Error> {
        assert!(
            version <= self.latest(),
            "Requested version {version} exceeds latest available version {}",
            self.latest()
        );

        let current = Self::current_version(conn)?;
        if current >= version {
            return Ok(());
        }

        let tx = conn.transaction()?;
        for version in (current + 1)..=version {
            let schema = self.schemas[version - 1]; // Migrations are 1-indexed, indexes are 0-indexed.
            tx.execute_batch(schema)?;
        }
        Self::set_current_version(&tx, version)?;
        tx.commit()?;

        Ok(())
    }

    fn set_current_version(tx: &Transaction<'_>, version: usize) -> Result<(), Error> {
        tx.pragma_update(None, "user_version", version as isize)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    const NUM_SCHEMAS: usize = 3;
    const SCHEMAS: [&str; NUM_SCHEMAS] = [
        "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        "ALTER TABLE users ADD COLUMN email TEXT",
        concat!(
            "CREATE TABLE IF NOT EXISTS posts ",
            "(id INTEGER PRIMARY KEY, user_id INTEGER, content TEXT NOT NULL, ",
            "FOREIGN KEY (user_id) REFERENCES users(id))",
        ),
    ];
    const MIGRATIONS: Migrations = Migrations::new(&SCHEMAS);

    #[test]
    fn returns_latest_version() {
        assert_eq!(MIGRATIONS.latest(), NUM_SCHEMAS);
    }

    #[test]
    fn initial_current_version_is_0() -> Result<(), Error> {
        let conn = Connection::open_in_memory()?;
        assert_eq!(Migrations::current_version(&conn)?, 0);
        Ok(())
    }

    #[test]
    fn migrates() -> Result<(), Error> {
        let mut conn = Connection::open_in_memory()?;
        for version in 1..=MIGRATIONS.latest() {
            assert!(MIGRATIONS.migrate(&mut conn, version).is_ok());
            assert_eq!(Migrations::current_version(&conn)?, version);
        }
        Ok(())
    }
}
