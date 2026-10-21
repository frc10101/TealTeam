//! `tt-web dev --check` (F13): the dev environment seeds a throwaway
//! database in a temp folder and runs every access check on it, through the
//! built binary, the way `cargo run -- dev --check` does. A FAIL line in the
//! report fails this test, with the report printed.

use std::path::Path;
use std::process::{Command, Output};

fn dev(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tt-web"))
        .arg("dev")
        .args(args)
        .current_dir(dir)
        // Dev mode ignores these: a production-looking setup must not leak in.
        .env_remove("TEALTEAM_ENV")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/not-this.db?mode=rwc", dir.display()),
        )
        .env("ASK_BACKEND", "local")
        .env("ASK_URL", "http://127.0.0.1:1")
        .env("FTC_API_USERNAME", "someone")
        .env("FTC_API_KEY", "secret")
        .output()
        .expect("run tt-web dev")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_dev_environment_seeds_and_every_check_passes() {
    let dir = std::env::temp_dir().join(format!("tt-dev-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let first = dev(&dir, &["--check"]);
    let report = text(&first);
    assert!(first.status.success(), "dev --check failed:\n{report}");
    assert!(report.contains(" 0 failed,"), "{report}");
    // The matrix and the sections ran: hundreds of checks, not a handful.
    let total: usize = report
        .lines()
        .find_map(|l| l.strip_prefix("dev checks: "))
        .and_then(|l| l.rsplit(", ").next())
        .and_then(|l| l.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(total > 400, "only {total} checks ran:\n{report}");
    assert!(report.contains("PASS"), "{report}");

    // The data is in the folder it should be, and DATABASE_URL was not used.
    assert!(dir.join("dev-data/tealteam-dev.db").exists());
    assert!(!dir.join("not-this.db").exists());
    // No check scratch copy is left behind.
    let left: Vec<_> = std::fs::read_dir(dir.join("dev-data"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("check-") || n.ends_with(".seeding"))
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");

    // Run again: nothing is seeded twice, and it still passes.
    let again = dev(&dir, &["--check"]);
    assert!(
        again.status.success(),
        "second run failed:\n{}",
        text(&again)
    );

    // --fresh wipes and reseeds.
    let fresh = dev(&dir, &["--check", "--fresh"]);
    assert!(fresh.status.success(), "--fresh failed:\n{}", text(&fresh));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dev_mode_refuses_a_production_environment() {
    let dir = std::env::temp_dir().join(format!("tt-dev-prod-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_tt-web"))
        .args(["dev", "--check"])
        .current_dir(&dir)
        .env("TEALTEAM_ENV", "prod")
        .output()
        .expect("run tt-web dev");
    assert!(!out.status.success());
    assert!(text(&out).contains("TEALTEAM_ENV"), "{}", text(&out));
    // It stopped before making anything.
    assert!(!dir.join("dev-data").exists());

    std::fs::remove_dir_all(&dir).ok();
}
