//! The scouting assistant (AI1): `GET /ask` and `POST /ask`.
//!
//! Off unless `ASK_BACKEND` is set. A question is answered by a language model
//! from the data sheet `tt_core::ask` builds, and nothing else; the page shows
//! the sheet under the answer. See `docs/SCOUTING_ASSISTANT.md`.
//!
//! - `ASK_BACKEND=local`: any OpenAI-compatible chat endpoint at `ASK_URL`,
//!   such as `llama-server` on the Pi (the team's laptops are Chromebooks,
//!   which cannot run one).
//! - `ASK_BACKEND=anthropic`: the Anthropic Messages API with
//!   `ANTHROPIC_API_KEY`. It needs internet, so it is for use away from the
//!   venue: the event LAN has no uplink, and event data does not go by radio.
//!
//! One question runs at a time and none waits longer than `ASK_TIMEOUT_SECS`,
//! so a slow model can neither pile up work on the Pi nor hang a page. Nothing
//! else in the app waits on it.

use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::{Form, State};
use axum::response::Response;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::{info, warn};
use tt_core::ask::{self, INSTRUCTIONS, Observed, SHEET_BUDGET, Sheet, Team};
use tt_core::review::ReviewState;
use tt_core::timezone;
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{AskPage, Nav};

use crate::auth::Strategist;
use crate::events::{EventContext, EventParam};
use crate::handlers::{event_page, html};
use crate::startup::AppState;

/// Longest question accepted, in characters.
pub const MAX_QUESTION: usize = 500;

const DEFAULT_TIMEOUT_SECS: u64 = 90;
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";

/// What answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// An OpenAI-compatible `/v1/chat/completions` at `url`.
    Local { url: String, model: String },
    Anthropic {
        key: String,
        model: String,
        url: String,
    },
}

#[derive(Debug)]
pub struct Assistant {
    pub backend: Backend,
    pub timeout: Duration,
    http: reqwest::Client,
    /// One question at a time.
    busy: Semaphore,
}

/// Why a question got no answer, in words for the page.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AskError {
    #[error("Another question is being answered. Ask again in a moment.")]
    Busy,
    #[error(
        "The assistant did not answer within {0} seconds. Try a shorter question, or ask again."
    )]
    Timeout(u64),
    #[error(
        "The assistant at {0} could not be reached. Check that it is running and on the network."
    )]
    Unreachable(String),
    #[error("The assistant answered with an error ({0}). The server's log has the detail.")]
    Failed(u16),
    #[error("The assistant declined to answer that question.")]
    Declined,
    #[error("The assistant's reply was empty or unreadable. Ask again.")]
    Unreadable,
}

static ASSISTANT: OnceLock<Option<Assistant>> = OnceLock::new();

/// Read the settings once at startup. A bad setting is logged and leaves the
/// assistant off; it never stops the server.
pub fn init_from_env() {
    let assistant = match Assistant::from_lookup(|key| std::env::var(key).ok()) {
        Ok(Some(a)) => {
            info!("scouting assistant: {}", a.describe());
            Some(a)
        }
        Ok(None) => None,
        Err(e) => {
            warn!("scouting assistant is off: {e}");
            None
        }
    };
    let _ = ASSISTANT.set(assistant);
}

/// The assistant, when one is set up.
pub fn configured() -> Option<&'static Assistant> {
    ASSISTANT.get().and_then(Option::as_ref)
}

