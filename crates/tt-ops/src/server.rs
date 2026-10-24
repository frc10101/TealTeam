//! The running server, over plain HTTP on localhost, as the admin.
//!
//! Every change goes through the server's own forms, never around them: the
//! same validation, the same change log the tablets follow, the same line in
//! the server's log naming who did it. So the tool signs in once, as an
//! admin, the way the lead's laptop does, and keeps the session cookie for
//! the run. Nothing is stored.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, COOKIE, LOCATION, SET_COOKIE};
use tt_core::user::Roles;

const SESSION_COOKIE: &str = "tt_session";

#[derive(Debug, Clone)]
pub struct Report {
    pub ok: bool,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Failure {
    /// No session, or the server no longer accepts it: sign in again.
    NotSignedIn,
    /// The server was reached and said no.
    Refused(String),
    /// The server was not reached.
    Unreachable(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::NotSignedIn => write!(f, "not signed in"),
            Failure::Refused(why) => write!(f, "{why}"),
            Failure::Unreachable(why) => write!(f, "server not reached: {why}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Server {
    pub base: String,
    client: reqwest::Client,
    pub session: Option<String>,
    pub signed_in_as: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SchedulePost {
    pub code: String,
    pub name: String,
    pub timezone: String,
    pub date: String,
    pub schedule: String,
    pub roster: String,
}

impl Server {
    pub fn new(base: String) -> Server {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .expect("a plain HTTP client builds");
        Server {
            base,
            client,
            session: None,
            signed_in_as: None,
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// `POST /api/auth/login`. The server answers 303 either way: to `/` with
    /// a cookie, or back to the sign-in page with `failed=1`.
    pub async fn sign_in(&mut self, email: &str, password: &str) -> Result<(), Failure> {
        let response = self
            .client
            .post(self.url("/api/auth/login"))
            .form(&[("email", email), ("password", password)])
            .send()
            .await
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        let location = header(&response, LOCATION);
        if location.contains("failed") {
            return Err(Failure::Refused("wrong email or password".into()));
        }
        let cookie = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|c| {
                c.split(';')
                    .next()
                    .and_then(|pair| {
                        pair.trim()
                            .strip_prefix(SESSION_COOKIE)
                            .and_then(|r| r.strip_prefix('='))
                    })
                    .map(str::to_string)
            });
        match cookie {
            Some(value) if !value.is_empty() => {
                self.session = Some(value);
                self.signed_in_as = Some(email.to_string());
                Ok(())
            }
            _ => Err(Failure::Refused(format!(
                "signed in but no session cookie came back (status {})",
                response.status()
            ))),
        }
    }

    pub fn sign_out(&mut self) {
        self.session = None;
        self.signed_in_as = None;
    }

    fn cookie(&self) -> Result<String, Failure> {
        match &self.session {
            Some(s) => Ok(format!("{SESSION_COOKIE}={s}")),
            None => Err(Failure::NotSignedIn),
        }
    }

    /// Send a form as the admin; turn "go sign in" answers into
    /// [`Failure::NotSignedIn`].
    async fn post_form(
        &self,
        path: &str,
        form: &[(String, String)],
        accept: &str,
    ) -> Result<reqwest::Response, Failure> {
        let cookie = self.cookie()?;
        let response = self
            .client
            .post(self.url(path))
            .header(COOKIE, cookie)
            .header(ACCEPT, accept)
            .form(form)
            .send()
            .await
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        let status = response.status();
        let location = header(&response, LOCATION);
        if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || (status.is_redirection() && location.contains("/sign-in"))
        {
            return Err(Failure::NotSignedIn);
        }
        if status.is_server_error() {
            return Err(Failure::Refused(format!("server error {status}")));
        }
        Ok(response)
    }

    /// `POST /api/frc/sync`: FIRST and TBA now, or on an FTC build with a key,
    /// the FTC Events pull. JSON when asked for it.
    pub async fn sync_now(&self) -> Result<Report, Failure> {
        let response = self
            .post_form("/api/frc/sync", &[], "application/json")
            .await?;
        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Failure::Refused(format!("unreadable answer ({status}): {e}")))?;
        let ok = body["ok"].as_bool().unwrap_or(false);
        let mut lines = Vec::new();
        // The FTC build with no key answers 404 and says why.
        if let Some(error) = body["error"].as_str() {
            lines.push(error.to_string());
        }
        if let Some(headline) = body["headline"].as_str() {
            lines.push(headline.to_string());
        }
        let count = |key: &str| body[key].as_u64().unwrap_or(0);
        if body.get("events").is_some() {
            lines.push(format!(
                "{} events, {} teams, {} event-team links",
                count("events"),
                count("teams"),
                count("event_teams")
            ));
        }
        if body["pass_started"].as_bool() == Some(true) {
            lines.push(if body["statistics"].as_bool() == Some(true) {
                "matches and statistics follow in the background".into()
            } else {
                "matches follow in the background".into()
            });
        }
        if let Some(problems) = body["problems"].as_array() {
            for p in problems {
                if let Some(p) = p.as_str() {
                    lines.push(format!("problem: {p}"));
                }
            }
        }
        if let Some(report) = body["report"].as_array() {
            for p in report {
                if let Some(p) = p.as_str() {
                    lines.push(p.to_string());
                }
            }
        }
        if lines.is_empty() {
            lines.push(format!("status {status}: {body}"));
        }
        Ok(Report { ok, lines })
    }

    /// `POST /api/ftc/pull`: the schedule page comes back with what the pull
    /// did, read out of its alerts.
    pub async fn ftc_pull(&self) -> Result<Report, Failure> {
        let response = self.post_form("/api/ftc/pull", &[], "text/html").await?;
        Ok(report_from_page(&response.text().await.unwrap_or_default()))
    }

    /// `POST /api/schedule/import`, the lead's form.
    pub async fn import_schedule(&self, post: &SchedulePost) -> Result<Report, Failure> {
        let form = vec![
            ("code".to_string(), post.code.clone()),
            ("name".to_string(), post.name.clone()),
            ("timezone".to_string(), post.timezone.clone()),
            ("date".to_string(), post.date.clone()),
            ("schedule".to_string(), post.schedule.clone()),
            ("roster".to_string(), post.roster.clone()),
        ];
        let response = self
            .post_form("/api/schedule/import", &form, "text/html")
            .await?;
        Ok(report_from_page(&response.text().await.unwrap_or_default()))
    }

    /// `POST /api/admin/users/{id}`: roles and team, then a 303 whose query
    /// carries the outcome.
    pub async fn save_account(
        &self,
        id: i64,
        team: Option<i32>,
        roles: Roles,
    ) -> Result<Report, Failure> {
        let mut form = vec![(
            "team_number".to_string(),
            team.map(|t| t.to_string()).unwrap_or_default(),
        )];
        for (ticked, name) in [
            (roles.is_admin, "is_admin"),
            (roles.is_lead_scout, "is_lead_scout"),
            (roles.is_coach, "is_coach"),
        ] {
            if ticked {
                form.push((name.to_string(), "on".to_string()));
            }
        }
        let response = self
            .post_form(&format!("/api/admin/users/{id}"), &form, "text/html")
            .await?;
        Ok(outcome_from_redirect(
            &header(&response, LOCATION),
            response.status(),
        ))
    }

    /// `POST /api/admin/users/{id}/password`: signs that account out everywhere.
    pub async fn reset_password(&self, id: i64, new_password: &str) -> Result<Report, Failure> {
        let form = vec![("new_password".to_string(), new_password.to_string())];
        let response = self
            .post_form(
                &format!("/api/admin/users/{id}/password"),
                &form,
                "text/html",
            )
            .await?;
        Ok(outcome_from_redirect(
            &header(&response, LOCATION),
            response.status(),
        ))
    }
}

fn header(response: &reqwest::Response, name: reqwest::header::HeaderName) -> String {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// The accounts page's `done=` codes (tt-web's `accounts::Done`).
fn outcome_from_redirect(location: &str, status: StatusCode) -> Report {
    let query = location.split_once('?').map(|(_, q)| q).unwrap_or("");
    let code = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == "done")
        .map(|(_, v)| v)
        .unwrap_or("");
    let (ok, text) = match code {
        "saved" => (true, "Saved."),
        "reset" => (
            true,
            "Password reset; that account is signed out everywhere.",
        ),
        "last-admin" => (false, "Refused: that is the last admin."),
        "bad-team" => (false, "Refused: not a team number."),
        "short-password" => (false, "Refused: the password is too short (8 or more)."),
        "gone" => (false, "No such account any more."),
        "failed" => (false, "The server could not save it; see its log."),
        _ => {
            return Report {
                ok: status.is_success() || status.is_redirection(),
                lines: vec![format!(
                    "status {status}{}",
                    if location.is_empty() {
                        String::new()
                    } else {
                        format!(", to {location}")
                    }
                )],
            };
        }
    };
    Report {
        ok,
        lines: vec![text.to_string()],
    }
}

/// The schedule page's alerts: every `<li>` inside `alert-error` is a
/// problem, every one inside `alert-success` a line of the report.
pub fn report_from_page(html: &str) -> Report {
    let errors = alert_items(html, "alert-error");
    let report = alert_items(html, "alert-success");
    let ok = errors.is_empty() && !report.is_empty();
    let mut lines = report;
    lines.extend(errors.into_iter().map(|e| format!("problem: {e}")));
    if lines.is_empty() {
        lines.push(if html.contains("Match schedule") {
            "the page came back with no report".to_string()
        } else {
            format!("unexpected answer ({} bytes)", html.len())
        });
    }
    Report { ok, lines }
}

fn alert_items(html: &str, class: &str) -> Vec<String> {
    let Some(start) = html.find(class) else {
        return Vec::new();
    };
    let rest = &html[start..];
    let end = rest.find("</div>").unwrap_or(rest.len());
    let block = &rest[..end];
    let mut out = Vec::new();
    let mut cursor = block;
    while let Some(open) = cursor.find("<li>") {
        let after = &cursor[open + 4..];
        let Some(close) = after.find("</li>") else {
            break;
        };
        out.push(unescape(after[..close].trim()));
        cursor = &after[close + 5..];
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a running server, as its admin. Ignored unless asked for:
    ///
    /// ```sh
    /// TT_OPS_LIVE_URL=http://127.0.0.1:18080 TT_OPS_LIVE_EMAIL=ops@tealteam.local \
    /// TT_OPS_LIVE_PASSWORD=opsadmin1 TT_OPS_LIVE_ACCOUNT=1 \
    /// cargo test -p tt-ops live_ -- --ignored --nocapture
    /// ```
    ///
    /// Saves the account's roles as they are (admin on), imports the
    /// rehearsal schedule as REHEARSAL, asks for a sync, resets the password
    /// to the same value and signs in again with it.
    #[tokio::test]
    #[ignore]
    async fn live_admin_paths_against_a_running_server() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let mut server = Server::new(var("TT_OPS_LIVE_URL"));
        let email = var("TT_OPS_LIVE_EMAIL");
        let password = var("TT_OPS_LIVE_PASSWORD");
        let account: i64 = var("TT_OPS_LIVE_ACCOUNT").parse().expect("an account id");

        assert!(matches!(
            server.sign_in(&email, "wrong-password").await,
            Err(Failure::Refused(_))
        ));
        server.sign_in(&email, &password).await.expect("sign in");
        assert!(server.session.is_some());

        let saved = server
            .save_account(
                account,
                Some(10101),
                Roles {
                    is_admin: true,
                    is_lead_scout: true,
                    is_coach: false,
                },
            )
            .await
            .expect("save account");
        println!("save_account: {saved:?}");
        assert!(saved.ok, "{saved:?}");

        let csv = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/ftc/FTC_BETA_REHEARSAL_SCHEDULE.csv"
        ))
        .expect("the rehearsal schedule");
        let imported = server
            .import_schedule(&SchedulePost {
                code: "REHEARSAL".into(),
                name: "Ops rehearsal".into(),
                timezone: "America/Chicago".into(),
                date: "2026-10-24".into(),
                schedule: csv,
                roster: String::new(),
            })
            .await
            .expect("import");
        println!("import_schedule: {imported:?}");
        assert!(imported.ok, "{imported:?}");
        assert!(imported.lines[0].starts_with("Imported"));

        let bad = server
            .import_schedule(&SchedulePost {
                code: "REHEARSAL".into(),
                name: "Ops rehearsal".into(),
                timezone: "America/Chicago".into(),
                date: "2026-10-24".into(),
                schedule: "match,red1,red2,blue1,blue2\n1,abc,2,3,4\n".into(),
                roster: String::new(),
            })
            .await
            .expect("import");
        println!("bad import: {bad:?}");
        assert!(!bad.ok);
        assert!(bad.lines.iter().any(|l| l.starts_with("problem:")));

        let synced = server.sync_now().await.expect("sync answers");
        println!("sync_now: {synced:?}");
        assert!(!synced.lines.is_empty());

        let reset = server
            .reset_password(account, &password)
            .await
            .expect("reset");
        println!("reset_password: {reset:?}");
        assert!(reset.ok, "{reset:?}");
        // That signed the account out everywhere, this session included. (Not
        // probed with sync: on a build without one, "not on this build" comes
        // before the sign-in check.)
        let probe = server
            .save_account(
                account,
                Some(10101),
                Roles {
                    is_admin: true,
                    is_lead_scout: true,
                    is_coach: false,
                },
            )
            .await;
        assert!(matches!(probe, Err(Failure::NotSignedIn)), "{probe:?}");
        server
            .sign_in(&email, &password)
            .await
            .expect("sign in again");
    }

    #[test]
    fn reads_the_schedule_page_alerts() {
        let html = r#"<div class="alert alert-error" role="alert"><p>Nothing was imported.</p>
          <ul><li>row 3: team &quot;x&quot; is not a number</li><li>row 4: short</li></ul></div>
          <section>Match schedule</section>"#;
        let report = report_from_page(html);
        assert!(!report.ok);
        assert_eq!(report.lines.len(), 2);
        assert_eq!(
            report.lines[0],
            "problem: row 3: team \"x\" is not a number"
        );

        let html = r#"<div class="alert alert-success" role="status"><ul><li>Imported 12 matches and 24 teams for X.</li></ul>
          <p><strong>Surrogates (1):</strong></p><ul><li>Q3 red 2: 1234</li></ul><p><a href="/x">Assign scouts</a></p></div>"#;
        let report = report_from_page(html);
        assert!(report.ok);
        assert_eq!(
            report.lines,
            vec!["Imported 12 matches and 24 teams for X.", "Q3 red 2: 1234"]
        );
    }

    #[test]
    fn reads_the_account_outcome() {
        let r = outcome_from_redirect("/admin/users?event=x&done=saved", StatusCode::SEE_OTHER);
        assert!(r.ok);
        let r = outcome_from_redirect("/admin/users?done=last-admin", StatusCode::SEE_OTHER);
        assert!(!r.ok);
        assert!(r.lines[0].contains("last admin"));
    }
}
