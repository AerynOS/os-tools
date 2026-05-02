// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::{Connection as _, SqliteConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use itertools::Itertools;

use super::{Connection, Error, MAX_VARIABLE_NUMBER};
use crate::State;
use crate::state::{self, Id, Selection};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("src/db/state/migrations");

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

    pub fn list_ids(&self) -> Result<Vec<(Id, DateTime<Utc>)>, Error> {
        self.conn.exec(|conn| {
            model::state::table
                .select(model::Created::as_select())
                .load_iter(conn)?
                .map(|result| {
                    let row = result?;
                    Ok((row.id.into(), row.created.0))
                })
                .collect()
        })
    }

    pub fn all(&self) -> Result<Vec<State>, Error> {
        self.conn.exec(|conn| {
            let states = model::state::table
                .select(model::State::as_select())
                .load::<model::State>(conn)?;
            let mut selections = model::state_selections::table
                .select(model::Selection::as_select())
                .load::<model::Selection>(conn)?
                .into_iter()
                .map(|row| {
                    (
                        Id::from(row.state_id),
                        Selection {
                            package: row.package_id,
                            explicit: row.explicit,
                            reason: row.reason,
                        },
                    )
                })
                .into_group_map();

            Ok(states
                .into_iter()
                .map(|state| {
                    let id = state.id.into();
                    let selections = selections.remove(&id).unwrap_or_default();
                    State {
                        id,
                        summary: state.summary,
                        description: state.description,
                        selections,
                        created: state.created.0,
                        kind: state.kind,
                    }
                })
                .collect())
        })
    }

    pub fn get(&self, id: Id) -> Result<State, Error> {
        self.conn.exec(|conn| {
            let state = model::state::table
                .select(model::State::as_select())
                .find(i32::from(id))
                .first(conn)?;
            let selections = model::Selection::belonging_to(&state)
                .select(model::Selection::as_select())
                .load_iter(conn)?
                .map(|result| {
                    let row = result?;
                    Ok(Selection {
                        package: row.package_id,
                        explicit: row.explicit,
                        reason: row.reason,
                    })
                })
                .collect::<Result<_, Error>>()?;

            Ok(State {
                id: state.id.into(),
                summary: state.summary,
                description: state.description,
                selections,
                created: state.created.0,
                kind: state.kind,
            })
        })
    }

    pub fn add(
        &self,
        selections: &[Selection],
        summary: Option<&str>,
        description: Option<&str>,
    ) -> Result<State, Error> {
        self.conn
            .exclusive_tx(|tx| {
                let state = model::NewState {
                    summary,
                    description,
                    kind: state::Kind::Transaction.to_string(),
                };

                let id = diesel::insert_into(model::state::table)
                    .values(state)
                    .returning(model::state::id)
                    .get_result::<i32>(tx)?;

                let selections = selections
                    .iter()
                    .map(|selection| model::NewSelection {
                        state_id: id,
                        package_id: selection.package.as_str(),
                        explicit: selection.explicit,
                        reason: selection.reason.as_deref(),
                    })
                    .collect::<Vec<_>>();

                for chunk in selections.chunks(MAX_VARIABLE_NUMBER / 4) {
                    diesel::insert_into(model::state_selections::table)
                        .values(chunk)
                        .execute(tx)?;
                }

                Ok(id.into())
            })
            .and_then(|id| self.get(id))
    }

    pub fn remove(&self, state: &Id) -> Result<(), Error> {
        self.batch_remove(Some(state))
    }

    pub fn batch_remove<'a>(&self, states: impl IntoIterator<Item = &'a Id>) -> Result<(), Error> {
        self.conn.exclusive_tx(|tx| {
            let states = states.into_iter().map(|id| i32::from(*id)).collect::<Vec<_>>();

            for chunk in states.chunks(MAX_VARIABLE_NUMBER) {
                // Cascading wipes other tables
                diesel::delete(model::state::table.filter(model::state::id.eq_any(chunk))).execute(tx)?;
            }

            Ok(())
        })
    }
}

mod model {
    use astr::AStr;
    use diesel::{
        Selectable,
        associations::{Associations, Identifiable},
        deserialize::Queryable,
        prelude::Insertable,
        sqlite::Sqlite,
    };

    use crate::{db::Timestamp, package, state::Kind};

    pub use super::schema::{state, state_selections};

    #[derive(Queryable, Selectable, Identifiable)]
    #[diesel(table_name = state)]
    #[diesel(check_for_backend(Sqlite))]
    pub struct State {
        pub id: i32,
        #[diesel(deserialize_as = i64)]
        pub created: Timestamp,
        pub summary: Option<String>,
        pub description: Option<String>,
        #[diesel(column_name = "type_", deserialize_as = String)]
        pub kind: Kind,
    }

    #[derive(Queryable, Selectable, Identifiable, Associations)]
    #[diesel(table_name = state_selections)]
    #[diesel(primary_key(state_id, package_id))]
    #[diesel(belongs_to(State))]
    pub struct Selection {
        pub state_id: i32,
        #[diesel(deserialize_as = AStr)]
        pub package_id: package::Id,
        pub explicit: bool,
        pub reason: Option<String>,
    }

    #[derive(Queryable, Selectable, Identifiable)]
    #[diesel(table_name = state)]
    #[diesel(check_for_backend(Sqlite))]
    pub struct Created {
        pub id: i32,
        #[diesel(deserialize_as = i64)]
        pub created: Timestamp,
    }

    #[derive(Insertable)]
    #[diesel(table_name = state)]
    pub struct NewState<'a> {
        pub summary: Option<&'a str>,
        pub description: Option<&'a str>,
        #[diesel(column_name = "type_")]
        pub kind: String,
    }

    #[derive(Insertable)]
    #[diesel(table_name = state_selections)]
    pub struct NewSelection<'a> {
        pub state_id: i32,
        pub package_id: &'a str,
        pub explicit: bool,
        pub reason: Option<&'a str>,
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashSet, iter};

    use super::*;
    use crate::package;

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
        itertools::assert_equal(states.into_iter().map(|s| Into::<TestState>::into(s)), all_entries());
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
            all_entries.into_iter().map(|s| Into::<TestState>::into(s)),
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

        itertools::assert_equal(
            all_entries.into_iter().map(|s| Into::<TestState>::into(s)),
            expected_states(),
        );
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
            db.all()?.into_iter().map(|s| Into::<TestState>::into(s)),
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
            db.all()?.into_iter().map(|s| Into::<TestState>::into(s)),
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
            assert!(created >= now_ - grace_time, "Created time {created} is too old")
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
                    let pkg_id = format!("pkg_{}_{}", index, j);
                    Selection {
                        package: package::Id::from(pkg_id),
                        explicit: j % 2 == 0,
                        reason: (j % 3 == 0).then_some(format!("Reason {index}")),
                    }
                })
                .collect()
        };

        (0..NUM_STATES).map(move |i| TestState {
            summary: (i % 2 == 0).then_some(format!("Summary {}", i)),
            description: (i % 3 == 0).then_some(format!("Description {}", i)),
            selections: selections_from_index(i),
            kind: state::Kind::Transaction,
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
        kind: state::Kind,
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