impl Assistant {
    /// `Ok(None)` when `ASK_BACKEND` is unset or `off`.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let setting = |key: &str| {
            get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let timeout = match setting("ASK_TIMEOUT_SECS") {
            None => DEFAULT_TIMEOUT_SECS,
            Some(raw) => raw.parse::<u64>().ok().filter(|s| *s > 0).ok_or(format!(
                "ASK_TIMEOUT_SECS must be a number of seconds, got {raw:?}"
            ))?,
        };
        let backend = match setting("ASK_BACKEND")
            .map(|b| b.to_ascii_lowercase())
            .as_deref()
        {
            None | Some("off") | Some("none") => return Ok(None),
            Some("local") => Backend::Local {
                url: setting("ASK_URL")
                    .ok_or("ASK_BACKEND=local needs ASK_URL, such as http://10.101.0.20:8080")?
                    .trim_end_matches('/')
                    .to_string(),
                model: setting("ASK_MODEL").unwrap_or_else(|| "local".into()),
            },
            Some("anthropic") => Backend::Anthropic {
                key: setting("ANTHROPIC_API_KEY")
                    .ok_or("ASK_BACKEND=anthropic needs ANTHROPIC_API_KEY")?,
                model: setting("ASK_MODEL").unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.into()),
                url: setting("ASK_URL").unwrap_or_else(|| ANTHROPIC_URL.into()),
            },
            Some(other) => {
                return Err(format!(
                    "ASK_BACKEND must be 'local', 'anthropic' or 'off', got {other:?}"
                ));
            }
        };
        Ok(Some(Self::new(backend, Duration::from_secs(timeout))))
    }

    pub fn new(backend: Backend, timeout: Duration) -> Self {
        Self {
            backend,
            timeout,
            http: reqwest::Client::new(),
            busy: Semaphore::new(1),
        }
    }

    /// For the page: what is answering.
    pub fn describe(&self) -> String {
        match &self.backend {
            Backend::Local { url, .. } => format!("a local model at {url}"),
            Backend::Anthropic { model, .. } => {
                format!("Claude ({model}) over the internet, which the event LAN does not have")
            }
        }
    }

    fn address(&self) -> &str {
        match &self.backend {
            Backend::Local { url, .. } | Backend::Anthropic { url, .. } => url,
        }
    }

    /// Answer `question` from `sheet`.
    pub async fn answer(&self, sheet: &str, question: &str) -> Result<String, AskError> {
        let Ok(_turn) = self.busy.try_acquire() else {
            return Err(AskError::Busy);
        };
        let request = match &self.backend {
            Backend::Local { url, model } => self
                .http
                .post(format!("{url}/v1/chat/completions"))
                .json(&local_body(model, sheet, question)),
            Backend::Anthropic { key, model, url } => self
                .http
                .post(url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", "server-side-fallback-2026-07-01")
                .json(&anthropic_body(model, sheet, question)),
        };
        let sent = tokio::time::timeout(self.timeout, async {
            let response = request.send().await?;
            let status = response.status();
            let body = response.text().await?;
            Ok::<_, reqwest::Error>((status, body))
        })
        .await;
        let (status, body) = match sent {
            Err(_) => return Err(AskError::Timeout(self.timeout.as_secs())),
            Ok(Err(e)) => {
                warn!("scouting assistant at {}: {e}", self.address());
                return Err(AskError::Unreachable(self.address().to_string()));
            }
            Ok(Ok(sent)) => sent,
        };
        if !status.is_success() {
            warn!(
                "scouting assistant answered {status}: {}",
                truncate(&body, 500)
            );
            return Err(AskError::Failed(status.as_u16()));
        }
        let reply: Value = serde_json::from_str(&body).map_err(|_| AskError::Unreadable)?;
        match &self.backend {
            Backend::Local { .. } => local_text(&reply),
            Backend::Anthropic { .. } => anthropic_text(&reply),
        }
    }
}

fn local_body(model: &str, sheet: &str, question: &str) -> Value {
    // The instructions and the sheet first and unchanged between questions,
    // so a server that caches a prompt's start (llama-server does) reads the
    // sheet once per change, not once per question.
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": format!("{INSTRUCTIONS}\n\nDATA SHEET\n\n{sheet}")},
            {"role": "user", "content": question},
        ],
        "temperature": 0.2,
        "max_tokens": 700,
        "stream": false,
    })
}

