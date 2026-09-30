//! Askama templates and their view models.
//!
//! Askama compiles templates to Rust at build time, so a malformed template is a
//! build error rather than a runtime 500 -- worth a great deal on a team of
//! high-school students (REBUILD_SPEC.md 7).
//!
//! This crate has no server dependencies and is held to the wasm32 gate along
//! with `tt-core`, because that is what allows rendering to move into a service
//! worker later without rewriting the UI (ACTION_ITEMS.md C5/C6).
//!
//! View models live beside their templates: handlers assemble a struct, the
//! template consumes it, and no template ever reaches back into storage.

use askama::Template;

mod coach;
pub use coach::{AllianceView, CoachCard, CoachTeam, DriveCoachPage};
mod team;
pub use team::{
    EventLink, NoteLine, StatLine, SummaryLine, SummarySection, TeamAtEvent, TeamCard,
    TeamMatchLine, TeamPage, stat_lines, summary_sections, team_href,
};
use tt_core::assignments::{self, AssigneeKey, Assignment, Sighting, SlotState};
use tt_core::form::{FormErrors, RawAnswers, input_name, is_on};
use tt_core::records::{Event, MatchRecord, Team};
use tt_core::season::{FieldKind, SeasonSchema};
use tt_core::user::User;

/// A template failed to render.
///
/// Wraps the engine's error so callers never name it. `tt-web` depends on this
/// crate, not on Askama -- swapping engines is then a change here and nowhere
/// else.
#[derive(Debug, thiserror::Error)]
#[error("template render failed: {0}")]
pub struct RenderError(#[from] askama::Error);

/// Renders to a complete HTML string.
///
/// Blanket-implemented for every Askama template in this crate, so adding a page
/// costs one `#[derive(Template)]` and nothing else.
pub trait Page {
    fn render_html(&self) -> Result<String, RenderError>;
}

impl<T: Template> Page for T {
    fn render_html(&self) -> Result<String, RenderError> {
        Ok(self.render()?)
    }
}

/// Layout chrome: what the nav bar and footer need on every page.
///
/// Assembled once per request from the signed-in user. Role flags are
/// pre-resolved here so templates never contain access logic -- and note that
/// hiding a nav link is a courtesy, not a permission check. The typed guard on
/// the handler is the actual control.
#[derive(Debug, Clone, Default)]
pub struct Nav {
    pub signed_in: bool,
    pub name: String,
    pub can_lead: bool,
    pub can_coach: bool,
    pub can_admin: bool,
    /// False when the database is unreachable, so the footer can say so rather
    /// than letting a scout type into a form that will not save.
    pub storage_ready: bool,
    /// The header's event switcher. Hidden on pages that set no options.
    pub event: EventSwitcher,
}

impl Nav {
    pub fn anonymous(storage_ready: bool) -> Self {
        Self {
            storage_ready,
            ..Self::default()
        }
    }

    pub fn for_user(user: Option<&User>, storage_ready: bool) -> Self {
        match user {
            Some(u) => Self {
                signed_in: true,
                name: u.name.clone(),
                can_lead: u.roles.can_lead(),
                can_coach: u.roles.can_coach(),
                can_admin: u.roles.can_admin(),
                storage_ready,
                event: EventSwitcher::default(),
            },
            None => Self::anonymous(storage_ready),
        }
    }
}

/// The event switcher in the header (U2).
///
/// The selected event lives in the URL (`?event=2026mabil`), not on the
/// session: a page is bookmarkable, two tabs can show two events, and nothing
/// about it needs the server's memory -- which is what it will need offline.
#[derive(Debug, Clone, Default)]
pub struct EventSwitcher {
    pub options: Vec<EventOption>,
    /// Key of the event the page is showing; empty when there is none.
    pub selected: String,
}

#[derive(Debug, Clone)]
pub struct EventOption {
    pub key: String,
    /// `"Greater Boston Regional · Mar 12–15"`.
    pub label: String,
    /// Precomputed so the template holds no comparison logic.
    pub selected: bool,
}

impl EventSwitcher {
    pub fn new(events: &[Event], selected: Option<&Event>) -> Self {
        let selected = selected.map(|e| e.key.clone()).unwrap_or_default();
        Self {
            options: events
                .iter()
                .map(|e| EventOption {
                    key: e.key.clone(),
                    label: match e.date_range() {
                        Some(dates) => format!("{} · {dates}", e.name),
                        None => e.name.clone(),
                    },
                    selected: e.key == selected,
                })
                .collect(),
            selected,
        }
    }

    /// `?event=<key>` for event-scoped links, so moving between pages keeps the
    /// event. Empty when nothing is selected.
    ///
    /// Not percent-encoded: keys are `{year}{event code}`, alphanumeric, and
    /// only ever taken from the database, never echoed from a request.
    pub fn query(&self) -> String {
        if self.selected.is_empty() {
            String::new()
        } else {
            format!("?event={}", self.selected)
        }
    }
}

/// What the home page says about the selected event (U3).
#[derive(Debug, Clone, Default)]
pub struct EventPanel {
    pub summary: Option<EventSummary>,
    /// The URL named an event this database does not have.
    pub unknown_key: String,
    /// The database holds no events at all.
    pub none_loaded: bool,
}

#[derive(Debug, Clone)]
pub struct EventSummary {
    pub name: String,
    /// `"Mar 12–15, 2026"`; empty when the event has no dates.
    pub dates: String,
    pub location: String,
    pub team_count: usize,
    pub match_count: usize,
    pub played_count: usize,
    /// By team number.
    pub roster: Vec<RosterEntry>,
    /// The viewer has a team and it is not on the roster (REBUILD_SPEC.md 5.1).
    pub team_missing: bool,
}

#[derive(Debug, Clone)]
pub struct RosterEntry {
    pub number: i32,
    pub name: String,
    /// The viewer's own team, so they can find it in a list of fifty.
    pub is_viewer: bool,
}

impl EventSummary {
    pub fn new(
        event: &Event,
        roster: &[Team],
        matches: &[MatchRecord],
        viewer_team: Option<i32>,
    ) -> Self {
        Self {
            name: event.name.clone(),
            dates: event.date_range_with_year().unwrap_or_default(),
            location: event.location.clone().unwrap_or_default(),
            team_count: roster.len(),
            match_count: matches.len(),
            played_count: matches.iter().filter(|m| m.played).count(),
            roster: roster
                .iter()
                .map(|t| RosterEntry {
                    number: t.number,
                    name: t.name.clone(),
                    is_viewer: viewer_team == Some(t.number),
                })
                .collect(),
            team_missing: viewer_team.is_some_and(|n| !roster.iter().any(|t| t.number == n)),
        }
    }
}

/// Status page shown when there is nothing else to say yet -- including when the
/// database is down and the app is deliberately serving degraded pages.
#[derive(Template)]
#[template(path = "health.html")]
pub struct HealthPage {
    pub storage_ready: bool,
    pub schema_version: Option<i64>,
}

#[derive(Template)]
#[template(path = "pages/home.html")]
pub struct HomePage {
    pub title: String,
    pub nav: Nav,
    pub team_display: String,
    pub season_name: String,
    pub season_year: i32,
    pub event: EventPanel,
}

#[derive(Template)]
#[template(path = "pages/sign_in.html")]
pub struct SignInPage {
    pub title: String,
    pub nav: Nav,
    /// Preserved across a failed attempt so the user does not retype it.
    pub email: String,
    pub error: String,
}

#[derive(Template)]
#[template(path = "pages/sign_up.html")]
pub struct SignUpPage {
    pub title: String,
    pub nav: Nav,
    pub name: String,
    pub email: String,
    pub team_number: String,
    pub error: String,
    /// The first account on a fresh database becomes an admin; say so, so it is
    /// a deliberate act rather than a surprise.
    pub first_account: bool,
}

/// A reachable, access-controlled page whose content is still to come.
///
/// Exists so the nav never links a 404, and so the guard on a privileged route
/// is settled before there is anything on it worth protecting.
#[derive(Template)]
#[template(path = "pages/placeholder.html")]
pub struct PlaceholderPage {
    pub title: String,
    pub nav: Nav,
    pub heading: String,
    pub summary: String,
    pub season_name: String,
}

/// Which dead end a browser reached (U10). Picks the wording on the error page;
/// the status code itself stays whatever the server said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// 404: nothing at this address.
    NotFound,
    /// 405: the address a form posts to, opened as a page. After a form comes
    /// back with an error the address bar shows it, so opening it again -- from
    /// history, the address bar, or a restored tab -- lands here.
    FormAddress,
    /// Any other 4xx: the server could not use what was sent.
    Refused,
    /// 5xx.
    ServerFault,
}

