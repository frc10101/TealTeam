//! TealTeam server binary.
//!
//! The only crate that knows about HTTP. Everything it serves comes from
//! `tt-core` (domain), `tt-templates` (rendering), and a `Repo` implementation
//! (storage) -- so that when handlers move into a service worker later, the
//! pieces they depend on come along and this crate stays behind.
//!
//! Startup (F5) is deliberately fault-tolerant. See [`startup::run`].

mod assignments;
mod auth;
mod coach;
mod config;
mod errors;
mod events;
mod handlers;
mod picklist;
mod ranking;
mod review;
mod scouting;
mod standings;
mod startup;
mod teams;
mod upstream;

use std::process::ExitCode;
use tracing::error;

const USAGE: &str = "\
usage: tt-web [command]

commands:
  serve       run the server (the default)
  bulk-load   pull the full FIRST and TBA snapshot into the database, print
              per-event counts, and exit. Run it at the shop before an event.
  help        show this message

Configuration comes from the environment and .env files; see .env.example.";

/// What the binary was asked to do.
///
/// Hand-parsed rather than pulling in an argument parser, for the same reason
/// config.rs hand-rolls `.env`: two words do not justify a dependency in the
/// one binary that has to work on event day.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Serve,
    BulkLoad,
    Help,
}

impl Command {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let command = match args.next().as_deref() {
            None | Some("serve") => Self::Serve,
            Some("bulk-load") => Self::BulkLoad,
            Some("help" | "-h" | "--help") => Self::Help,
            Some(other) => return Err(format!("unknown command {other:?}")),
        };
        match args.next() {
            None => Ok(command),
            Some(extra) => Err(format!("unexpected argument {extra:?}")),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let result = match Command::parse(std::env::args().skip(1)) {
        Ok(Command::Serve) => startup::run().await,
        Ok(Command::BulkLoad) => startup::bulk_load().await,
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Errors reaching here are configuration or bind failures -- things no
            // amount of degrading can work around -- or a bulk load that failed.
            error!("fatal: {e:#}");
            eprintln!("fatal: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        Command::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_serves() {
        // The Pi's autostart runs the bare binary; that must keep meaning "serve".
        assert_eq!(parse(&[]), Ok(Command::Serve));
        assert_eq!(parse(&["serve"]), Ok(Command::Serve));
    }

    #[test]
    fn bulk_load_and_help_are_recognised() {
        assert_eq!(parse(&["bulk-load"]), Ok(Command::BulkLoad));
        for help in ["help", "-h", "--help"] {
            assert_eq!(parse(&[help]), Ok(Command::Help), "{help}");
        }
    }

    #[test]
    fn a_typo_is_an_error_rather_than_a_server() {
        // Starting the server when someone meant bulk-load would look like the
        // load hung.
        assert!(parse(&["bulkload"]).unwrap_err().contains("bulkload"));
        assert!(parse(&["bulk-load", "now"]).unwrap_err().contains("now"));
    }
}
