//! Pure TealTeam domain logic.
//!
//! Everything in this crate is a value transformation: no database, no HTTP, no
//! filesystem, no wall clock. Functions that need the current time take it as a
//! parameter. That is what lets the crate compile to `wasm32-unknown-unknown`
//! and run browser-side, and it is also what makes it testable.
//!
//! The CI job in `.github/workflows/ci.yml` builds this crate for wasm32 on
//! every push. If that job fails, a server-only dependency has leaked in and
//! the code belongs in an adapter crate instead.

pub mod connectivity;
pub mod error;
pub mod matches;
pub mod records;
pub mod season;
pub mod upstream;
pub mod user;

pub use error::{DomainError, Result};
