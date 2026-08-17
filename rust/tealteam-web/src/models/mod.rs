//! Model layer: entity structs mapped by `sqlx::FromRow`, plus the queries
//! that read and write them.
//!
//! **Every SQL statement in the app lives under this module.** Controllers
//! call these functions; they never build SQL themselves. Views may read model
//! types but never reach the database.
//!
//! # Conventions
//!
//! - Field names match the snake_case column names, so `SELECT *` decodes
//!   directly (a port of `Models/Entities.cs`).
//! - Timestamp columns declared without `NOT NULL` are `Option<_>` so runtime
//!   decoding never panics on legacy rows.
//! - `NUMERIC` columns must be selected with `::float8` casts — see
//!   [`stats::TeamEventStats::SELECT`].
//! - Queries are written with the runtime `query`/`query_as` API rather than
//!   the compile-time macros: the schema is owned by the shared `migrations/`
//!   directory and created at boot, so there is no compile-time database to
//!   check against, and hand-written SQL stays identical to the Go/C# ports.
//!
//! # Error handling
//!
//! Read paths that a page can render without generally return the value
//! directly and fall back to an empty result, so a database hiccup degrades
//! one panel instead of failing the whole request. Write paths return
//! `Result`, because the controller has to tell the user whether their change
//! was saved.
//!
//! # Layout
//!
//! | Module | Tables |
//! |---|---|
//! | [`user`], [`session`] | `users`, `sessions` — accounts, bcrypt, cookies |
//! | [`event`], [`team`] | `events`, `teams`, `event_teams` |
//! | [`stats`] | `team_event_stats` synced from TBA/FIRST |
//! | [`scouting`], [`scouting_points`] | `scouting_submissions`, `scouting_data`, `scouting_point_weights` |
//! | [`assignment`], [`device`] | `matches`, `scout_assignments`, `devices` |
//! | [`pick_list`] | `pick_list_entries` |
//! | [`schema`] | `information_schema` introspection for the DB viewer |

pub mod assignment;
pub mod device;
pub mod event;
pub mod pick_list;
pub mod schema;
pub mod scouting;
pub mod scouting_points;
pub mod session;
pub mod stats;
pub mod team;
pub mod user;

pub use scouting::ScoutingData;
pub use session::Session;
pub use stats::TeamEventStats;
pub use team::Team;
pub use user::User;
