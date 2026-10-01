//! What every module shares: timestamps as the server stores them, and
//! rusqlite's errors as [`RepoError`]s.

use chrono::{DateTime, Utc};
use rusqlite::ErrorCode;
use rusqlite::ffi;
use tt_repo::{RepoError, Result};

/// The server's format exactly (`tt_repo_sqlite::users::to_sql`): ISO-8601
/// UTC with milliseconds, which sorts as a string. A row this writes and one
/// pulled from the server must compare.
pub(crate) fn to_sql(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn from_sql(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(crate) fn ts_column(raw: Option<String>) -> Option<DateTime<Utc>> {
    raw.as_deref().and_then(from_sql)
}

pub(crate) fn query_err(context: &str, e: rusqlite::Error) -> RepoError {
    // As on the server: a file that will not open is storage gone, which
    // callers treat differently from a query that ran and failed.
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(
                f.code,
                ErrorCode::CannotOpen | ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt
            ) =>
        {
            RepoError::Unavailable(format!("{context}: {e}"))
        }
        _ => RepoError::Query(format!("{context}: {e}")),
    }
}

/// What sqlx calls a unique violation: a UNIQUE index or a primary key.
pub(crate) fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _)
        if f.extended_code == ffi::SQLITE_CONSTRAINT_UNIQUE
            || f.extended_code == ffi::SQLITE_CONSTRAINT_PRIMARYKEY)
}

/// `.ctx("loading user")?` for `.map_err(|e| query_err("loading user", e))?`.
pub(crate) trait Context<T> {
    fn ctx(self, what: &str) -> Result<T>;
}

impl<T> Context<T> for rusqlite::Result<T> {
    fn ctx(self, what: &str) -> Result<T> {
        self.map_err(|e| query_err(what, e))
    }
}
