//! The state the screen is drawn from, and what each key does to it.
//!
//! One task owns this. Everything slow -- the database read, the status
//! probes, a command, a request to the server -- runs elsewhere and comes
//! back as a [`Msg`], so a sync that takes a minute never freezes the keys.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;
use tt_core::user::Roles;
use tt_repo_sqlite::SqliteRepo;

use crate::data::{self, Snapshot};
use crate::logs::{self, Level, Line, Ring, Subject};
use crate::server::{Failure, Report, SchedulePost, Server};
use crate::site::Site;
use crate::system::{self, Done, Status};

/// How often the database and the machine are asked again.
pub const REFRESH: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Logs,
    People,
    Data,
    Actions,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Logs,
        Tab::People,
        Tab::Data,
        Tab::Actions,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Logs => "Logs",
            Tab::People => "People",
            Tab::Data => "Data",
            Tab::Actions => "Actions",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn next(self) -> Tab {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Tab {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Everything the Actions tab offers, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Restart,
    Stop,
    Start,
    Backup,
    CheckBackup,
    SyncNow,
    FtcPull,
    ImportSchedule,
    SignIn,
    SignOut,
    Poweroff,
}

impl Action {
    pub fn list(site: &Site, running: bool, signed_in: bool) -> Vec<Action> {
        let mut out = Vec::new();
        if running {
            out.push(Action::Restart);
            out.push(Action::Stop);
        } else {
            out.push(Action::Start);
        }
        out.push(Action::Backup);
        out.push(Action::CheckBackup);
        out.push(Action::SyncNow);
        out.push(Action::FtcPull);
        out.push(Action::ImportSchedule);
        out.push(if signed_in {
            Action::SignOut
        } else {
            Action::SignIn
        });
        if site.is_systemd() {
            out.push(Action::Poweroff);
        }
        out
    }

    pub fn title(self) -> &'static str {
        match self {
            Action::Restart => "Restart the server",
            Action::Stop => "Stop the server",
            Action::Start => "Start the server",
            Action::Backup => "Back up now",
            Action::CheckBackup => "Check the newest backup restores",
            Action::SyncNow => "Sync now (FIRST and TBA, or the FTC pull on that build)",
            Action::FtcPull => "Pull the schedule from FTC Events",
            Action::ImportSchedule => "Import a schedule CSV",
            Action::SignIn => "Sign in as an admin (for sync, import and account changes)",
            Action::SignOut => "Forget the admin session",
            Action::Poweroff => "Power off this machine",
        }
    }

    pub fn detail(self, site: &Site) -> String {
        match self {
            Action::Restart => match site.is_systemd() {
                true => "sudo -n systemctl restart tealteam. Pages are back in about 5 seconds.".into(),
                false => "Stops the pid in server.pid and starts tt-web serve again, in the background.".into(),
            },
            Action::Stop => "Scouts' pages stop answering; devices keep scouting offline.".into(),
            Action::Start => "Starts the server the way its install does.".into(),
            Action::Backup => format!(
                "tt-web backup FOLDER: a snapshot into the folder, then a restore check. Default {}. Name the stick's folder to take a copy out of the building.",
                site.backup_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "(none)".into())
            ),
            Action::CheckBackup => "tt-web check-backup: restores the newest timed snapshot into a scratch database and says what it holds.".into(),
            Action::SyncNow => "POST /api/frc/sync as the admin. Needs internet; at the venue it will say no uplink.".into(),
            Action::FtcPull => "POST /api/ftc/pull as the admin. Needs FTC_EVENT_CODE and the FTC key on the server.".into(),
            Action::ImportSchedule => "The lead scout's import form, from a CSV file on this machine (match,red1,red2,blue1,blue2; * marks a surrogate).".into(),
            Action::SignIn => "Signs in over localhost like the lead's laptop. Nothing is stored.".into(),
            Action::SignOut => "Drops the session cookie this tool holds.".into(),
            Action::Poweroff => "sudo -n systemctl poweroff. Back up to the stick first. Type the word to confirm.".into(),
        }
    }

    fn needs_session(self) -> bool {
        matches!(
            self,
            Action::SyncNow | Action::FtcPull | Action::ImportSchedule
        )
    }
}

