# TealTeam

An FRC scouting and analytics application for team 10101 — manual scouting observations merged with official FIRST and Blue Alliance competition data, served from a Raspberry Pi at the event on a LAN with no reliable internet.

## Status: active Rust implementation

The current implementation is Rust-based and is organized as a Cargo workspace. The retired Go, .NET, and Render ports are no longer part of this repository.

The application is functional end to end: the server, SQLite repository, season-driven scouting forms, upstream sync, offline client pieces, QR handoff, and browser-side read-only pages are implemented and covered by Rust and browser tests. The remaining software work is tracked at the top of [docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md).

### Quick start

Install the Rust toolchain specified by [rust-toolchain.toml](rust-toolchain.toml), then:

```sh
cp .env.example .env
cargo run -p tt-web
```

The server listens on `http://127.0.0.1:8080` by default. Check it with:

```sh
curl http://127.0.0.1:8080/health
```

Run the local verification suite with:

```sh
./check.sh
```

Use `./check.sh --fast` to skip the wasm32 builds. The browser tests under `crates/tt-web/tests/browser/` require their documented browser-test tooling and are separate from the default Rust test command. To build the service-worker client bundle, install the locked `wasm-bindgen-cli` version and run:

```sh
deploy/build-client.sh
```

## Start here

| Document | What it is |
| --- | --- |
| **[docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md)** | Current status, remaining work, implementation history, and resolved decisions. |
| [docs/REBUILD_SPEC.md](docs/REBUILD_SPEC.md) | Domain, data model, route surface, business rules, upstream API behavior, front-end conventions, configuration, and deployment constraints. |
| [RefurbishInstructions.md](RefurbishInstructions.md) | The rationale for the offline-first client, season-driven schemas, and sync design. |

Work from the action items. Read the rebuild spec for what the app *did*, and the refurbish plan for why the target design is what it is. Where the two disagree, the refurbish plan wins — it was written knowing this system's flaws.

### Supporting reference

- [docs/FRC_API_Calls.md](docs/FRC_API_Calls.md) — FIRST and TBA endpoint catalog with payload examples
- [docs/TBA_SCHEMA_FIX_SUMMARY.md](docs/TBA_SCHEMA_FIX_SUMMARY.md) — TBA schema variance across seasons; read before writing deserializers
- [docs/TIMEZONE_HANDLING.md](docs/TIMEZONE_HANDLING.md) — event timezone rules
- [docs/DataPoints.md](docs/DataPoints.md) — what is collected and what is derived
- [docs/TEAM_STATS_DISPLAY.md](docs/TEAM_STATS_DISPLAY.md) — what the team page shows and why it matters
- [docs/PREDICTIONS_REIMPLEMENTATION.md](docs/PREDICTIONS_REIMPLEMENTATION.md) — the removed OPR/DPR prediction feature and its formulas
- [docs/TEALTEAM_DETAILED_OVERVIEW_SOURCE.md](docs/TEALTEAM_DETAILED_OVERVIEW_SOURCE.md) — dense domain synthesis (its stack and deployment sections describe the retired implementation)

## Target stack

Rust + axum + Askama + sqlx, over SQLite, on a Raspberry Pi 5 at the event. One server, and it lives at the event — no cloud tier.

Pages are server-rendered HTML that works without JavaScript, styled by one hand-written stylesheet. A few small scripts in `crates/tt-web/static/js/` add to them, with no build step. The retired app's Unpoly and Tailwind are deliberately gone; [ACTION_ITEMS.md](docs/ACTION_ITEMS.md) (U8, and the Phase 1 notes) says why.

The one structural rule to get right on the first commit: split the workspace into a pure `tt-core` / `tt-templates` layer that compiles to `wasm32`, behind a `Repo` trait, with a CI job enforcing it. See [REBUILD_SPEC.md §9](docs/REBUILD_SPEC.md#9-crate-layout-for-the-rebuild). Retrofitting that split was the single largest item in the refurbish plan; building it in costs almost nothing.

## Rust toolchain

The repository pins Rust in [rust-toolchain.toml](rust-toolchain.toml). `rustup` will select the same compiler locally, and CI uses the same version. Do not change the pinned version casually: update the toolchain file and CI together, then run `./check.sh`.

The `wasm32-unknown-unknown` target is included by the toolchain file. The client bundle also needs the `wasm-bindgen-cli` version recorded in `Cargo.lock`; `deploy/build-client.sh` checks that version and prints the exact install command when it is missing.
