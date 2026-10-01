//! One simulated scout: a phone with its own connections, doing what a scout
//! does between matches.
//!
//! Each cycle: open the scouting page for the next match, save an
//! observation, land on the page the save redirects to, and pull what changed.
//! The tablet checks in once a minute (device.js), and scout 0 is also the
//! lead, refreshing the assignment grid each cycle as its live region does.
//!
//! A save that fails or hangs past [`REQUEST_TIMEOUT`] is sent again with the
//! same record id, every [`RESAVE`], as a scout tapping Save again would --
//! including after the run's end, for up to [`GRACE`], so no scout walks
//! away from an unsaved observation.

use std::time::Duration;

use anyhow::Context;
use rand::Rng;
use reqwest::header::{LOCATION, SET_COOKIE};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::Value;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::report::Shared;
use crate::seed::{EVENT, match_key, robots};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const RESAVE: Duration = Duration::from_secs(2);
pub const GRACE: Duration = Duration::from_secs(120);
const HEARTBEAT: Duration = Duration::from_secs(60);
const PASSWORD: &str = "load-test-password";

/// How the run is paced.
pub struct Plan {
    pub base: String,
    pub clients: usize,
    pub cycle: Duration,
    /// When scouts stop starting new cycles, since the run began.
    pub until: Duration,
    pub matches: u32,
    /// Where every stream starts: the change log's head before anyone saved.
    pub cursors: (i64, i64),
}

/// A phone's connections. Pages and saves give up after
/// [`REQUEST_TIMEOUT`]; streams have their own.
pub fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("http client")
}

fn uuid() -> String {
    let ms = chrono::Utc::now().timestamp_millis() as u64;
    tt_core::record_id::uuid_v7(ms, rand::rng().random())
}