fn local_text(reply: &Value) -> Result<String, AskError> {
    reply["choices"][0]["message"]["content"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .ok_or(AskError::Unreadable)
}

fn anthropic_body(model: &str, sheet: &str, question: &str) -> Value {
    json!({
        "model": model,
        "max_tokens": 16000,
        "system": [
            {"type": "text", "text": INSTRUCTIONS},
            {"type": "text", "text": format!("DATA SHEET\n\n{sheet}"), "cache_control": {"type": "ephemeral"}},
        ],
        "messages": [{"role": "user", "content": question}],
        // Short lookups over a small sheet: low effort answers them well.
        "output_config": {"effort": "low"},
        // A safety classifier's false positive is retried on Anthropic's
        // recommended model rather than refused outright.
        "fallbacks": "default",
    })
}

fn anthropic_text(reply: &Value) -> Result<String, AskError> {
    // Check why it stopped before reading what it said: a refusal can come
    // with no text at all.
    if reply["stop_reason"].as_str() == Some("refusal") {
        return Err(AskError::Declined);
    }
    let text: Vec<&str> = reply["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect();
    let text = text.join("").trim().to_string();
    if text.is_empty() {
        Err(AskError::Unreadable)
    } else {
        Ok(text)
    }
}

fn truncate(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

/// `GET /ask`.
pub async fn get(
    State(state): State<AppState>,
    Strategist(user): Strategist,
    EventParam(requested): EventParam,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    html(page(&state, &user, nav, &context, configured(), None).await)
}

#[derive(Debug, Deserialize)]
pub struct Question {
    #[serde(default)]
    question: String,
}

/// `POST /ask`: the same page, with the answer.
pub async fn post(
    State(state): State<AppState>,
    Strategist(user): Strategist,
    EventParam(requested): EventParam,
    Form(form): Form<Question>,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    html(
        page(
            &state,
            &user,
            nav,
            &context,
            configured(),
            Some(&form.question),
        )
        .await,
    )
}

/// The page for `context`'s event, answering `question` when there is one.
pub async fn page(
    state: &AppState,
    user: &User,
    nav: Nav,
    context: &EventContext,
    assistant: Option<&Assistant>,
    question: Option<&str>,
) -> AskPage {
    let storage_ready = nav.storage_ready;
    let mut page = AskPage::new(nav);
    let Some(assistant) = assistant else {
        page.unavailable = "The assistant is not set up on this server. An admin can turn it \
            on with ASK_BACKEND in the server's settings: docs/SCOUTING_ASSISTANT.md."
            .into();
        return page;
    };
    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to ask about.".into()
        } else {
            "The server's storage is unavailable, so there is nothing to ask about.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    page.backend = assistant.describe();
    page.post_href = format!("/ask?event={}", event.key);

    let Some(question) = question.map(str::trim).filter(|q| !q.is_empty()) else {
        return page;
    };
    page.question = question.chars().take(MAX_QUESTION).collect();

    let sheet = match build_sheet(state, user, event, &page.question).await {
        Ok(sheet) => sheet,
        Err(e) => {
            warn!("assistant sheet for {}: {e}", event.key);
            page.error = "Could not read the event's data. Ask again.".into();
            return page;
        }
    };
    page.sheet_about = about(&sheet, &timezone::clock_time(event.zone(), Utc::now()));
    match assistant.answer(&sheet.text, &page.question).await {
        Ok(answer) => page.answer = answer,
        Err(e) => page.error = e.to_string(),
    }
    page.sheet = sheet.text;
    page
}

/// `"Built 10:32 from 41 observations of 12 teams."`
fn about(sheet: &Sheet, at: &str) -> String {
    let mut about = format!(
        "built {at} from {} observation{} of {} team{}",
        sheet.observations,
        if sheet.observations == 1 { "" } else { "s" },
        sheet.teams,
        if sheet.teams == 1 { "" } else { "s" },
    );
    if !sheet.focused.is_empty() {
        let named: Vec<String> = sheet.focused.iter().map(i32::to_string).collect();
        about.push_str(&format!(
            "; in full for {}, the rest in brief",
            named.join(", ")
        ));
    }
    if sheet.notes_left_out > 0 {
        about.push_str(&format!(
            ", {} notes left out for length",
            sheet.notes_left_out
        ));
    }
    about
}

async fn build_sheet(
    state: &AppState,
    user: &User,
    event: &tt_core::records::Event,
    question: &str,
) -> tt_repo::Result<Sheet> {
    let approved = state.repo.approved_observations(&event.key).await?;
    let overrides = state.repo.weight_overrides().await?;
    let roster = state.repo.event_teams(&event.key).await?;
    let stats = state.repo.event_stats(&event.key).await?;
    let matches = state.repo.event_matches(&event.key).await?;

    let labels: Vec<(String, String)> =
        matches.iter().map(|m| (m.key.clone(), m.label())).collect();
    let label = |key: &str| {
        labels
            .iter()
            .find(|(k, _)| k == key)
            .map_or(key.to_string(), |(_, l)| l.clone())
    };
    let match_labels: Vec<String> = approved.iter().map(|o| label(&o.match_key)).collect();
    let observed: Vec<Observed> = approved
        .iter()
        .zip(&match_labels)
        .filter(|(o, _)| o.review_state == ReviewState::Approved)
        .map(|(o, label)| Observed {
            team_number: o.team_number,
            payload: &o.payload,
            schema_version: o.schema_version,
            submitting_team: o.submitting_team,
            match_label: label,
        })
        .collect();
    let teams: Vec<Team> = roster
        .iter()
        .map(|t| Team {
            number: t.number,
            name: &t.name,
            stats: stats.iter().find(|s| s.team_number == t.number),
        })
        .collect();
    let now = Utc::now();
    Ok(ask::sheet(
        &state.season,
        &overrides,
        &event.name,
        &timezone::day_and_time(event.zone(), now),
        &teams,
        &observed,
        user.team_number,
        &ask::named_teams(question),
        SHEET_BUDGET,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::post;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn unset_or_off_is_no_assistant() {
        assert!(Assistant::from_lookup(lookup(&[])).unwrap().is_none());
        assert!(
            Assistant::from_lookup(lookup(&[("ASK_BACKEND", "off")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_local_backend_needs_its_address() {
        let err = Assistant::from_lookup(lookup(&[("ASK_BACKEND", "local")])).unwrap_err();
        assert!(err.contains("ASK_URL"), "{err}");
        let a = Assistant::from_lookup(lookup(&[
            ("ASK_BACKEND", "local"),
            ("ASK_URL", "http://10.101.0.20:8080/"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            a.backend,
            Backend::Local {
                url: "http://10.101.0.20:8080".into(),
                model: "local".into()
            }
        );
        assert_eq!(a.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    }

    #[test]
    fn anthropic_needs_a_key_and_defaults_to_opus() {
        assert!(Assistant::from_lookup(lookup(&[("ASK_BACKEND", "anthropic")])).is_err());
        let a = Assistant::from_lookup(lookup(&[
            ("ASK_BACKEND", "Anthropic"),
            ("ANTHROPIC_API_KEY", "sk-test"),
            ("ASK_TIMEOUT_SECS", "30"),
        ]))
        .unwrap()
        .unwrap();
        assert!(
            matches!(&a.backend, Backend::Anthropic { model, .. } if model == "claude-opus-5-5")
        );
        assert_eq!(a.timeout, Duration::from_secs(30));
        assert!(a.describe().contains("does not have"));
    }

    #[test]
    fn a_bad_setting_is_an_error_not_a_guess() {
        assert!(Assistant::from_lookup(lookup(&[("ASK_BACKEND", "gpt")])).is_err());
        assert!(
            Assistant::from_lookup(lookup(&[
                ("ASK_BACKEND", "local"),
                ("ASK_URL", "http://x"),
                ("ASK_TIMEOUT_SECS", "soon"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn a_claude_refusal_is_declined_and_text_blocks_are_joined() {
        let refused = json!({"stop_reason": "refusal", "content": []});
        assert_eq!(anthropic_text(&refused), Err(AskError::Declined));
        let ok = json!({"stop_reason": "end_turn", "content": [
            {"type": "thinking", "thinking": ""},
            {"type": "text", "text": "Team 7 parks "},
            {"type": "text", "text": "in 4 of 5."},
        ]});
        assert_eq!(anthropic_text(&ok).unwrap(), "Team 7 parks in 4 of 5.");
    }

    #[test]
    fn the_sheet_goes_in_the_system_prompt_and_the_question_alone_after_it() {
        let body = local_body("m", "Team 7 ...", "who parks?");
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.starts_with(INSTRUCTIONS));
        assert!(system.ends_with("Team 7 ..."));
        assert_eq!(body["messages"][1]["content"], "who parks?");
        let body = anthropic_body("claude-opus-5-5", "Team 7 ...", "who parks?");
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["messages"][0]["content"], "who parks?");
    }

    /// A stand-in for `llama-server`, answering after `delay`.
    async fn stub(delay: Duration) -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |axum::Json(body): axum::Json<Value>| async move {
                tokio::time::sleep(delay).await;
                let asked = body["messages"][1]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                axum::Json(
                    json!({"choices": [{"message": {"content": format!(" You asked: {asked} ")}}]}),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    fn local(url: String, timeout: Duration) -> Assistant {
        Assistant::new(
            Backend::Local {
                url,
                model: "m".into(),
            },
            timeout,
        )
    }

    #[tokio::test]
    async fn a_local_model_answers() {
        let a = local(stub(Duration::ZERO).await, Duration::from_secs(5));
        assert_eq!(
            a.answer("sheet", "who parks?").await.unwrap(),
            "You asked: who parks?"
        );
    }

    #[tokio::test]
    async fn a_slow_model_times_out_and_a_second_question_waits_its_turn() {
        let a = std::sync::Arc::new(local(
            stub(Duration::from_millis(600)).await,
            Duration::from_millis(300),
        ));
        let first = tokio::spawn({
            let a = a.clone();
            async move { a.answer("sheet", "one").await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(a.answer("sheet", "two").await, Err(AskError::Busy));
        assert_eq!(first.await.unwrap(), Err(AskError::Timeout(0)));
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let a = local(url.clone(), Duration::from_secs(5));
        assert_eq!(
            a.answer("sheet", "q").await,
            Err(AskError::Unreachable(url))
        );
    }
}
