//! TealTeam server binary.
//!
//! The only crate that knows about HTTP. Everything it serves comes from
//! `tt-core` (domain), `tt-templates` (rendering), and a `Repo` implementation
//! (storage) -- so that when handlers move into a service worker later, the
//! pieces they depend on come along and this crate stays behind.
//!
//! Startup (F5) is deliberately fault-tolerant. See [`run`].

mod auth;
mod config;
mod handlers;
mod startup;

use tracing::error;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match startup::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // Errors reaching here are configuration or bind failures -- things no
            // amount of degrading can work around.
            error!("fatal: {e:#}");
            eprintln!("fatal: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