/// A pending change that needs the admin session.
#[derive(Debug, Clone)]
pub enum Pending {
    Action(Action),
    SaveAccount {
        id: i64,
        team: Option<i32>,
        roles: Roles,
    },
    ResetPassword {
        id: i64,
        password: String,
    },
    Import(SchedulePost),
}

/// A single-line text field.
#[derive(Debug, Clone, Default)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub secret: bool,
}

impl Field {
    fn new(label: &'static str, value: impl Into<String>) -> Field {
        Field {
            label,
            value: value.into(),
            secret: false,
        }
    }

    fn secret(label: &'static str) -> Field {
        Field {
            label,
            value: String::new(),
            secret: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Modal {
    /// Yes or no; `word` set means the word must be typed.
    Confirm {
        title: String,
        body: Vec<String>,
        word: Option<String>,
        typed: String,
        then: Pending,
    },
    /// Fields to fill, then Enter.
    Form {
        title: String,
        hint: String,
        fields: Vec<Field>,
        active: usize,
        kind: FormKind,
    },
    /// What something did.
    Result {
        title: String,
        ok: bool,
        lines: Vec<String>,
        scroll: u16,
    },
}

#[derive(Debug, Clone)]
pub enum FormKind {
    SignIn { then: Option<Pending> },
    Backup,
    CheckBackup,
    EditAccount { id: i64 },
    ResetPassword { id: i64 },
    Import,
}

/// What arrives from the other tasks and the keyboard.
#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize,
    Snapshot(Snapshot),
    Status(Status),
    Log(Line),
    LogSource {
        name: String,
        trouble: Option<String>,
    },
    Done(Done),
    Server(Result<Report, Failure>, Option<Pending>),
    SignedIn(Result<Server, Failure>, Option<Pending>),
    Quit,
}

pub struct App {
    pub site: Site,
    pub repo: Arc<SqliteRepo>,
    pub server: Server,
    tx: UnboundedSender<Msg>,
    pub tab: Tab,
    pub snapshot: Snapshot,
    pub status: Status,
    pub logs: Ring,
    pub log_level: Level,
    pub log_subject: Subject,
    pub log_follow: bool,
    /// Index into the filtered lines of the newest line shown.
    pub log_at: usize,
    pub people_at: usize,
    pub people_devices: bool,
    pub devices_at: usize,
    pub data_at: usize,
    pub actions_at: usize,
    pub modal: Option<Modal>,
    pub notice: Option<(String, bool, Instant)>,
    pub busy: Option<String>,
    refreshing: bool,
    probing: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(site: Site, repo: SqliteRepo, tx: UnboundedSender<Msg>) -> App {
        let server = Server::new(site.base_url());
        let mut logs = Ring::default();
        logs.source = logs::describe_source(&site);
        App {
            site,
            repo: Arc::new(repo),
            server,
            tx,
            tab: Tab::Overview,
            snapshot: Snapshot::default(),
            status: Status::default(),
            logs,
            log_level: Level::Info,
            log_subject: Subject::All,
            log_follow: true,
            log_at: 0,
            people_at: 0,
            people_devices: false,
            devices_at: 0,
            data_at: 0,
            actions_at: 0,
            modal: None,
            notice: None,
            busy: None,
            refreshing: false,
            probing: false,
            should_quit: false,
        }
    }

    pub fn actions(&self) -> Vec<Action> {
        Action::list(
            &self.site,
            self.status.process.running,
            self.server.session.is_some(),
        )
    }

    pub fn now_lines(&self) -> Vec<&Line> {
        self.logs
            .lines
            .iter()
            .filter(|l| l.shows(self.log_level, self.log_subject))
            .collect()
    }

    /// The event the tool acts on: the selected row of the Data tab.
    pub fn selected_event(&self) -> Option<&data::EventSummary> {
        self.snapshot.events.get(self.data_at)
    }

    fn say(&mut self, text: impl Into<String>, is_error: bool) {
        self.notice = Some((text.into(), is_error, Instant::now()));
    }

    // ── Background work ─────────────────────────────────────────────────

    pub fn refresh(&mut self) {
        if !self.refreshing {
            self.refreshing = true;
            let repo = Arc::clone(&self.repo);
            let site = self.site.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let snapshot = data::load(&repo, &site).await;
                let _ = tx.send(Msg::Snapshot(snapshot));
            });
        }
        if !self.probing {
            self.probing = true;
            let site = self.site.clone();
            let client = self.server.client().clone();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let status = system::status(&site, &client).await;
                let _ = tx.send(Msg::Status(status));
            });
        }
    }

    fn run_done(&mut self, what: &str, work: impl Future<Output = Done> + Send + 'static) {
        if self.busy.is_some() {
            self.say("something is already running", true);
            return;
        }
        self.busy = Some(what.to_string());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let done = work.await;
            let _ = tx.send(Msg::Done(done));
        });
    }

    fn run_server(&mut self, what: &str, pending: Pending) {
        if self.busy.is_some() {
            self.say("something is already running", true);
            return;
        }
        if self.server.session.is_none() {
            self.open_sign_in(Some(pending));
            return;
        }
        self.busy = Some(what.to_string());
        let server = self.server.clone();
        let tx = self.tx.clone();
        let again = pending.clone();
        tokio::spawn(async move {
            let result = match &pending {
                Pending::Action(Action::SyncNow) => server.sync_now().await,
                Pending::Action(Action::FtcPull) => server.ftc_pull().await,
                Pending::Action(other) => Err(Failure::Refused(format!(
                    "{} is not a server action",
                    other.title()
                ))),
                Pending::SaveAccount { id, team, roles } => {
                    server.save_account(*id, *team, *roles).await
                }
                Pending::ResetPassword { id, password } => {
                    server.reset_password(*id, password).await
                }
                Pending::Import(post) => server.import_schedule(post).await,
            };
            let _ = tx.send(Msg::Server(result, Some(again)));
        });
    }

    fn open_sign_in(&mut self, then: Option<Pending>) {
        let admin = self
            .snapshot
            .accounts
            .iter()
            .find(|a| a.roles.is_admin)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        self.modal = Some(Modal::Form {
            title: "Sign in as an admin".into(),
            hint: "The server checks the password; this tool keeps only the session cookie, until it exits.".into(),
            fields: vec![Field::new("Email", admin), Field::secret("Password")],
            active: 1,
            kind: FormKind::SignIn { then },
        });
    }

    // ── Messages ────────────────────────────────────────────────────────

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Key(key) => self.key(key),
            Msg::Resize => {}
            Msg::Snapshot(snapshot) => {
                self.refreshing = false;
                let events = snapshot.events.len();
                if self.data_at >= events {
                    self.data_at = events.saturating_sub(1);
                }
                let people = snapshot.accounts.len();
                if self.people_at >= people {
                    self.people_at = people.saturating_sub(1);
                }
                self.snapshot = snapshot;
            }
            Msg::Status(status) => {
                self.probing = false;
                self.status = status;
            }
            Msg::Log(line) => {
                self.logs.push(line);
                if self.log_follow {
                    self.log_at = usize::MAX;
                }
            }
            Msg::LogSource { name, trouble } => {
                self.logs.source = name;
                self.logs.trouble = trouble;
            }
            Msg::Done(done) => {
                self.busy = None;
                self.modal = Some(Modal::Result {
                    title: done.title.clone(),
                    ok: done.ok,
                    lines: done.lines,
                    scroll: 0,
                });
                self.refresh();
            }
            Msg::Server(result, pending) => {
                self.busy = None;
                match result {
                    Ok(report) => {
                        let title = match &pending {
                            Some(Pending::Action(a)) => a.title().to_string(),
                            Some(Pending::SaveAccount { .. }) => "Account".into(),
                            Some(Pending::ResetPassword { .. }) => "Password reset".into(),
                            Some(Pending::Import(_)) => "Schedule import".into(),
                            None => "Server".into(),
                        };
                        self.modal = Some(Modal::Result {
                            title,
                            ok: report.ok,
                            lines: report.lines,
                            scroll: 0,
                        });
                        self.refresh();
                    }
                    Err(Failure::NotSignedIn) => {
                        self.server.sign_out();
                        self.say("the server wants a sign-in", true);
                        self.open_sign_in(pending);
                    }
                    Err(e) => {
                        self.modal = Some(Modal::Result {
                            title: "Not done".into(),
                            ok: false,
                            lines: vec![e.to_string()],
                            scroll: 0,
                        });
                    }
                }
            }
            Msg::SignedIn(result, then) => {
                self.busy = None;
                match result {
                    Ok(server) => {
                        let who = server.signed_in_as.clone().unwrap_or_default();
                        self.server = server;
                        self.say(format!("signed in as {who}"), false);
                        if let Some(pending) = then {
                            self.start_pending(pending);
                        }
                    }
                    Err(e) => {
                        self.say(format!("sign-in failed: {e}"), true);
                        self.open_sign_in(then);
                    }
                }
            }
            Msg::Quit => self.should_quit = true,
        }
    }

    pub fn on_tick(&mut self) {
        if let Some((_, _, at)) = &self.notice
            && at.elapsed() > Duration::from_secs(6)
        {
            self.notice = None;
        }
        self.refresh();
    }

    // ── Keys ────────────────────────────────────────────────────────────

    fn key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.modal.is_some() {
            self.modal_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab | KeyCode::Right if self.tab != Tab::Logs || key.code == KeyCode::Tab => {
                self.tab = self.tab.next()
            }
            KeyCode::BackTab | KeyCode::Left
                if self.tab != Tab::Logs || key.code == KeyCode::BackTab =>
            {
                self.tab = self.tab.prev()
            }
            KeyCode::Char('1') => self.tab = Tab::Overview,
            KeyCode::Char('2') => self.tab = Tab::Logs,
            KeyCode::Char('3') => self.tab = Tab::People,
            KeyCode::Char('4') => self.tab = Tab::Data,
            KeyCode::Char('5') => self.tab = Tab::Actions,
            KeyCode::Char('r') => {
                self.refresh();
                self.say("refreshing", false);
            }
            _ => match self.tab {
                Tab::Overview => {}
                Tab::Logs => self.logs_key(key),
                Tab::People => self.people_key(key),
                Tab::Data => self.data_key(key),
                Tab::Actions => self.actions_key(key),
            },
        }
    }

    fn logs_key(&mut self, key: KeyEvent) {
        let shown = self.now_lines().len();
        let last = shown.saturating_sub(1);
        if self.log_at > last {
            self.log_at = last;
        }
        match key.code {
            KeyCode::Char('f') => {
                self.log_follow = !self.log_follow;
                if self.log_follow {
                    self.log_at = last;
                }
            }
            KeyCode::Char('l') => {
                self.log_level = match self.log_level {
                    Level::Info => Level::Warn,
                    Level::Warn => Level::Error,
                    _ => Level::Info,
                };
                self.log_at = usize::MAX;
            }
            KeyCode::Char('s') => {
                self.log_subject = self.log_subject.next();
                self.log_at = usize::MAX;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.log_follow = false;
                self.log_at = self.log_at.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.log_at < last {
                    self.log_at += 1;
                } else {
                    self.log_follow = true;
                }
            }
            KeyCode::PageUp => {
                self.log_follow = false;
                self.log_at = self.log_at.saturating_sub(20);
            }
            KeyCode::PageDown => {
                self.log_at = (self.log_at + 20).min(last);
                if self.log_at == last {
                    self.log_follow = true;
                }
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.log_follow = false;
                self.log_at = 0;
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.log_follow = true;
                self.log_at = last;
            }
            _ => {}
        }
    }

    fn people_key(&mut self, key: KeyEvent) {
        let accounts = self.snapshot.accounts.len();
        let devices = self.snapshot.devices.len();
        match key.code {
            KeyCode::Char('d') => self.people_devices = !self.people_devices,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.people_devices {
                    self.devices_at = self.devices_at.saturating_sub(1);
                } else {
                    self.people_at = self.people_at.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.people_devices {
                    self.devices_at = (self.devices_at + 1).min(devices.saturating_sub(1));
                } else {
                    self.people_at = (self.people_at + 1).min(accounts.saturating_sub(1));
                }
            }
            KeyCode::Enter | KeyCode::Char('e') if !self.people_devices => {
                if let Some(account) = self.snapshot.accounts.get(self.people_at) {
                    let roles = account.roles;
                    self.modal = Some(Modal::Form {
                        title: format!("Edit {}", account.name),
                        hint: "Roles: type y or n. Team: a number, or blank for none. Enter saves through the server's Accounts form."
                            .into(),
                        fields: vec![
                            Field::new("Team number", account.team_number.map(|t| t.to_string()).unwrap_or_default()),
                            Field::new("Admin (y/n)", yn(roles.is_admin)),
                            Field::new("Lead scout (y/n)", yn(roles.is_lead_scout)),
                            Field::new("Coach (y/n)", yn(roles.is_coach)),
                        ],
                        active: 0,
                        kind: FormKind::EditAccount { id: account.id },
                    });
                }
            }
            KeyCode::Char('p') if !self.people_devices => {
                if let Some(account) = self.snapshot.accounts.get(self.people_at) {
                    self.modal = Some(Modal::Form {
                        title: format!("Reset {}'s password", account.name),
                        hint: "8 characters or more. That account is signed out everywhere; tell them the new one in person."
                            .into(),
                        fields: vec![Field::secret("New password")],
                        active: 0,
                        kind: FormKind::ResetPassword { id: account.id },
                    });
                }
            }
            _ => {}
        }
    }

    fn data_key(&mut self, key: KeyEvent) {
        let events = self.snapshot.events.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.data_at = self.data_at.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.data_at = (self.data_at + 1).min(events.saturating_sub(1))
            }
            KeyCode::Char('i') => self.open_import(),
            KeyCode::Char('s') => self.start_action(Action::SyncNow),
            KeyCode::Char('f') => self.start_action(Action::FtcPull),
            KeyCode::Char('b') => self.start_action(Action::Backup),
            _ => {}
        }
    }

    fn actions_key(&mut self, key: KeyEvent) {
        let actions = self.actions();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.actions_at = self.actions_at.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.actions_at = (self.actions_at + 1).min(actions.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(action) = actions.get(self.actions_at) {
                    self.start_action(*action);
                }
            }
            _ => {}
        }
    }

    // ── Starting things ─────────────────────────────────────────────────

    fn start_action(&mut self, action: Action) {
        match action {
            Action::Restart | Action::Stop | Action::Poweroff => {
                let (body, word) = match action {
                    Action::Poweroff => (
                        vec![
                            "This powers off the machine the server runs on.".into(),
                            "Back up to the stick first, and eject it.".into(),
                            String::new(),
                            "Type poweroff to confirm.".into(),
                        ],
                        Some("poweroff".to_string()),
                    ),
                    Action::Stop => (
                        vec![
                            "The pages stop answering until it is started again.".into(),
                            String::new(),
                            "y to confirm".into(),
                        ],
                        None,
                    ),
                    _ => (
                        vec![
                            "Everyone's next page load waits about 5 seconds.".into(),
                            "Forms a device holds are not lost; they sync when it is back.".into(),
                            String::new(),
                            "y to confirm".into(),
                        ],
                        None,
                    ),
                };
                self.modal = Some(Modal::Confirm {
                    title: action.title().into(),
                    body,
                    word,
                    typed: String::new(),
                    then: Pending::Action(action),
                });
            }
            Action::Start => self.start_pending(Pending::Action(action)),
            Action::Backup => {
                let default = self
                    .site
                    .backup_dir
                    .as_ref()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default();
                self.modal = Some(Modal::Form {
                    title: "Back up now".into(),
                    hint: "The folder the copy goes into. A USB stick is under /media or /run/media. Wait for \"It is on the disk.\"".into(),
                    fields: vec![Field::new("Folder", default)],
                    active: 0,
                    kind: FormKind::Backup,
                });
            }
            Action::CheckBackup => {
                let newest = self
                    .snapshot
                    .backups
                    .first()
                    .map(|b| b.path.display().to_string())
                    .unwrap_or_default();
                self.modal = Some(Modal::Form {
                    title: "Check a backup".into(),
                    hint: "The file to restore into a scratch database. Blank: the newest timed snapshot.".into(),
                    fields: vec![Field::new("Backup file", newest)],
                    active: 0,
                    kind: FormKind::CheckBackup,
                });
            }
            Action::ImportSchedule => self.open_import(),
            Action::SignIn => self.open_sign_in(None),
            Action::SignOut => {
                self.server.sign_out();
                self.say("session forgotten", false);
            }
            Action::SyncNow | Action::FtcPull => self.start_pending(Pending::Action(action)),
        }
    }

    fn open_import(&mut self) {
        let (code, name, timezone) = match self.selected_event() {
            Some(e) => (
                e.key
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .to_uppercase(),
                e.name.clone(),
                String::new(),
            ),
            None => (
                self.site
                    .env
                    .get("FTC_EVENT_CODE")
                    .cloned()
                    .unwrap_or_default(),
                String::new(),
                String::new(),
            ),
        };
        let timezone = if timezone.is_empty() {
            "America/Chicago".to_string()
        } else {
            timezone
        };
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.modal = Some(Modal::Form {
            title: "Import a schedule".into(),
            hint: "Paths are on this machine. Re-importing replaces the matches and keeps every observation. Roster is optional (number,name per line)."
                .into(),
            fields: vec![
                Field::new("Event code", code),
                Field::new("Event name", name),
                Field::new("Time zone (IANA)", timezone),
                Field::new("Date (YYYY-MM-DD)", today),
                Field::new("Schedule CSV path", ""),
                Field::new("Roster CSV path", ""),
            ],
            active: 4,
            kind: FormKind::Import,
        });
    }

    fn start_pending(&mut self, pending: Pending) {
        match &pending {
            Pending::Action(action) if !action.needs_session() => {
                let site = self.site.clone();
                let action = *action;
                self.run_done(action.title(), async move {
                    match action {
                        Action::Restart => system::restart(&site).await,
                        Action::Stop => system::stop(&site).await,
                        Action::Start => system::start(&site).await,
                        Action::Poweroff => system::poweroff(&site).await,
                        _ => Done {
                            ok: false,
                            title: action.title().into(),
                            lines: vec!["not a command".into()],
                        },
                    }
                });
            }
            Pending::Action(action) => {
                let title = action.title();
                self.run_server(title, pending.clone());
            }
            Pending::SaveAccount { .. } => self.run_server("saving the account", pending.clone()),
            Pending::ResetPassword { .. } => {
                self.run_server("resetting the password", pending.clone())
            }
            Pending::Import(_) => self.run_server("importing the schedule", pending.clone()),
        }
    }

    // ── Modal keys ──────────────────────────────────────────────────────

    fn modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        match modal {
            Modal::Result {
                title,
                ok,
                lines,
                scroll,
            } => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ') => {}
                KeyCode::Up | KeyCode::Char('k') => {
                    self.modal = Some(Modal::Result {
                        title,
                        ok,
                        lines,
                        scroll: scroll.saturating_sub(1),
                    })
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let max = lines.len().saturating_sub(1) as u16;
                    self.modal = Some(Modal::Result {
                        title,
                        ok,
                        lines,
                        scroll: (scroll + 1).min(max),
                    })
                }
                _ => {
                    self.modal = Some(Modal::Result {
                        title,
                        ok,
                        lines,
                        scroll,
                    })
                }
            },
            Modal::Confirm {
                title,
                body,
                word,
                mut typed,
                then,
            } => match (&word, key.code) {
                (_, KeyCode::Esc) => self.say("cancelled", false),
                (None, KeyCode::Char('y')) | (None, KeyCode::Enter) => self.start_pending(then),
                (None, _) => {
                    self.modal = Some(Modal::Confirm {
                        title,
                        body,
                        word,
                        typed,
                        then,
                    })
                }
                (Some(expected), KeyCode::Enter) => {
                    if typed.trim() == expected {
                        self.start_pending(then);
                    } else {
                        self.say(format!("type {expected} exactly to confirm"), true);
                        self.modal = Some(Modal::Confirm {
                            title,
                            body,
                            word,
                            typed: String::new(),
                            then,
                        });
                    }
                }
                (Some(_), KeyCode::Backspace) => {
                    typed.pop();
                    self.modal = Some(Modal::Confirm {
                        title,
                        body,
                        word,
                        typed,
                        then,
                    });
                }
                (Some(_), KeyCode::Char(c)) => {
                    typed.push(c);
                    self.modal = Some(Modal::Confirm {
                        title,
                        body,
                        word,
                        typed,
                        then,
                    });
                }
                (Some(_), _) => {
                    self.modal = Some(Modal::Confirm {
                        title,
                        body,
                        word,
                        typed,
                        then,
                    })
                }
            },
            Modal::Form {
                title,
                hint,
                mut fields,
                mut active,
                kind,
            } => match key.code {
                KeyCode::Esc => self.say("cancelled", false),
                KeyCode::Enter => self.submit(fields, kind),
                KeyCode::Tab | KeyCode::Down => {
                    active = (active + 1) % fields.len();
                    self.modal = Some(Modal::Form {
                        title,
                        hint,
                        fields,
                        active,
                        kind,
                    });
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active = (active + fields.len() - 1) % fields.len();
                    self.modal = Some(Modal::Form {
                        title,
                        hint,
                        fields,
                        active,
                        kind,
                    });
                }
                KeyCode::Backspace => {
                    fields[active].value.pop();
                    self.modal = Some(Modal::Form {
                        title,
                        hint,
                        fields,
                        active,
                        kind,
                    });
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    fields[active].value.clear();
                    self.modal = Some(Modal::Form {
                        title,
                        hint,
                        fields,
                        active,
                        kind,
                    });
                }
                KeyCode::Char(c) => {
                    fields[active].value.push(c);
                    self.modal = Some(Modal::Form {
                        title,
                        hint,
                        fields,
                        active,
                        kind,
                    });
                }
                _ => {
                    self.modal = Some(Modal::Form {
                        title,
                        hint,
                        fields,
                        active,
                        kind,
                    })
                }
            },
        }
    }

    fn submit(&mut self, fields: Vec<Field>, kind: FormKind) {
        let value = |i: usize| {
            fields
                .get(i)
                .map(|f| f.value.trim().to_string())
                .unwrap_or_default()
        };
        match kind {
            FormKind::SignIn { then } => {
                let email = value(0);
                let password = value(1);
                if email.is_empty() || password.is_empty() {
                    self.say("email and password are both needed", true);
                    self.open_sign_in(then);
                    return;
                }
                self.busy = Some("signing in".into());
                let mut server = self.server.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = server.sign_in(&email, &password).await.map(|()| server);
                    let _ = tx.send(Msg::SignedIn(result, then));
                });
            }
            FormKind::Backup => {
                let folder = value(0);
                if folder.is_empty() {
                    self.say("name a folder", true);
                    return;
                }
                let site = self.site.clone();
                let folder = PathBuf::from(folder);
                self.run_done(
                    "backing up",
                    async move { system::backup(&site, &folder).await },
                );
            }
            FormKind::CheckBackup => {
                let file = value(0);
                let site = self.site.clone();
                let file = (!file.is_empty()).then(|| PathBuf::from(file));
                self.run_done("checking the backup", async move {
                    system::check_backup(&site, file.as_deref()).await
                });
            }
            FormKind::EditAccount { id } => {
                let team_raw = value(0);
                let team = if team_raw.is_empty() {
                    None
                } else {
                    match team_raw.parse::<i32>() {
                        Ok(t) if t > 0 => Some(t),
                        _ => {
                            self.say("team must be a positive number or blank", true);
                            return;
                        }
                    }
                };
                let flag = |i: usize| {
                    matches!(
                        value(i).to_ascii_lowercase().as_str(),
                        "y" | "yes" | "true" | "1"
                    )
                };
                let roles = Roles {
                    is_admin: flag(1),
                    is_lead_scout: flag(2),
                    is_coach: flag(3),
                };
                self.start_pending(Pending::SaveAccount { id, team, roles });
            }
            FormKind::ResetPassword { id } => {
                let password = value(0);
                if password.chars().count() < 8 {
                    self.say("8 characters or more", true);
                    return;
                }
                self.start_pending(Pending::ResetPassword { id, password });
            }
            FormKind::Import => {
                let schedule_path = value(4);
                if schedule_path.is_empty() {
                    self.say("name the schedule CSV", true);
                    return;
                }
                let schedule = match std::fs::read_to_string(&schedule_path) {
                    Ok(s) => s,
                    Err(e) => {
                        self.say(format!("cannot read {schedule_path}: {e}"), true);
                        return;
                    }
                };
                let roster_path = value(5);
                let roster = if roster_path.is_empty() {
                    String::new()
                } else {
                    match std::fs::read_to_string(&roster_path) {
                        Ok(s) => s,
                        Err(e) => {
                            self.say(format!("cannot read {roster_path}: {e}"), true);
                            return;
                        }
                    }
                };
                self.start_pending(Pending::Import(SchedulePost {
                    code: value(0),
                    name: value(1),
                    timezone: value(2),
                    date: value(3),
                    schedule,
                    roster,
                }));
            }
        }
    }
}

fn yn(b: bool) -> &'static str {
    if b { "y" } else { "n" }
}
