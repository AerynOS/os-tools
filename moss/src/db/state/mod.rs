// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::iter;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use indoc::indoc;
use itertools::Itertools;
use rusqlite::{Row, limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER, types::ToSqlOutput, types::Value, types::ValueRef};

use crate::db::Connection;
use crate::db::Error;
use crate::db::migrations::Migrations;
use crate::state::{Id, Kind, Selection, State};

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
        conn.pragma_update(None, "foreign_keys", "ON")?;
        rusqlite::vtab::array::load_module(&conn)?;
        MIGRATIONS.migrate(&mut conn, MIGRATIONS.latest())?;
        Ok(Database {
            conn: Connection::new(conn),
        })
    }

    pub fn list_ids(&self) -> Result<Vec<(Id, DateTime<Utc>)>, Error> {
        self.conn.exec(|conn| {
            conn.prepare("SELECT id, created FROM state")?
                .query_and_then([], |row| Ok((From::<i32>::from(row.get(0)?), row.get(1)?)))?
                .collect()
        })
    }

    pub fn all(&self) -> Result<Vec<State>, Error> {
        self.query_states(None)
    }

    pub fn get(&self, id: Id) -> Result<State, Error> {
        let mut states = self.query_states(Some(id))?;
        states.pop().ok_or(Error::from(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn add(
        &self,
        selections: &[Selection],
        summary: Option<&str>,
        description: Option<&str>,
    ) -> Result<State, Error> {
        let id = self.conn.exec_mut(|conn| -> Result<i64, rusqlite::Error> {
            let tx = conn.transaction()?;
            tx.prepare("INSERT INTO state (type, summary, description) VALUES (?, ?, ?)")?
                .execute((Kind::Transaction.to_string(), summary, description))?;
            let id = tx.last_insert_rowid();

            let selections = selections
                .iter()
                .map(|s| {
                    [
                        ToSqlOutput::from(id),
                        ToSqlOutput::from(s.package.to_string()),
                        ToSqlOutput::from(s.explicit),
                        ToSqlOutput::Borrowed(ValueRef::from(s.reason.as_deref())),
                    ]
                })
                .collect::<Vec<_>>();
            let num_placeholders = tx.limit(SQLITE_LIMIT_VARIABLE_NUMBER).unwrap() as usize;
            for chunk in selections.chunks(num_placeholders / 4) {
                let mut selection_stmt = tx.prepare_cached(&format!(
                    "INSERT INTO state_selections (state_id, package_id, explicit, reason) VALUES {}",
                    iter::repeat_n("(?,?,?,?)", chunk.len()).join(",")
                ))?;
                selection_stmt.execute(rusqlite::params_from_iter(chunk.iter().flatten()))?;
            }

            tx.commit()?;
            Ok(id)
        })?;
        self.get(Id::from(id as i32))
    }

    pub fn remove(&self, state: &Id) -> Result<(), Error> {
        self.batch_remove(iter::once(state))
    }

    pub fn batch_remove<'a>(&self, ids: impl IntoIterator<Item = &'a Id>) -> Result<(), Error> {
        self.conn.exec_mut(|conn| {
            let ids: Rc<Vec<Value>> = Rc::new(ids.into_iter().map(|id| Value::from(id.to_string())).collect());
            Ok(conn.execute("DELETE FROM state WHERE id IN rarray(?)", [ids])?).map(|_| ())
        })
    }

    fn query_states(&self, id: Option<Id>) -> Result<Vec<State>, Error> {
        self.conn.exec(|conn| {
            let mut stmt = conn.prepare(indoc! {"
                SELECT
                    s.id, s.summary, s.description, s.created, s.type,
                    json_group_array(
                        json_object(
                            'package_id', ss.package_id,
                            'explicit',   ss.explicit,
                            'reason',     ss.reason
                        )
                    ) FILTER (WHERE ss.package_id IS NOT NULL) AS selections
                FROM state s
                LEFT JOIN state_selections ss ON s.id = ss.state_id
                WHERE s.id = ?1 OR ?1 IS NULL
                GROUP BY s.id
            "})?;
            let rows = stmt
                .query_and_then([id.map(|i| i.to_string())], |row: &Row<'_>| -> Result<State, Error> {
                    row.try_into()
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(rows)
        })
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashSet, iter};

    use itertools::Itertools;

    use super::*;
    use crate::{package, state::Kind};

    #[test]
    fn creates_in_memory_db_connection() -> Result<(), Error> {
        Database::new(":memory:").map(|_| ())
    }

    #[test]
    fn db_lists_ids() -> Result<(), Error> {
        // IDs are assigned by the database, we can't predict them.
        // We can only check that the correct number of IDs is returned
        // and that they are unique.
        let ids = create_db()?.list_ids()?;
        assert_eq!(ids.len(), NUM_STATES as usize);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
        Ok(())
    }

    #[test]
    fn db_returns_all_states() -> Result<(), Error> {
        let states = create_db()?.all()?;
        itertools::assert_equal(states.into_iter().map(Into::<TestState>::into), all_entries());
        Ok(())
    }

    #[test]
    fn db_gets_single_state() -> Result<(), Error> {
        let (id, _) = create_db()?
            .list_ids()?
            .into_iter()
            .sorted_by_key(|(id1, _)| *id1)
            .nth(NUM_STATES as usize / 2)
            .unwrap();
        let state = create_db()?.get(id)?;
        let expected_state = all_entries().nth(NUM_STATES as usize / 2).unwrap();

        assert_eq!(Into::<TestState>::into(state), expected_state);
        Ok(())
    }

    #[test]
    fn db_adds_one_state() -> Result<(), Error> {
        let expected_state = all_entries().nth(NUM_STATES as usize / 3).unwrap();

        let db = Database::new(":memory:")?;
        db.add(
            &expected_state.selections,
            expected_state.summary.as_deref(),
            expected_state.description.as_deref(),
        )?;
        let all_entries = db.all()?;

        itertools::assert_equal(
            all_entries.into_iter().map(Into::<TestState>::into),
            iter::once(expected_state),
        );
        Ok(())
    }

    #[test]
    fn db_adds_multiple_states() -> Result<(), Error> {
        let expected_states = || all_entries().take(10);

        let db = Database::new(":memory:")?;
        for entry in expected_states() {
            db.add(
                &entry.selections,
                entry.summary.as_deref(),
                entry.description.as_deref(),
            )?;
        }
        let mut all_entries = db.all()?;
        all_entries.sort_by_key(|s| s.id);

        itertools::assert_equal(all_entries.into_iter().map(Into::<TestState>::into), expected_states());
        Ok(())
    }

    #[test]
    fn db_removes_one_state() -> Result<(), Error> {
        const SAMPLE_SIZE: usize = 10;
        let expected_states = || all_entries().take(SAMPLE_SIZE);

        let db = Database::new(":memory:")?;
        for entry in expected_states() {
            db.add(
                &entry.selections,
                entry.summary.as_deref(),
                entry.description.as_deref(),
            )?;
        }
        let mut ids = db.list_ids()?;
        ids.sort_by_key(|id| *id);
        db.remove(&ids[0].0)?;

        itertools::assert_equal(
            db.all()?.into_iter().map(Into::<TestState>::into),
            expected_states().skip(1),
        );
        Ok(())
    }

    #[test]
    fn db_batch_removes_states() -> Result<(), Error> {
        const SAMPLE_SIZE: usize = 10;
        const DELETE_COUNT: usize = SAMPLE_SIZE / 2;
        let expected_states = || all_entries().take(SAMPLE_SIZE);

        let db = Database::new(":memory:")?;
        for entry in expected_states() {
            db.add(
                &entry.selections,
                entry.summary.as_deref(),
                entry.description.as_deref(),
            )?;
        }
        let mut ids = db.list_ids()?;
        ids.sort_by_key(|id| *id);
        db.batch_remove(ids.iter().take(DELETE_COUNT).map(|(id, _)| id))?;

        itertools::assert_equal(
            db.all()?.into_iter().map(Into::<TestState>::into),
            expected_states().skip(DELETE_COUNT),
        );
        Ok(())
    }

    #[test]
    fn db_created_timestamps_are_recent() -> Result<(), Error> {
        let entries = create_db()?.all()?;
        let created_times = entries.into_iter().map(|s| s.created);

        let now_ = Utc::now();
        // Unfortunately we have to gamble on the created time being
        // within 1 hour of the test running, since we can't predict it.
        let grace_time = chrono::Duration::hours(1);
        for created in created_times {
            assert!(created <= now_, "Created time {created} is in the future");
            assert!(created >= now_ - grace_time, "Created time {created} is too old");
        }
        Ok(())
    }

    fn create_db() -> Result<Database, Error> {
        let db = Database::new(":memory:").unwrap();
        for state in all_entries() {
            db.add(
                &state.selections,
                state.summary.as_deref(),
                state.description.as_deref(),
            )
            .unwrap();
        }
        Ok(db)
    }

    const NUM_STATES: u32 = 128;

    fn all_entries() -> impl Iterator<Item = TestState> {
        let selections_from_index = |index: u32| -> Vec<Selection> {
            let count = (index % 4) as usize;
            (0..count)
                .map(|j| {
                    let pkg_id = format!("pkg_{index}_{j}");
                    Selection {
                        package: package::Id::from(pkg_id),
                        explicit: j % 2 == 0,
                        reason: (j % 3 == 0).then_some(format!("Reason {index}")),
                    }
                })
                .collect()
        };

        (0..NUM_STATES).map(move |i| TestState {
            summary: (i % 2 == 0).then_some(format!("Summary {i}")),
            description: (i % 3 == 0).then_some(format!("Description {i}")),
            selections: selections_from_index(i),
            kind: Kind::Transaction,
        })
    }

    /// Helper struct to compare expected states
    /// without relying on database-assigned fields, since
    /// these are not predictable.
    #[derive(Debug, PartialEq, Eq)]
    struct TestState {
        // `id` is assigned by the database.
        summary: Option<String>,
        description: Option<String>,
        selections: Vec<Selection>,
        // `created` is assigned by the database.
        kind: Kind,
    }

    impl PartialEq<State> for TestState {
        fn eq(&self, other: &State) -> bool {
            self.summary == other.summary
                && self.description == other.description
                && self.selections == other.selections
                && self.kind == other.kind
        }
    }

    impl From<State> for TestState {
        fn from(test_state: State) -> Self {
            Self {
                summary: test_state.summary,
                description: test_state.description,
                selections: test_state.selections,
                kind: test_state.kind,
            }
        }
    }
}