impl ErrorKind {
    pub fn for_status(status: u16) -> Self {
        match status {
            404 => Self::NotFound,
            405 => Self::FormAddress,
            500.. => Self::ServerFault,
            _ => Self::Refused,
        }
    }

    /// The page heading, and its tab title.
    pub fn heading(self) -> &'static str {
        match self {
            Self::NotFound => "Page not found",
            Self::FormAddress => "Nothing to show here",
            Self::Refused => "That did not work",
            Self::ServerFault => "Something went wrong",
        }
    }
}

/// What a browser gets instead of an empty or plain-text error response
/// (`tt-web`'s `errors` layer). Full layout, so the nav is there to go on from.
#[derive(Template)]
#[template(path = "pages/error.html")]
pub struct ErrorPage {
    pub title: String,
    pub nav: Nav,
    pub kind: ErrorKind,
    pub status: u16,
    /// The address asked for, shown so a scout can read it out to whoever is
    /// fixing the link.
    pub path: String,
}

impl ErrorPage {
    pub fn new(status: u16, path: String, nav: Nav) -> Self {
        let kind = ErrorKind::for_status(status);
        Self {
            title: kind.heading().into(),
            nav,
            kind,
            status,
            path,
        }
    }
}

/// The lead-scout panel. For now it carries the upstream card and the manual
/// sync (I13); the queue, rankings, and assignments arrive with L1-L12.
#[derive(Template)]
#[template(path = "pages/lead_scout.html")]
pub struct LeadScoutPage {
    pub title: String,
    pub nav: Nav,
    pub season_name: String,
    pub upstream: UpstreamPanel,
    /// What is stored for the selected event, so a sync can be watched landing.
    /// `None` with no event, or when storage could not say.
    pub stored: Option<StoredCounts>,
    /// The review queue (L8). `None` with no event selected.
    pub queue: Option<ReviewQueue>,
    /// What the review that led here did.
    pub reviewed: String,
}

// ── Rankings (L11) and weights (L12) ────────────────────────────────────────

/// Teams at an event, by official rank or by what scouts recorded.
#[derive(Template)]
#[template(path = "pages/rankings.html")]
pub struct RankingsPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why there is no table, when there is not.
    pub unavailable: String,
    pub errors: Vec<String>,
    pub rows: Vec<RankingView>,
    /// The column headers, each a link that sorts by it.
    pub columns: Vec<SortLink>,
    /// Observations still waiting for review: not counted yet.
    pub pending: usize,
    /// Point values the lead scout has changed from the schema.
    pub weights_changed: usize,
    /// When the newest counted observation was recorded; empty with none (U14).
    pub latest_scouted: String,
    /// When the ranks were last synced or typed in; empty when there are none
    /// (I12).
    pub ranks_updated: String,
    /// Old enough, while the event runs, that the ranks may have moved.
    pub ranks_stale: bool,
}

#[derive(Debug, Clone)]
pub struct SortLink {
    pub label: &'static str,
    pub href: String,
    /// This column orders the table.
    pub current: bool,
    /// Right-aligned numbers.
    pub numeric: bool,
}

#[derive(Debug, Clone)]
pub struct RankingView {
    /// Empty when the event has not ranked the team.
    pub rank: String,
    pub team: i32,
    pub name: String,
    /// `"12.5"`; empty when no approved observation counts.
    pub score: String,
    /// How many observations the score averages.
    pub n: usize,
    /// Too few observations to lean on.
    pub thin: bool,
}

/// Fewer observations than this and a score is marked as thin.
pub const THIN_BELOW: usize = 3;

/// Typing the rankings in off the audience display (I14).
#[derive(Template)]
#[template(path = "pages/rankings_entry.html")]
pub struct RankingsEntryPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why there is no form, when there is not.
    pub unavailable: String,
    /// Every bad line of a refused save, or anything else that stopped it.
    pub errors: Vec<String>,
    /// This is a save coming back refused, so nothing was stored.
    pub refused: bool,
    /// What the save that led here did.
    pub notice: String,
    /// The box's contents: the stored ranking, or what a refused save sent.
    pub text: String,
    /// How old the stored ranking is, `"12 min ago"`; empty when there is none.
    pub as_of: String,
    pub roster_size: usize,
    pub post_href: String,
    pub back_href: String,
}

/// The point values behind the scouting score (L12).
#[derive(Template)]
#[template(path = "pages/weights.html")]
pub struct WeightsPage {
    pub title: String,
    pub nav: Nav,
    pub groups: Vec<WeightGroup>,
    /// What the save that led here did.
    pub notice: String,
    /// The form came back with values to fix: nothing was saved.
    pub rejected: bool,
    /// Anything else that stopped a save.
    pub errors: Vec<String>,
    /// Point values currently changed from the schema.
    pub changed: usize,
}

#[derive(Debug, Clone)]
pub struct WeightGroup {
    pub field_label: String,
    pub inputs: Vec<WeightInput>,
}

#[derive(Debug, Clone)]
pub struct WeightInput {
    /// `weight_{field}__{option}`.
    pub name: String,
    pub option_label: String,
    /// As stored, or as typed when the form came back.
    pub value: String,
    pub default: i64,
    /// Differs from the default.
    pub changed: bool,
    pub error: String,
}

// ── Review (L8-L10) ─────────────────────────────────────────────────────────

/// Observations waiting for the lead scout, oldest first.
#[derive(Debug, Clone, Default)]
pub struct ReviewQueue {
    pub items: Vec<QueueItem>,
    /// Storage could not say. Distinct from an empty queue, which is good news.
    pub unavailable: bool,
    /// This page, which the queue refreshes itself from.
    pub live_href: String,
}

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub id: i64,
    /// `"Q14 · 254 · Red"`.
    pub heading: String,
    pub scout: String,
    /// `"5 minutes ago"`.
    pub ago: String,
    /// Every free-text field blank: the retired queue's yellow flag.
    pub missing_notes: bool,
    /// The detail page.
    pub href: String,
}

/// One observation in full, for the lead scout to approve or decline.
#[derive(Template)]
#[template(path = "pages/review.html")]
pub struct ReviewPage {
    pub title: String,
    pub nav: Nav,
    pub id: i64,
    /// `"Q14 · Team 254 · Red"`.
    pub heading: String,
    pub team_name: String,
    /// `"Sam, 5 minutes ago"`.
    pub byline: String,
    pub state: &'static str,
    pub state_label: &'static str,
    /// `"Declined by Kim, 2 minutes ago: that was 1678"`. Empty while pending.
    pub verdict: String,
    pub answers: Vec<tt_core::review::AnswerGroup>,
    /// Said in place of notes held back from this viewer (U13): whose they
    /// are, e.g. "Only scouts on team 10101 can read these notes."
    pub hidden_notes: String,
    pub missing_notes: bool,
    /// Saved on a different version of the form than the one in use.
    pub other_version: String,
    pub pending: bool,
    /// The verdict on the previous observation, when "and see the next" led
    /// here.
    pub notice: String,
    pub errors: Vec<String>,
    /// A decline reason coming back after a refused decline.
    pub reason: String,
    /// Waiting after this one, for "Approve" to say where it goes next.
    pub waiting: usize,
    pub back_href: String,
}

/// A scout's record that the lead scout declined, and how to answer it (L10).
#[derive(Debug, Clone)]
pub struct DeclinedNotice {
    /// `"team 254 in Q2"`.
    pub what: String,
    pub reason: String,
    pub reviewer: String,
    /// Records the robot again.
    pub href: String,
}

/// How much of one event the server holds.
#[derive(Debug, Clone)]
pub struct StoredCounts {
    pub event_name: String,
    pub teams: usize,
    pub matches: usize,
    pub played: usize,
}

/// The "FIRST and TBA data" card: what the server knows about its upstream
/// feeds, and the button that refreshes them.
#[derive(Debug, Clone, Default)]
pub struct UpstreamPanel {
    /// The server's uplink, e.g. "No internet", and its badge class.
    pub uplink_label: &'static str,
    pub uplink_class: &'static str,
    /// "3 minutes ago", or "never".
    pub last_sync: String,
    /// Synced, but long enough ago that the data should not be trusted as live.
    pub stale: bool,
    pub first_configured: bool,
    pub tba_configured: bool,
    /// The outcome of a sync requested from this page. Empty on a plain visit.
    pub result_headline: String,
    pub result_ok: bool,
    pub result_problems: Vec<String>,
}

