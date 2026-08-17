//! Service layer: external integrations, background sync jobs, and network
//! connectivity tracking.
//!
//! Services are the only place that talks to the outside world. Two upstreams
//! are in play, and they answer different questions:
//!
//! - **FIRST Events API** ([`first_api`]) is authoritative for what exists —
//!   events, team rosters and the match schedule. [`first_sync`] pulls it into
//!   `events`, `teams` and `event_teams`.
//! - **The Blue Alliance** ([`tba`]) is authoritative for what happened —
//!   rankings, OPR/DPR and match results. [`tba_stats_sync`] writes those into
//!   `team_event_stats` and `matches`, driven by the [`stats_syncer`]
//!   background loop.
//!
//! # Offline behaviour
//!
//! The app is built for a competition LAN that may have no uplink, so every
//! outbound call goes through [`connectivity`]: a fast TCP preflight, bounded
//! retries with backoff, and a record of the outcome. Callers can then tell
//! "the internet is down" from "the API said no"
//! ([`connectivity::is_internet_unavailable`]) and say so in the UI instead of
//! failing blankly. Both clients are no-ops without credentials, in which case
//! the app runs on whatever is already in the database.

pub mod connectivity;
pub mod first_api;
pub mod first_sync;
pub mod stats_syncer;
pub mod tba;
pub mod tba_stats_sync;