/// Sign scout `index` up, or in if this server has met them before. The
/// cookie header their requests carry: session and tablet.
pub async fn sign_in(base: &str, index: usize) -> anyhow::Result<String> {
    let http = client();
    let email = format!("scout{index}%40load.test");
    let signup = format!(
        "name=Load+Scout+{index}&email={email}&team_number=10101\
         &password={PASSWORD}&confirm_password={PASSWORD}"
    );
    let login = format!("email={email}&password={PASSWORD}");
    for (path, body) in [("/api/auth/signup", signup), ("/api/auth/login", login)] {
        let response = http
            .post(format!("{base}{path}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .with_context(|| format!("scout {index}: {path}"))?;
        let session = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.strip_prefix("tt_session="))
            .and_then(|v| v.split(';').next());
        if let Some(session) = session {
            return Ok(format!("tt_session={session}; tt_device={}", uuid()));
        }
    }
    anyhow::bail!("scout {index} could neither sign up nor sign in")
}

/// What came back, read to the end.
struct Reply {
    status: StatusCode,
    location: Option<String>,
    body: String,
}

/// Send, read the whole answer, and record how long that took.
async fn timed(shared: &Shared, route: &'static str, request: RequestBuilder) -> Option<Reply> {
    let started = Instant::now();
    let reply = async {
        let response = request.send().await.ok()?;
        let status = response.status();
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let body = response.text().await.ok()?;
        Some(Reply {
            status,
            location,
            body,
        })
    }
    .await;
    let ok = reply
        .as_ref()
        .is_some_and(|r| r.status.is_success() || r.status.is_redirection());
    shared.sample(route, started, ok);
    reply.filter(|_| ok)
}

/// Pull from `cursors` until there is no more. Every change, and the cursors
/// to send next time. `None` if any page failed.
pub async fn pull(
    shared: &Shared,
    http: &Client,
    base: &str,
    cookie: &str,
    mut cursors: (i64, i64),
) -> Option<((i64, i64), Vec<Value>)> {
    let mut changes = Vec::new();
    loop {
        let url = format!(
            "{base}/api/sync/pull?changes={}&upstream={}&event={EVENT}",
            cursors.0, cursors.1
        );
        let reply = timed(shared, "pull", http.get(url).header("cookie", cookie)).await?;
        let page: Value = serde_json::from_str(&reply.body).ok()?;
        cursors = (
            page["changes_cursor"].as_i64().unwrap_or(cursors.0),
            page["upstream_cursor"].as_i64().unwrap_or(cursors.1),
        );
        if let Some(rows) = page["changes"].as_array() {
            changes.extend(rows.iter().cloned());
        }
        if page["changes_more"] != true && page["upstream_more"] != true {
            return Some((cursors, changes));
        }
    }
}

/// A random but valid set of answers.
fn answers() -> String {
    let mut rng = rand::rng();
    let position = ["left", "center", "right"][rng.random_range(0..3)];
    let broke = if rng.random_bool(0.1) {
        "&f.broke_down=on"
    } else {
        ""
    };
    format!(
        "f.starting_position={position}&f.auto_scored={}&f.teleop_scored={}\
         &f.penalties={}&f.notes=load+test{broke}",
        rng.random_range(0..=20),
        rng.random_range(0..=60),
        rng.random_range(0..=3),
    )
}

/// Scout `index` works until the plan says stop. `navigated` tells its
/// stream that the page changed.
pub async fn work(shared: &Shared, plan: &Plan, index: usize, cookie: &str, navigated: &Notify) {
    let http = client();
    let base = &plan.base;
    let with = |r: RequestBuilder| r.header("cookie", cookie);
    let mut cursors = plan.cursors;
    let mut heartbeat_due = Instant::now();

    // Spread the scouts across the first cycle, as they would arrive.
    tokio::time::sleep(plan.cycle.mul_f64(index as f64 / plan.clients as f64)).await;

    for n in 1..=plan.matches {
        if shared.elapsed() >= plan.until {
            break;
        }
        let began = Instant::now();
        let key = match_key(n);
        let team = robots(n)[index % 6];

        if began >= heartbeat_due {
            let url = format!("{base}/api/device/heartbeat");
            timed(shared, "heartbeat", with(http.post(url))).await;
            heartbeat_due = began + HEARTBEAT;
        }

        let page = format!("{base}/submission?event={EVENT}&match={key}&team={team}");
        timed(shared, "scouting page", with(http.get(page))).await;

        let record_id = uuid();
        let form = format!(
            "match={key}&team={team}&record_id={record_id}&{}",
            answers()
        );
        let mut attempts = 0;
        let landed = loop {
            attempts += 1;
            let save = with(http.post(format!("{base}/api/submission")))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(form.clone());
            match timed(shared, "save", save).await {
                Some(r) if r.status == StatusCode::SEE_OTHER => {
                    shared
                        .acked
                        .lock()
                        .unwrap()
                        .insert(record_id.clone(), shared.elapsed());
                    break r.location;
                }
                Some(r) => {
                    // The form came back: the server read the save and said no.
                    let why = r
                        .body
                        .find("Not saved")
                        .map(|at| r.body[at..].chars().take(90).collect::<String>())
                        .unwrap_or_else(|| format!("HTTP {}", r.status));
                    shared
                        .refusals
                        .lock()
                        .unwrap()
                        .push(format!("scout {index}, Q{n} {team}: {why}"));
                    break None;
                }
                None if shared.elapsed() > plan.until + GRACE => break None,
                None => tokio::time::sleep(RESAVE).await,
            }
        };
        if attempts > 1 {
            *shared.retried.lock().unwrap() += 1;
        }

        if let Some(next) = landed {
            timed(
                shared,
                "page after save",
                with(http.get(format!("{base}{next}"))),
            )
            .await;
            navigated.notify_one();
        }

        if let Some((next, _)) = pull(shared, &http, base, cookie, cursors).await {
            cursors = next;
        }

        if index == 0 {
            let grid = format!("{base}/lead-scout/assignments?event={EVENT}");
            timed(shared, "assignment grid", with(http.get(grid))).await;
        }

        let pace = plan.cycle.mul_f64(rand::rng().random_range(0.8..1.2));
        tokio::time::sleep_until(began + pace).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_what_the_season_form_accepts() {
        let schema = tt_core::season::current_season().unwrap();
        for _ in 0..50 {
            let pairs: Vec<(String, String)> = answers()
                .split('&')
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap();
                    (k.to_string(), v.replace('+', " "))
                })
                .collect();
            let raw = tt_core::form::RawAnswers::from_pairs(&pairs);
            assert!(
                tt_core::form::read_answers(&schema, &raw).is_ok(),
                "{pairs:?}"
            );
        }
    }
}