// ── Assignments (L1) ────────────────────────────────────────────────────────

/// The lead scout's assignment grid: every match, every robot, and who is
/// watching it -- and the tools that change it (L2).
#[derive(Template)]
#[template(path = "pages/assignments.html")]
pub struct AssignmentsPage {
    pub title: String,
    pub nav: Nav,
    /// Empty when no event is selected.
    pub event_name: String,
    /// Why there is no grid, when there is not. Replaces the grid.
    pub unavailable: String,
    pub errors: Vec<String>,
    /// What the change that led here did.
    pub notice: String,
    pub grid: Option<AssignmentGrid>,
    /// One match open for editing (`?edit=`).
    pub editor: Option<MatchEditor>,
    /// Who auto-distribute can hand robots to.
    pub pool: Vec<PoolEntry>,
    pub devices: Vec<DeviceRow>,
    /// How each assignee is doing (L6).
    pub coverage: Vec<TallyRow>,
    /// The grid's own address, which its live regions refresh from.
    pub live_href: String,
}

/// Someone who can be handed a robot, as a form offers them.
#[derive(Debug, Clone)]
pub struct AssigneeChoice {
    pub key: AssigneeKey,
    /// `"Sam"`, or `"Stands Left (tablet)"`.
    pub label: String,
}

/// A checkbox in auto-distribute's "who is scouting" list.
#[derive(Debug, Clone)]
pub struct PoolEntry {
    /// `u:3` or `d:2`.
    pub value: String,
    pub label: String,
    pub by_device: bool,
    pub online: bool,
    /// Ticked when the page opens.
    pub checked: bool,
}

/// A tablet in the list a lead scout names them from.
#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub id: i64,
    /// The stored name, for the rename field. Empty when unnamed.
    pub name: String,
    /// What the grid calls it.
    pub display: String,
    pub online: bool,
    /// `"just now"`, `"2 hours ago"`, or `"never"`.
    pub last_seen: String,
    /// Who was signed in on it last. Empty when nobody was.
    pub last_user: String,
}

/// The six selects for one match (L2).
#[derive(Debug, Clone)]
pub struct MatchEditor {
    pub match_key: String,
    /// `"Q14"`.
    pub label: String,
    pub played: bool,
    pub slots: Vec<EditorSlot>,
    /// The next match, for "Save and edit next". `None` on the last one.
    pub next_label: String,
    pub has_next: bool,
    /// Where Cancel goes: back to this row of the grid.
    pub close_href: String,
}

#[derive(Debug, Clone)]
pub struct EditorSlot {
    pub color: &'static str,
    pub station: String,
    /// Empty where the schedule has no team: nothing to assign.
    pub team: String,
    pub team_name: String,
    /// The select's name: `a.254`.
    pub field: String,
    pub options: Vec<EditorOption>,
}

