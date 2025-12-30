use diesel::{
    deserialize::{FromSql, Result},
    sql_types::Text,
    sqlite::Sqlite,
};

use crate::AStr;

impl FromSql<Text, Sqlite> for AStr {
    fn from_sql(bytes: <Sqlite as diesel::backend::Backend>::RawValue<'_>) -> Result<Self> {
        let ptr = <*const str as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        // SAFETY: from_sql per its docs provides a reference that borrows from
        // bytes, only converted to a pointer because the trait does not allow
        // this borrowing relationship to be expressed.
        Ok(Self::from(unsafe { &*ptr }))
    }
}
