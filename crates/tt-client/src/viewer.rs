//! Who scouts on this device (OF1): the claims of the last offline token
//! (C9) a page handed the worker.
//!
//! A page made here cannot ask the server who is signed in. The scouting
//! page needs to know: whose assignments to lead with, which robots they have
//! recorded, and who an offline save is by. A signed-in page hands the worker
//! its token (`sync.js`), and the claims are kept here, in this device's own
//! table, read without the key: the device trusts its own storage for what
//! to show, and the Pi checks every write when the outbox is pushed.
//!
//! The table is this device's own, like the outbox: a snapshot never carries
//! one, and replacing the file loses it until the next page hands it again.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use tt_core::token::{Claims, read_unverified, split_credential};
use tt_core::user::User;
use tt_repo::Result;

use crate::ClientRepo;
use crate::sql::{Context, to_sql};

const VIEWER: &str = "CREATE TABLE IF NOT EXISTS device_viewer (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    -- tt_core::token::Claims, as JSON.
    claims     TEXT    NOT NULL,
    updated_at TEXT    NOT NULL
) STRICT";

/// The claims in a page's credential (`token~key`, or a bare token), if
/// they read.
pub fn claims_of(credential: &str) -> Option<Claims> {
    let token = split_credential(credential).map_or(credential, |(token, _)| token);
    read_unverified(token).ok()
}

/// The scout the claims name, as the pages take one.
pub fn user_of(claims: &Claims) -> User {
    User {
        id: claims.user_id,
        email: String::new(),
        name: claims.name.clone(),
        team_number: claims.team,
        roles: claims.roles,
    }
}

impl ClientRepo {
    fn ensure_viewer(&self) -> Result<()> {
        self.conn
            .execute_batch(VIEWER)
            .ctx("making the device's viewer")
    }

    /// Keep `claims` as who scouts here. Written only when they changed, so
    /// every page load does not rewrite the file.
    pub async fn remember_viewer(&self, claims: &Claims, now: DateTime<Utc>) -> Result<()> {
        self.ensure_viewer()?;
        let json = serde_json::to_string(claims).unwrap_or_default();
        self.conn
            .execute(
                "INSERT INTO device_viewer (id, claims, updated_at) VALUES (1, ?, ?) \
                 ON CONFLICT (id) DO UPDATE SET claims = excluded.claims, \
                 updated_at = excluded.updated_at WHERE claims <> excluded.claims",
                params![json, to_sql(now)],
            )
            .ctx("keeping who scouts here")?;
        self.flush().await
    }

    /// Who scouts here, as the last page said, or `None` before any did.
    pub fn viewer(&self) -> Result<Option<Claims>> {
        self.ensure_viewer()?;
        let json: Option<String> = self
            .conn
            .query_row("SELECT claims FROM device_viewer WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .ctx("reading who scouts here")?;
        Ok(json.and_then(|json| serde_json::from_str(&json).ok()))
    }
}