#[derive(Debug, Clone)]
pub struct EditorOption {
    /// Empty for "Unassigned".
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// The grid page's address, optionally opened on a match's editor.
///
/// Not percent-encoded: event and match keys are TBA keys from the database.
pub fn assignments_href(event_key: &str, edit: Option<&str>) -> String {
    match edit {
        Some(key) => format!("/lead-scout/assignments?event={event_key}&edit={key}#edit"),
        None => format!("/lead-scout/assignments?event={event_key}"),
    }
}

impl MatchEditor {
    /// `chosen` overrides what is stored, team by team: a form coming back
    /// after a refused save shows what was picked, not what was there.
    pub fn new(
        event_key: &str,
        record: &MatchRecord,
        next: Option<&MatchRecord>,
        roster: &[Team],
        assignments: &[Assignment],
        choices: &[AssigneeChoice],
        chosen: Option<&[(i32, Option<AssigneeKey>)]>,
    ) -> Self {
        let side = |color: &'static str, station: &str, teams: &[Option<i32>; 3]| {
            teams
                .iter()
                .enumerate()
                .map(|(i, slot)| {
                    let current = slot.and_then(|n| match chosen {
                        Some(chosen) => chosen.iter().find(|(t, _)| *t == n).and_then(|c| c.1),
                        None => assignments
                            .iter()
                            .find(|a| a.match_key == record.key && a.team_number == n)
                            .map(|a| a.assignee.key()),
                    });
                    let mut options = vec![EditorOption {
                        value: String::new(),
                        label: "Unassigned".into(),
                        selected: current.is_none(),
                    }];
                    options.extend(choices.iter().map(|c| EditorOption {
                        value: c.key.to_string(),
                        label: c.label.clone(),
                        selected: current == Some(c.key),
                    }));
                    EditorSlot {
                        color,
                        station: format!("{station} {}", i + 1),
                        team: slot.map(|n| n.to_string()).unwrap_or_default(),
                        team_name: slot
                            .and_then(|n| roster.iter().find(|t| t.number == n))
                            .map(|t| t.name.clone())
                            .unwrap_or_default(),
                        field: slot.map(|n| format!("a.{n}")).unwrap_or_default(),
                        options,
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut slots = side("red", "Red", &record.red);
        slots.extend(side("blue", "Blue", &record.blue));

        Self {
            match_key: record.key.clone(),
            label: record.label(),
            played: record.played,
            slots,
            next_label: next.map(|m| m.label()).unwrap_or_default(),
            has_next: next.is_some(),
            close_href: format!("{}#{}", assignments_href(event_key, None), record.key),
        }
    }
}

/// Matches down, the six driver stations across.
///
/// Upcoming matches first, since those are the ones a lead scout can still do
/// something about; played ones follow, folded away.
#[derive(Debug, Clone)]
pub struct AssignmentGrid {
    pub upcoming: Vec<GridRow>,
    pub played: Vec<GridRow>,
    /// Robots in upcoming matches that have an assignee.
    pub assigned: usize,
    /// Robots in upcoming matches. Empty slots are not counted: there is
    /// nobody to watch.
    pub assignable: usize,
    /// Assignments naming a robot no longer in its match, in words.
    pub stale: Vec<String>,
    /// Robots in played matches, and how many of them somebody recorded (L6).
    pub played_robots: usize,
    pub played_recorded: usize,
}

impl AssignmentGrid {
    /// Robots in played matches that nobody recorded.
    pub fn played_unrecorded(&self) -> usize {
        self.played_robots - self.played_recorded
    }
}

#[derive(Debug, Clone)]
pub struct GridRow {
    /// The match key, used as the row's id so a link can land on it.
    pub id: String,
    /// `"Q14"`.
    pub label: String,
    /// Opens this match's editor.
    pub edit_href: String,
    /// Red 1-3, then blue 1-3.
    pub cells: Vec<GridCell>,
}

/// One driver station in one match.
#[derive(Debug, Clone)]
pub struct GridCell {
    /// `"red"` or `"blue"`, for the CSS class.
    pub color: &'static str,
    /// `"Red 1"`.
    pub station: String,
    /// Empty where the schedule has no team yet, as in playoffs before
    /// alliance selection.
    pub team: String,
    /// Empty when the team is not on the event's synced roster. The number is
    /// what a scout needs; the name is a courtesy.
    pub team_name: String,
    /// Empty when nobody is assigned.
    pub assignee: String,
    /// The assignee is a tablet rather than a person.
    pub by_device: bool,
    /// Where the slot stands (L6): a CSS class, and words where there is news.
    pub state: &'static str,
    /// `"Recorded"`, `"Recorded ×2"`, `"Missed"`, `"Not scouted"`, or empty.
    pub state_label: String,
}

impl GridCell {
    /// A robot is here, still to play, and nobody is watching it.
    pub fn is_open(&self) -> bool {
        self.state == "open"
    }
}

/// One line of the coverage table: how an assignee is doing (L6).
#[derive(Debug, Clone)]
pub struct TallyRow {
    pub name: String,
    pub by_device: bool,
    pub online: bool,
    pub recorded: usize,
    pub missed: usize,
    pub to_come: usize,
}

fn state_words(state: SlotState) -> (&'static str, String) {
    match state {
        SlotState::Empty => ("empty", String::new()),
        SlotState::Recorded(1) => ("recorded", "Recorded".into()),
        SlotState::Recorded(n) => ("recorded", format!("Recorded ×{n}")),
        SlotState::Missed => ("missed", "Missed".into()),
        SlotState::Unscouted => ("unscouted", "Not scouted".into()),
        SlotState::Assigned => ("assigned", String::new()),
        SlotState::Open => ("open", String::new()),
    }
}

impl AssignmentGrid {
    /// `matches` in playing order. Names come from `roster`, the event's teams.
    pub fn new(
        event_key: &str,
        matches: &[MatchRecord],
        roster: &[Team],
        assignments: &[Assignment],
        sightings: &[Sighting],
    ) -> Self {
        let name_of = |number: i32| {
            roster
                .iter()
                .find(|t| t.number == number)
                .map(|t| t.name.clone())
                .unwrap_or_default()
        };
        let assignee_of = |match_key: &str, number: i32| {
            assignments
                .iter()
                .find(|a| a.match_key == match_key && a.team_number == number)
                .map(|a| &a.assignee)
        };

        let row = |m: &MatchRecord| {
            let side = |color: &'static str, station: &str, teams: &[Option<i32>; 3]| {
                teams
                    .iter()
                    .enumerate()
                    .map(|(i, slot)| {
                        let assignee = slot.and_then(|n| assignee_of(&m.key, n));
                        let (state, state_label) = state_words(assignments::slot_state(
                            m,
                            *slot,
                            assignee.is_some(),
                            sightings,
                        ));
                        GridCell {
                            color,
                            station: format!("{station} {}", i + 1),
                            team: slot.map(|n| n.to_string()).unwrap_or_default(),
                            team_name: slot.map(name_of).unwrap_or_default(),
                            assignee: assignee.map(|a| a.name().to_string()).unwrap_or_default(),
                            by_device: assignee.is_some_and(|a| a.is_device()),
                            state,
                            state_label,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let mut cells = side("red", "Red", &m.red);
            cells.extend(side("blue", "Blue", &m.blue));
            GridRow {
                id: m.key.clone(),
                label: m.label(),
                edit_href: assignments_href(event_key, Some(&m.key)),
                cells,
            }
        };

        let (played, upcoming): (Vec<_>, Vec<_>) = matches.iter().partition(|m| m.played);
        let upcoming: Vec<GridRow> = upcoming.into_iter().map(row).collect();
        let played: Vec<GridRow> = played.into_iter().map(row).collect();
        let cells = || upcoming.iter().flat_map(|r| &r.cells);
        let played_cells = || played.iter().flat_map(|r| &r.cells);

        Self {
            played_robots: played_cells().filter(|c| !c.team.is_empty()).count(),
            played_recorded: played_cells().filter(|c| c.state == "recorded").count(),
            assigned: cells().filter(|c| !c.assignee.is_empty()).count(),
            assignable: cells().filter(|c| !c.team.is_empty()).count(),
            stale: assignments::stale(matches, assignments)
                .into_iter()
                .filter_map(|a| {
                    let label = matches.iter().find(|m| m.key == a.match_key)?.label();
                    Some(format!(
                        "{label}: team {} is assigned to {}, but is no longer in that match.",
                        a.team_number,
                        a.assignee.name()
                    ))
                })
                .collect(),
            played,
            upcoming,
        }
    }
}

// ── Scouting (U4) ───────────────────────────────────────────────────────────

/// The scouting page: pick a match, pick a robot, record what it did.
#[derive(Template)]
#[template(path = "pages/submission.html")]
pub struct SubmissionPage {
    pub title: String,
    pub nav: Nav,
    /// Why there is nothing to scout, when there is not. Replaces the picker.
    pub unavailable: String,
    pub picker: Option<MatchPicker>,
    /// Confirmation of the save that led here.
    pub saved: String,
    pub errors: Vec<String>,
    /// Something to know that is not a failure.
    pub notice: String,
    pub form: Option<ScoutForm>,
    /// Following an assignment (L5): the robot is already chosen, and this
    /// card stands in for the picker. Leaving it is a deliberate tap.
    pub assigned: Option<AssignedCard>,
    /// The scout chose a robot other than the one assigned to them in this
    /// match. Allowed -- they are the one watching -- but said out loud.
    pub off_assignment: String,
    /// The scout's next assignment, when it is not what the page shows.
    pub next_duty: Option<MatchLink>,
    /// Assigned robots in played matches the scout has not recorded.
    pub missed: Vec<MatchLink>,
    /// The escape hatch (L3): type a team number instead of finding it.
    pub keypad: Option<Keypad>,
    /// Records of this scout's the lead scout declined, not yet redone (L10).
    pub declined: Vec<DeclinedNotice>,
}

/// "You are scouting 1678 · Q34 · Red 2".
#[derive(Debug, Clone)]
pub struct AssignedCard {
    pub team: i32,
    pub team_name: String,
    /// `"Q34 · Red 2"`.
    pub where_: String,
    /// The same match with the robot picker open.
    pub choose_href: String,
}

/// A team-number field. The event's roster feeds a `<datalist>`, which gives
/// type-ahead -- `16` offers `166`, `1619`, `1678` -- with no script at all.
#[derive(Debug, Clone)]
pub struct Keypad {
    pub event_key: String,
    pub roster: Vec<RosterEntry>,
}

/// Choosing the match and the robot.
///
/// A robot is picked from the six in the match, never from the event's list of
/// fifty: the wrong-robot entries the retired app suffered came from exactly
/// that list. Assignments (L3-L5) will preselect a robot here rather than
/// replace the picker.
#[derive(Debug, Clone)]
pub struct MatchPicker {
    pub event_key: String,
    pub options: Vec<MatchOption>,
    /// `"Q14"`.
    pub label: String,
    pub played: bool,
    /// Red, then blue.
    pub alliances: Vec<AllianceSlots>,
    pub previous: Option<MatchLink>,
    pub next: Option<MatchLink>,
}

#[derive(Debug, Clone)]
pub struct MatchOption {
    pub key: String,
    /// `"Q14"`, or `"Q14 · played"`.
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct MatchLink {
    pub href: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct AllianceSlots {
    /// `"red"` or `"blue"`: a CSS class and the accessible name.
    pub color: &'static str,
    pub slots: Vec<RobotSlot>,
}

/// One of the six driver-station positions.
#[derive(Debug, Clone)]
pub struct RobotSlot {
    /// `"Red 1"`.
    pub station: String,
    /// Empty where the schedule has no team in this slot.
    pub team: String,
    /// Empty where there is nothing to choose.
    pub href: String,
    pub selected: bool,
    /// This scout has already recorded this robot in this match.
    pub recorded: bool,
    /// This robot is assigned to this scout.
    pub assigned: bool,
}

/// Link to the scouting page for a match, and optionally a robot in it.
///
/// Not percent-encoded: event and match keys are TBA keys, alphanumeric plus
/// underscores, and only ever taken from the database.
pub fn scout_href(event_key: &str, match_key: &str, team: Option<i32>) -> String {
    let mut href = format!("/submission?event={event_key}&match={match_key}");
    if let Some(team) = team {
        href.push_str(&format!("&team={team}"));
    }
    href
}

/// The robot picker for a match, even where the scout has an assignment in it.
pub fn choose_href(event_key: &str, match_key: &str) -> String {
    format!("/submission?event={event_key}&match={match_key}&choose=1")
}

impl MatchPicker {
    /// `matches` in playing order, with `index` the one shown.
    pub fn new(
        event_key: &str,
        matches: &[MatchRecord],
        index: usize,
        team: Option<i32>,
        recorded: &[i32],
        assigned: Option<i32>,
    ) -> Self {
        let shown = &matches[index];
        let link = |m: &MatchRecord| MatchLink {
            href: scout_href(event_key, &m.key, None),
            label: m.label(),
        };
        let alliance =
            |color: &'static str, station: &str, teams: &[Option<i32>; 3]| AllianceSlots {
                color,
                slots: teams
                    .iter()
                    .enumerate()
                    .map(|(i, slot)| RobotSlot {
                        station: format!("{station} {}", i + 1),
                        team: slot.map(|n| n.to_string()).unwrap_or_default(),
                        href: slot
                            .map(|n| scout_href(event_key, &shown.key, Some(n)))
                            .unwrap_or_default(),
                        selected: slot.is_some() && *slot == team,
                        recorded: slot.is_some_and(|n| recorded.contains(&n)),
                        assigned: slot.is_some() && *slot == assigned,
                    })
                    .collect(),
            };

        Self {
            event_key: event_key.to_string(),
            options: matches
                .iter()
                .enumerate()
                .map(|(i, m)| MatchOption {
                    key: m.key.clone(),
                    label: if m.played {
                        format!("{} · played", m.label())
                    } else {
                        m.label()
                    },
                    selected: i == index,
                })
                .collect(),
            label: shown.label(),
            played: shown.played,
            alliances: vec![
                alliance("red", "Red", &shown.red),
                alliance("blue", "Blue", &shown.blue),
            ],
            previous: index.checked_sub(1).map(|i| link(&matches[i])),
            next: matches.get(index + 1).map(link),
        }
    }
}

/// A scouting form, rendered from the season schema rather than written per
/// season. Next January is a new `seasons/*.json`, not a new template.
#[derive(Debug, Clone)]
pub struct ScoutForm {
    pub match_key: String,
    pub team_number: i32,
    /// Idempotency key for the save (D7). Kept across a failed save, so a
    /// retry cannot store the observation twice.
    pub record_id: String,
    /// `"Q14 · Team 254 · Red 2"`.
    pub heading: String,
    /// The team's name, when the roster has it.
    pub team_name: String,
    pub sections: Vec<FormSection>,
    /// Problems not tied to one field.
    pub errors: Vec<String>,
    /// Any problem at all, so the page can say the save did not happen.
    pub has_errors: bool,
    /// The scout has no team, so their notes will be readable by nobody.
    pub notes_unshared: bool,
}

/// What a form holds between being shown and being saved.
#[derive(Debug, Clone)]
pub struct Draft {
    pub record_id: String,
    pub answers: RawAnswers,
    pub errors: FormErrors,
}

impl Draft {
    /// An untouched form.
    pub fn fresh(record_id: String) -> Self {
        Self {
            record_id,
            answers: RawAnswers::default(),
            errors: FormErrors::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FormSection {
    pub label: String,
    pub fields: Vec<FormField>,
}

#[derive(Debug, Clone)]
pub struct FormField {
    /// The input's `name`: `f.auto_scored`.
    pub name: String,
    /// The input's `id`: `f-auto_scored`.
    pub id: String,
    pub label: String,
    pub help: String,
    pub required: bool,
    pub error: String,
    pub input: FormInput,
}

/// How a field is drawn. Values are strings as typed, so a re-rendered form
/// shows exactly what failed.
#[derive(Debug, Clone)]
pub enum FormInput {
    /// A row of large buttons, one per option. Faster to tap than a dropdown.
    Choice {
        options: Vec<FormOption>,
    },
    /// A number between large − and + buttons.
    Counter {
        min: i64,
        max: i64,
        value: String,
    },
    Toggle {
        checked: bool,
    },
    Text {
        max_len: usize,
        value: String,
    },
}

#[derive(Debug, Clone)]
pub struct FormOption {
    pub value: String,
    pub label: String,
    pub checked: bool,
}

impl ScoutForm {
    pub fn new(
        schema: &SeasonSchema,
        record: &MatchRecord,
        team_number: i32,
        team_name: Option<&str>,
        draft: Draft,
        notes_unshared: bool,
    ) -> Self {
        let station = [("Red", &record.red), ("Blue", &record.blue)]
            .into_iter()
            .find_map(|(alliance, slots)| {
                let i = slots.iter().position(|s| *s == Some(team_number))?;
                Some(format!(" · {alliance} {}", i + 1))
            })
            .unwrap_or_default();

        Self {
            match_key: record.key.clone(),
            team_number,
            record_id: draft.record_id,
            heading: format!("{} · Team {team_number}{station}", record.label()),
            team_name: team_name.unwrap_or_default().to_string(),
            sections: form_sections(schema, &draft.answers, &draft.errors),
            has_errors: !draft.errors.is_empty(),
            errors: draft.errors.form,
            notes_unshared,
        }
    }
}

/// The schema's sections and fields, filled in with any answers so far.
///
/// With no answer yet a counter shows its minimum, so leaving it alone records
/// a real zero; everything else starts empty.
pub fn form_sections(
    schema: &SeasonSchema,
    answers: &RawAnswers,
    errors: &FormErrors,
) -> Vec<FormSection> {
    schema
        .sections
        .iter()
        .map(|section| FormSection {
            label: section.label.clone(),
            fields: section
                .fields
                .iter()
                .map(|field| {
                    let answer = answers.get(&field.key);
                    FormField {
                        name: input_name(&field.key),
                        id: format!("f-{}", field.key),
                        label: field.label.clone(),
                        help: field.help.clone().unwrap_or_default(),
                        required: field.required,
                        error: errors.field(&field.key).unwrap_or_default().to_string(),
                        input: match &field.kind {
                            FieldKind::Select { options } => FormInput::Choice {
                                options: options
                                    .iter()
                                    .map(|o| FormOption {
                                        value: o.key.clone(),
                                        label: o.label.clone(),
                                        checked: answer.map(str::trim) == Some(o.key.as_str()),
                                    })
                                    .collect(),
                            },
                            FieldKind::Counter { min, max, .. } => FormInput::Counter {
                                min: *min,
                                max: *max,
                                value: answer.map_or_else(|| min.to_string(), str::to_string),
                            },
                            FieldKind::Toggle { .. } => FormInput::Toggle {
                                checked: answer.is_some_and(is_on),
                            },
                            FieldKind::Text { max_len } => FormInput::Text {
                                max_len: *max_len,
                                value: answer.unwrap_or_default().to_string(),
                            },
                        },
                    }
                })
                .collect(),
        })
        .collect()
}

#[derive(Template)]
#[template(path = "pages/account.html")]
pub struct AccountPage {
    pub title: String,
    pub nav: Nav,
    pub user_name: String,
    pub user_email: String,
    pub team_display: String,
    pub role_labels: Vec<&'static str>,
    pub error: String,
    pub success: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::user::Roles;

    fn user(name: &str, roles: Roles) -> User {
        User {
            id: 1,
            email: "scout@example.com".into(),
            name: name.into(),
            team_number: Some(10101),
            roles,
        }
    }

    fn nav(roles: Roles) -> Nav {
        Nav::for_user(Some(&user("Sam", roles)), true)
    }

    #[test]
    fn renders_ready_state_with_schema_version() {
        let html = HealthPage {
            storage_ready: true,
            schema_version: Some(3),
        }
        .render_html()
        .expect("render");
        assert!(html.contains("ready"));
        assert!(html.contains("v3"));
    }

    #[test]
    fn renders_degraded_state_when_storage_is_down() {
        // The degraded path is the one that matters: this page has to render at
        // all when the database is unreachable.
        let html = HealthPage {
            storage_ready: false,
            schema_version: None,
        }
        .render_html()
        .expect("render");
        assert!(html.contains("unavailable"));
        assert!(html.contains("not yet migrated"));
    }

    #[test]
    fn nav_hides_privileged_links_from_a_plain_scout() {
        let html = HomePage {
            title: "Home".into(),
            nav: nav(Roles::SCOUT),
            team_display: "10101".into(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
            event: EventPanel::default(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("/submission"));
        assert!(!html.contains("/lead-scout"));
        assert!(!html.contains("/drive-coach"));
    }

    #[test]
    fn nav_shows_lead_and_coach_links_to_an_admin() {
        let html = HomePage {
            title: "Home".into(),
            nav: nav(Roles {
                is_admin: true,
                ..Roles::SCOUT
            }),
            team_display: "10101".into(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
            event: EventPanel::default(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("/lead-scout"));
        assert!(html.contains("/drive-coach"));
    }

    #[test]
    fn anonymous_visitors_see_sign_in_not_scouting() {
        let html = HomePage {
            title: "Home".into(),
            nav: Nav::anonymous(true),
            team_display: String::new(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
            event: EventPanel::default(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("/sign-in"));
        assert!(html.contains("/sign-up"));
        assert!(!html.contains("/submission"));
    }

    #[test]
    fn a_degraded_footer_warns_that_nothing_is_being_saved() {
        let html = HomePage {
            title: "Home".into(),
            nav: Nav::anonymous(false),
            team_display: String::new(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
            event: EventPanel::default(),
        }
        .render_html()
        .expect("render");
        assert!(html.contains("Storage unavailable"));
    }

    // ── Event switcher and summary (U2, U3) ─────────────────────────────────

    fn event(key: &str, name: &str) -> Event {
        Event {
            key: key.into(),
            name: name.into(),
            location: Some("Boston, MA".into()),
            timezone: None,
            start_date: chrono_date(2026, 3, 12),
            end_date: chrono_date(2026, 3, 15),
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        }
    }

    fn chrono_date(y: i32, m: u32, d: u32) -> Option<chrono::NaiveDate> {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
    }

    fn team(number: i32, name: &str) -> Team {
        Team {
            number,
            name: name.into(),
            nickname: None,
            school: None,
            city: None,
            state: None,
            country: None,
            rookie_year: None,
            website: None,
        }
    }

    fn home(nav: Nav, event: EventPanel) -> String {
        HomePage {
            title: "Home".into(),
            nav,
            team_display: String::new(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
            event,
        }
        .render_html()
        .expect("render")
    }

    #[test]
    fn the_switcher_marks_the_selected_event_and_carries_it_in_links() {
        let events = [
            event("2026mabil", "Boston"),
            event("2026nhgrs", "Granite State"),
        ];
        let switcher = EventSwitcher::new(&events, Some(&events[1]));

        assert_eq!(switcher.options[0].label, "Boston · Mar 12–15");
        assert!(!switcher.options[0].selected);
        assert!(switcher.options[1].selected);
        assert_eq!(switcher.query(), "?event=2026nhgrs");
        assert_eq!(EventSwitcher::new(&events, None).query(), "");

        let mut nav = nav(Roles {
            is_admin: true,
            ..Roles::SCOUT
        });
        nav.event = switcher;
        let html = home(nav, EventPanel::default());
        assert!(
            html.contains(r#"<option value="2026nhgrs" selected>"#),
            "{html}"
        );
        for link in [
            "/?event=2026nhgrs",
            "/submission?event=2026nhgrs",
            "/lead-scout?event=2026nhgrs",
        ] {
            assert!(html.contains(&format!(r#"href="{link}""#)), "{link}");
        }
    }

    #[test]
    fn with_no_events_there_is_no_switcher() {
        let html = home(nav(Roles::SCOUT), EventPanel::default());
        assert!(
            !html.contains("event-switcher\""),
            "no empty select in the header"
        );
        assert!(
            html.contains(r#"href="/submission""#),
            "and links carry nothing"
        );
    }

    #[test]
    fn a_summary_counts_teams_and_matches_and_finds_the_viewer() {
        let roster = [team(254, "Cheesy Poofs"), team(10101, "Teal Team")];
        let summary = EventSummary::new(&event("2026mabil", "Boston"), &roster, &[], Some(10101));

        assert_eq!(summary.dates, "Mar 12–15, 2026");
        assert_eq!(summary.team_count, 2);
        assert!(!summary.team_missing);
        assert!(summary.roster[1].is_viewer && !summary.roster[0].is_viewer);
    }

    #[test]
    fn a_team_absent_from_the_roster_is_told_so() {
        let roster = [team(254, "Cheesy Poofs")];
        let boston = event("2026mabil", "Boston");
        let summary = EventSummary::new(&boston, &roster, &[], Some(10101));
        assert!(summary.team_missing);

        let html = home(
            nav(Roles::SCOUT),
            EventPanel {
                summary: Some(summary),
                ..EventPanel::default()
            },
        );
        assert!(html.contains("Your team is not listed for this event yet."));

        // No team, nothing to be missing from.
        assert!(!EventSummary::new(&boston, &roster, &[], None).team_missing);
    }

    #[test]
    fn an_empty_database_says_how_to_fill_it() {
        let panel = EventPanel {
            none_loaded: true,
            ..EventPanel::default()
        };
        let lead = home(
            nav(Roles {
                is_lead_scout: true,
                ..Roles::SCOUT
            }),
            panel.clone(),
        );
        assert!(lead.contains("No events have been loaded yet."));
        assert!(lead.contains("/lead-scout#upstream"));

        let scout = home(nav(Roles::SCOUT), panel);
        assert!(scout.contains("A lead scout can load them."));
        assert!(!scout.contains("/lead-scout#upstream"));
    }

    // ── Scouting (U4) ───────────────────────────────────────────────────────

    fn scheduled(number: i32, played: bool) -> MatchRecord {
        MatchRecord {
            key: format!("2026mabil_qm{number}"),
            event_key: "2026mabil".into(),
            comp_level: tt_core::matches::CompLevel::Qualification,
            set_number: 1,
            match_number: number,
            red: [Some(10101), Some(254), None],
            blue: [Some(2), Some(3), Some(4)],
            red_score: None,
            blue_score: None,
            winner: None,
            played,
            scheduled_at: None,
            actual_at: None,
        }
    }

    fn season() -> SeasonSchema {
        tt_core::season::current_season().expect("shipped schema")
    }

    fn scouting_page(picker: Option<MatchPicker>, form: Option<ScoutForm>) -> String {
        SubmissionPage {
            title: "Scout".into(),
            nav: nav(Roles::SCOUT),
            unavailable: String::new(),
            picker,
            saved: String::new(),
            errors: Vec::new(),
            notice: String::new(),
            form,
            assigned: None,
            off_assignment: String::new(),
            next_duty: None,
            missed: Vec::new(),
            keypad: None,
            declined: Vec::new(),
        }
        .render_html()
        .expect("render")
    }

    fn answers(pairs: &[(&str, &str)]) -> RawAnswers {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        RawAnswers::from_pairs(&pairs)
    }

    #[test]
    fn the_picker_offers_the_six_robots_of_the_chosen_match() {
        let matches = [scheduled(1, true), scheduled(2, false), scheduled(3, false)];
        let picker = MatchPicker::new("2026mabil", &matches, 1, Some(254), &[3], Some(2));

        assert_eq!(picker.label, "Q2");
        assert_eq!(picker.options[0].label, "Q1 · played");
        assert!(picker.options[1].selected);
        assert_eq!(picker.previous.as_ref().unwrap().label, "Q1");
        assert_eq!(picker.next.as_ref().unwrap().label, "Q3");

        let red = &picker.alliances[0].slots;
        assert_eq!(red[1].station, "Red 2");
        assert!(red[1].selected);
        assert_eq!(
            red[1].href,
            "/submission?event=2026mabil&match=2026mabil_qm2&team=254"
        );
        assert!(
            red[2].team.is_empty() && red[2].href.is_empty(),
            "an empty slot"
        );
        assert!(picker.alliances[1].slots[1].recorded, "blue 2 is team 3");
        assert!(
            picker.alliances[1].slots[0].assigned,
            "blue 1 is team 2, this scout's"
        );

        let html = scouting_page(Some(picker), None);
        assert!(html.contains(r#"<ul class="alliance red""#));
        assert!(html.contains(r#"<option value="2026mabil_qm2" selected>"#));
        assert!(html.contains("Recorded"));
    }

    #[test]
    fn the_first_and_last_matches_have_no_link_past_the_end() {
        let matches = [scheduled(1, false), scheduled(2, false)];
        assert!(
            MatchPicker::new("e", &matches, 0, None, &[], None)
                .previous
                .is_none()
        );
        assert!(
            MatchPicker::new("e", &matches, 1, None, &[], None)
                .next
                .is_none()
        );
    }

    #[test]
    fn a_fresh_form_is_drawn_entirely_from_the_schema() {
        let schema = season();
        let form = ScoutForm::new(
            &schema,
            &scheduled(2, false),
            254,
            Some("Cheesy Poofs"),
            Draft::fresh("0191f7ac-1234-7000-8000-000000000001".into()),
            false,
        );
        assert_eq!(form.heading, "Q2 · Team 254 · Red 2");
        assert!(!form.has_errors);

        let html = scouting_page(None, Some(form));
        // Every field in the schema, by its prefixed name, and no other.
        for field in schema.fields() {
            assert!(
                html.contains(&format!(r#"name="f.{}""#, field.key)),
                "{}",
                field.key
            );
        }
        assert_eq!(
            html.matches(r#"name="f."#).count(),
            schema
                .fields()
                .map(|f| match &f.kind {
                    FieldKind::Select { options } => options.len(),
                    _ => 1,
                })
                .sum::<usize>()
        );
        assert!(html.contains("Cheesy Poofs"));
        assert!(html.contains(r#"value="0191f7ac-1234-7000-8000-000000000001""#));
        // Counters start at their minimum, so an untouched one is a real zero.
        assert!(html.contains(r#"name="f.auto_scored" value="0""#));
        assert!(!html.contains(" checked"), "nothing is preselected");
    }

    #[test]
    fn a_returned_form_keeps_exactly_what_was_typed() {
        let mut errors = FormErrors::default();
        errors
            .fields
            .insert("teleop_scored".into(), "Must be between 0 and 60.".into());
        errors.form.push(tt_core::form::STALE_FORM.into());
        let draft = Draft {
            record_id: "0191f7ac-1234-7000-8000-000000000001".into(),
            answers: answers(&[
                ("f.starting_position", "center"),
                ("f.teleop_scored", "61"),
                ("f.broke_down", "on"),
                ("f.notes", "<b>tippy</b>"),
            ]),
            errors,
        };
        let form = ScoutForm::new(&season(), &scheduled(2, false), 3, None, draft, false);
        assert!(form.has_errors);

        let html = scouting_page(None, Some(form));
        assert!(html.contains(r#"value="center" checked"#));
        assert!(
            html.contains(r#"value="61""#),
            "the mistake itself, so it can be fixed"
        );
        assert!(html.contains("Must be between 0 and 60."));
        assert!(html.contains("Not saved yet."));
        assert!(html.contains("different version of the form"));
        assert!(html.contains(r#"name="f.broke_down" value="on" checked"#));
        assert!(
            html.contains("&#60;b&#62;tippy&#60;/b&#62;</textarea>"),
            "escaped"
        );
    }

    #[test]
    fn a_scout_with_no_team_is_told_their_notes_go_nowhere() {
        let fresh = || Draft::fresh("0191f7ac-1234-7000-8000-000000000001".into());
        let form = ScoutForm::new(&season(), &scheduled(2, false), 3, None, fresh(), true);
        assert!(scouting_page(None, Some(form)).contains("no team will be able to read the notes"));

        let form = ScoutForm::new(&season(), &scheduled(2, false), 3, None, fresh(), false);
        assert!(!scouting_page(None, Some(form)).contains("no team will be able to read"));
    }

    #[test]
    fn counter_buttons_are_hidden_until_the_script_shows_them() {
        // Without JavaScript they would be buttons that do nothing.
        let form = ScoutForm::new(
            &season(),
            &scheduled(2, false),
            3,
            None,
            Draft::fresh("0191f7ac-1234-7000-8000-000000000001".into()),
            false,
        );
        let html = scouting_page(None, Some(form));
        assert!(html.contains(r#"data-step="1" aria-label="One more" hidden"#));
        assert!(html.contains("/static/js/counter.js"));
    }

    // ── Assignments (L1) ────────────────────────────────────────────────────

    fn assignment(
        match_number: i32,
        team_number: i32,
        assignee: assignments::Assignee,
    ) -> Assignment {
        Assignment {
            match_key: format!("2026mabil_qm{match_number}"),
            team_number,
            assignee,
        }
    }

    fn sam() -> assignments::Assignee {
        assignments::Assignee::Scout {
            id: 1,
            name: "Sam".into(),
        }
    }

    fn tablet() -> assignments::Assignee {
        assignments::Assignee::Device {
            id: 1,
            name: "Stands Left".into(),
        }
    }

    /// Q1 played, Q2 and Q3 to come. Each has 10101, 254, and a gap on red, and
    /// 2, 3, 4 on blue; only 10101 and 254 are on the roster.
    fn grid(assignments: &[Assignment]) -> AssignmentGrid {
        let matches = [scheduled(1, true), scheduled(2, false), scheduled(3, false)];
        let roster = [team(10101, "Teal Team"), team(254, "Cheesy Poofs")];
        AssignmentGrid::new("2026mabil", &matches, &roster, assignments, &[])
    }

    fn assignments_page(grid: Option<AssignmentGrid>, unavailable: &str) -> String {
        AssignmentsPage {
            title: "Assignments".into(),
            nav: nav(Roles {
                is_lead_scout: true,
                ..Roles::SCOUT
            }),
            event_name: "Boston".into(),
            unavailable: unavailable.into(),
            errors: Vec::new(),
            notice: String::new(),
            grid,
            editor: None,
            pool: Vec::new(),
            devices: Vec::new(),
            coverage: Vec::new(),
            live_href: "/lead-scout/assignments?event=2026mabil".into(),
        }
        .render_html()
        .expect("render")
    }

    #[test]
    fn the_grid_puts_upcoming_matches_first_and_folds_played_ones_away() {
        let grid = grid(&[]);
        let labels = |rows: &[GridRow]| rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&grid.upcoming), ["Q2", "Q3"]);
        assert_eq!(labels(&grid.played), ["Q1"]);

        let stations: Vec<_> = grid.upcoming[0]
            .cells
            .iter()
            .map(|c| c.station.as_str())
            .collect();
        assert_eq!(
            stations,
            ["Red 1", "Red 2", "Red 3", "Blue 1", "Blue 2", "Blue 3"]
        );

        let html = assignments_page(Some(grid), "");
        assert!(html.contains(r#"<tr role="row" id="2026mabil_qm2">"#));
        let played = html
            .find(r#"<details class="card" id="played">"#)
            .expect("played folded");
        assert!(html.find(r#"id="2026mabil_qm1""#).unwrap() > played);
        assert!(html.find(r#"id="2026mabil_qm2""#).unwrap() < played);
    }

    #[test]
    fn each_cell_names_the_robot_and_who_is_watching_it() {
        let grid = grid(&[assignment(2, 10101, sam()), assignment(2, 2, tablet())]);
        let q2 = &grid.upcoming[0].cells;

        assert_eq!(
            (q2[0].team.as_str(), q2[0].team_name.as_str()),
            ("10101", "Teal Team")
        );
        assert_eq!(q2[0].assignee, "Sam");
        assert!(!q2[0].by_device && !q2[0].is_open());

        assert_eq!(q2[3].assignee, "Stands Left");
        assert!(q2[3].by_device);
        assert!(q2[3].team_name.is_empty(), "team 2 is not on the roster");

        assert!(q2[1].is_open(), "254 has nobody");
        assert!(
            q2[2].team.is_empty() && !q2[2].is_open(),
            "a gap is not open"
        );

        let html = assignments_page(Some(grid), "");
        assert!(html.contains(r#"<td role="cell" class="slot red open">"#));
        assert!(html.contains("Sam"));
        assert!(html.contains(r#"Stands Left <span class="slot-kind">tablet</span>"#));
        assert!(html.contains(r#"<strong class="slot-team">TBD</strong>"#));
        assert!(html.contains(r#"<span class="slot-name unknown">not on roster</span>"#));
        assert!(html.contains("Unassigned"));
    }

    #[test]
    fn coverage_counts_only_upcoming_robots() {
        // One in a played match, which no longer needs a scout.
        let grid = grid(&[assignment(1, 254, sam()), assignment(3, 4, sam())]);
        assert_eq!((grid.assigned, grid.assignable), (1, 10));
        assert!(assignments_page(Some(grid), "").contains("<strong>1 of 10</strong> robots"));
    }

    #[test]
    fn an_assignment_the_schedule_moved_away_from_is_called_out() {
        let grid = grid(&[assignment(2, 99, sam())]);
        assert_eq!(
            grid.stale,
            ["Q2: team 99 is assigned to Sam, but is no longer in that match."]
        );
        assert_eq!(grid.assigned, 0, "and it covers nobody");
        assert!(assignments_page(Some(grid), "").contains("would watch the wrong robot"));
    }

    #[test]
    fn the_editor_offers_everyone_and_keeps_a_refused_pick() {
        let matches = [scheduled(1, false), scheduled(2, false)];
        let choices = [
            AssigneeChoice {
                key: AssigneeKey::Scout(1),
                label: "Sam".into(),
            },
            AssigneeChoice {
                key: AssigneeKey::Device(1),
                label: "Stands Left (tablet)".into(),
            },
        ];
        let stored = [assignment(1, 254, sam())];
        let roster = [team(254, "Cheesy Poofs")];

        let editor = MatchEditor::new(
            "2026mabil",
            &matches[0],
            Some(&matches[1]),
            &roster,
            &stored,
            &choices,
            None,
        );
        assert_eq!(
            (editor.label.as_str(), editor.next_label.as_str()),
            ("Q1", "Q2")
        );
        assert_eq!(
            editor.close_href,
            "/lead-scout/assignments?event=2026mabil#2026mabil_qm1"
        );
        let red2 = &editor.slots[1];
        assert_eq!(
            (red2.field.as_str(), red2.team_name.as_str()),
            ("a.254", "Cheesy Poofs")
        );
        let selected: Vec<_> = red2
            .options
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(selected, ["u:1"]);
        assert_eq!(red2.options.len(), 3, "unassigned, Sam, the tablet");
        assert!(
            editor.slots[2].field.is_empty(),
            "a gap has nothing to choose"
        );

        // Coming back from a refused save, the picks win over what is stored.
        let chosen = [(254, Some(AssigneeKey::Device(1)))];
        let editor = MatchEditor::new(
            "2026mabil",
            &matches[1],
            None,
            &roster,
            &stored,
            &choices,
            Some(&chosen),
        );
        assert!(!editor.has_next);
        let selected: Vec<_> = editor.slots[1]
            .options
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(selected, ["d:1"]);
        assert!(
            editor.slots[0].options[0].selected,
            "10101 was not picked: unassigned"
        );
    }

    #[test]
    fn played_robots_say_whether_anybody_recorded_them() {
        let matches = [scheduled(1, true), scheduled(2, false)];
        let sighting = |team: i32| Sighting {
            match_key: "2026mabil_qm1".into(),
            team_number: team,
            scouter_id: Some(1),
            device_id: None,
        };
        let grid = AssignmentGrid::new(
            "2026mabil",
            &matches,
            &[],
            &[assignment(1, 254, sam())],
            &[sighting(10101), sighting(10101)],
        );
        let q1 = &grid.played[0].cells;
        assert_eq!(
            (q1[0].state, q1[0].state_label.as_str()),
            ("recorded", "Recorded ×2")
        );
        assert_eq!(
            (q1[1].state, q1[1].state_label.as_str()),
            ("missed", "Missed")
        );
        assert_eq!(
            (q1[3].state, q1[3].state_label.as_str()),
            ("unscouted", "Not scouted")
        );
        assert_eq!(q1[2].state, "empty");
        assert_eq!((grid.played_robots, grid.played_recorded), (5, 1));
        assert_eq!(grid.played_unrecorded(), 4);

        let html = assignments_page(Some(grid), "");
        assert!(html.contains(r#"class="slot red missed""#));
        assert!(html.contains("1 of 5</strong> robots in played matches"));
        assert!(html.contains("Played matches (1) · 4 not scouted"));
    }

    #[test]
    fn the_coverage_table_lists_each_assignee() {
        let mut page = AssignmentsPage {
            title: "Assignments".into(),
            nav: nav(Roles::SCOUT),
            event_name: "Boston".into(),
            unavailable: String::new(),
            errors: Vec::new(),
            notice: String::new(),
            grid: Some(grid(&[])),
            editor: None,
            pool: Vec::new(),
            devices: Vec::new(),
            coverage: vec![TallyRow {
                name: "Sam".into(),
                by_device: false,
                online: true,
                recorded: 3,
                missed: 1,
                to_come: 4,
            }],
            live_href: "/lead-scout/assignments?event=2026mabil".into(),
        };
        let html = page.render_html().expect("render");
        assert!(
            html.contains(
                "<th scope=\"row\">Sam <span class=\"badge badge-teal\">online</span></th>"
            ),
            "{html}"
        );
        assert!(html.contains("<td class=\"missed\">1</td>"));

        page.coverage.clear();
        let html = page.render_html().expect("render");
        assert!(html.contains("Nobody has been assigned anything yet."));
    }

    #[test]
    fn with_no_grid_the_page_says_why() {
        let html = assignments_page(None, "Boston has no match schedule yet.");
        assert!(html.contains("Boston has no match schedule yet."));
        assert!(!html.contains("<table"));
        assert!(html.contains(r#"href="/lead-scout""#), "and the way back");
    }

    #[test]
    fn user_supplied_values_are_html_escaped() {
        // Askama escapes by default; this test is here so that a future switch
        // to a raw filter cannot silently open an injection.
        let html = SignInPage {
            title: "Sign in".into(),
            nav: Nav::anonymous(true),
            email: "\"><script>alert(1)</script>".into(),
            error: "<img src=x onerror=alert(1)>".into(),
        }
        .render_html()
        .expect("render");

        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img src=x"));
        // Askama emits numeric entities rather than named ones.
        assert!(html.contains("&#60;script&#62;"));
    }

    #[test]
    fn sign_in_preserves_the_email_across_a_failed_attempt() {
        let html = SignInPage {
            title: "Sign in".into(),
            nav: Nav::anonymous(true),
            email: "scout@example.com".into(),
            error: "Invalid email or password".into(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("scout@example.com"));
        assert!(html.contains("Invalid email or password"));
    }

    #[test]
    fn sign_up_announces_that_the_first_account_is_an_admin() {
        let html = SignUpPage {
            title: "Sign up".into(),
            nav: Nav::anonymous(true),
            name: String::new(),
            email: String::new(),
            team_number: String::new(),
            error: String::new(),
            first_account: true,
        }
        .render_html()
        .expect("render");
        assert!(html.contains("administrator"));
    }

    #[test]
    fn account_page_lists_every_role_badge() {
        let html = AccountPage {
            title: "Account".into(),
            nav: nav(Roles {
                is_admin: true,
                is_lead_scout: true,
                is_coach: true,
            }),
            user_name: "Sam".into(),
            user_email: "scout@example.com".into(),
            team_display: "10101".into(),
            role_labels: vec!["Admin", "Lead Scout", "Drive Coach"],
            error: String::new(),
            success: "Password changed".into(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("Admin"));
        assert!(html.contains("Lead Scout"));
        assert!(html.contains("Drive Coach"));
        assert!(html.contains("Password changed"));
    }

    // ── Error pages (U10) ───────────────────────────────────────────────────

    #[test]
    fn the_status_picks_the_wording_on_an_error_page() {
        assert_eq!(ErrorKind::for_status(404), ErrorKind::NotFound);
        assert_eq!(ErrorKind::for_status(405), ErrorKind::FormAddress);
        assert_eq!(ErrorKind::for_status(400), ErrorKind::Refused);
        assert_eq!(ErrorKind::for_status(422), ErrorKind::Refused);
        assert_eq!(ErrorKind::for_status(500), ErrorKind::ServerFault);
        assert_eq!(ErrorKind::for_status(503), ErrorKind::ServerFault);
    }

    #[test]
    fn an_error_page_says_what_happened_and_how_to_go_on() {
        for (status, heading, says) in [
            (404, "Page not found", "Nothing is at this address"),
            (405, "Nothing to show here", "Nothing was sent or changed"),
            (422, "That did not work", "reload that page and try again"),
            (500, "Something went wrong", "tell your lead scout"),
        ] {
            let html = ErrorPage::new(status, "/somewhere".into(), nav(Roles::SCOUT))
                .render_html()
                .expect("render");
            assert!(html.contains(&format!("<title>{heading} · TealTeam</title>")));
            assert!(html.contains(&format!("<h1>{heading}</h1>")), "{status}");
            assert!(html.contains(says), "{status}");
            assert!(html.contains(&format!("{status} · /somewhere")));
            assert!(html.contains(r#"<a class="btn btn-primary" href="/">"#));
            assert!(
                html.contains(r#"href="/submission""#),
                "the nav is there to go on from"
            );
        }
    }

    #[test]
    fn the_address_on_an_error_page_is_escaped() {
        let html = ErrorPage::new(404, "/<script>x</script>".into(), Nav::default())
            .render_html()
            .expect("render");
        assert!(!html.contains("<script>x"));
        assert!(html.contains("&#60;script&#62;x"));
    }

    #[test]
    fn a_failed_sync_is_announced_as_an_alert_and_a_good_one_as_a_status() {
        let page = |ok: bool| LeadScoutPage {
            title: "Lead Scout".into(),
            nav: nav(Roles::SCOUT),
            season_name: "Rebuilt".into(),
            upstream: UpstreamPanel {
                result_headline: "Sync finished".into(),
                result_ok: ok,
                ..UpstreamPanel::default()
            },
            stored: None,
            queue: None,
            reviewed: String::new(),
        };

        let failed = page(false).render_html().expect("render");
        assert!(failed.contains(r#"<div class="alert alert-error" role="alert">"#));

        let good = page(true).render_html().expect("render");
        assert!(good.contains(r#"<div class="alert alert-success" role="status">"#));
    }
}
