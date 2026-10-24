//! Drawing. Pure: reads the [`App`], writes the frame, decides nothing.

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, Padding, Paragraph, Row, Table, TableState, Tabs,
    Wrap,
};
use tt_repo::DEVICE_ONLINE_WINDOW;

use crate::app::{App, Modal, Tab};
use crate::data::{ago, human_bytes};
use crate::logs::Level;
use crate::system::Verdict;

const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;
const WARN: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;
const ACCENT: Color = Color::Cyan;

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_header(frame, app, header);
    match app.tab {
        Tab::Overview => draw_overview(frame, app, body),
        Tab::Logs => draw_logs(frame, app, body),
        Tab::People => draw_people(frame, app, body),
        Tab::Data => draw_data(frame, app, body),
        Tab::Actions => draw_actions(frame, app, body),
    }
    draw_footer(frame, app, footer);
    if let Some(modal) = &app.modal {
        draw_modal(frame, modal, frame.area());
    }
}

fn card<'a>(title: impl std::fmt::Display) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).bold(),
        ))
        .padding(Padding::horizontal(1))
}

fn kv<'a>(key: &'a str, value: impl Into<String>, style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{key:<12}"), Style::default().fg(DIM)),
        Span::styled(value.into(), style),
    ])
}

fn plain() -> Style {
    Style::default()
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let [tabs_area, status_area] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(44)]).areas(area);
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" {} {} ", i + 1, t.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0))
        .highlight_style(Style::default().fg(Color::Black).bg(ACCENT).bold())
        .divider("")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM))
                .title(Span::styled(
                    " TealTeam ops ",
                    Style::default().fg(ACCENT).bold(),
                )),
        );
    frame.render_widget(tabs, tabs_area);

    let process = &app.status.process;
    let (word, color) = if process.running && app.status.health.reachable {
        ("UP", GOOD)
    } else if process.running {
        ("STARTING", WARN)
    } else {
        ("DOWN", BAD)
    };
    let who = app
        .server
        .signed_in_as
        .as_deref()
        .map(|w| format!("admin: {w}"))
        .unwrap_or_else(|| "not signed in".into());
    let status = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                format!(" {word} "),
                Style::default().fg(Color::Black).bg(color).bold(),
            ),
            Span::raw(format!(
                " {} · {} · :{}",
                app.status.hostname,
                app.site.kind(),
                app.site.port
            )),
        ]),
        Line::from(Span::styled(format!(" {who}"), Style::default().fg(DIM))),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(DIM)),
    );
    frame.render_widget(status, status_area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let keys = match (&app.modal, app.tab) {
        (Some(Modal::Result { .. }), _) => "Esc/Enter close · j/k scroll",
        (Some(Modal::Confirm { word: Some(_), .. }), _) => "type the word, Enter · Esc cancel",
        (Some(Modal::Confirm { .. }), _) => "y confirm · Esc cancel",
        (Some(Modal::Form { .. }), _) => {
            "Tab next field · Enter submit · Ctrl-U clear · Esc cancel"
        }
        (None, Tab::Overview) => "1-5 tabs · Tab/Shift-Tab · r refresh · q quit",
        (None, Tab::Logs) => "j/k scroll · f follow · l level · s subject · g/G ends · q quit",
        (None, Tab::People) => {
            "j/k move · Enter edit roles/team · p reset password · d accounts/devices"
        }
        (None, Tab::Data) => "j/k pick event · i import CSV · s sync now · f FTC pull · b back up",
        (None, Tab::Actions) => "j/k move · Enter run · q quit",
    };
    let mut spans = vec![Span::styled(format!(" {keys}"), Style::default().fg(DIM))];
    if let Some(busy) = &app.busy {
        spans.push(Span::styled(
            format!("   ⟳ {busy}…"),
            Style::default().fg(WARN).bold(),
        ));
    }
    if let Some((text, is_error, _)) = &app.notice {
        let color = if *is_error { BAD } else { GOOD };
        spans.push(Span::styled(
            format!("   {text}"),
            Style::default().fg(color),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Overview ────────────────────────────────────────────────────────────────

fn draw_overview(frame: &mut Frame, app: &App, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let [process_area, network_area, data_area] = Layout::vertical([
        Constraint::Length(10),
        Constraint::Min(6),
        Constraint::Length(9),
    ])
    .areas(left);
    let [recent_area, alarms_area] =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(right);

    let now = Utc::now();
    let p = &app.status.process;
    let h = &app.status.health;
    let state_style = Style::default()
        .fg(if p.running { GOOD } else { BAD })
        .bold();
    let mut lines = vec![
        kv("process", &p.state, state_style),
        kv(
            "pid",
            match (p.pid, p.uptime) {
                (Some(pid), Some(up)) => format!("{pid}, up {}", duration(up)),
                (Some(pid), None) => pid.to_string(),
                _ => "—".into(),
            },
            plain(),
        ),
        kv(
            "memory",
            p.rss_bytes.map(human_bytes).unwrap_or_else(|| "—".into()),
            plain(),
        ),
        kv(
            "restarts",
            p.restarts
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            plain(),
        ),
        kv(
            "/health",
            if h.reachable {
                format!(
                    "storage {} · build {} · schema {} · form {}{}",
                    h.storage,
                    h.build,
                    h.schema,
                    h.form,
                    h.latency
                        .map(|l| format!(" · {} ms", l.as_millis()))
                        .unwrap_or_default()
                )
            } else {
                "not answering".into()
            },
            Style::default().fg(if h.reachable && h.storage == "ready" {
                GOOD
            } else if h.reachable {
                WARN
            } else {
                BAD
            }),
        ),
        kv("load", &app.status.load, plain()),
        kv("via", &p.via, Style::default().fg(DIM)),
    ];
    if let Some(tls) = app.status.tls_listening {
        lines.insert(
            5,
            kv(
                "https",
                format!(
                    "port {} {}",
                    app.site.tls_port.unwrap_or(0),
                    if tls { "listening" } else { "NOT listening" }
                ),
                Style::default().fg(if tls { GOOD } else { BAD }),
            ),
        );
    }
    frame.render_widget(Paragraph::new(lines).block(card("Server")), process_area);

    let checks: Vec<Line> = app
        .status
        .network
        .iter()
        .map(|c| {
            let (mark, color) = match c.verdict {
                Verdict::Ok => ("ok  ", GOOD),
                Verdict::Fail => ("FAIL", BAD),
                Verdict::Note => ("--  ", DIM),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(color).bold()),
                Span::raw(c.text.clone()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(checks)
            .wrap(Wrap { trim: false })
            .block(card("Network")),
        network_area,
    );

    let s = &app.snapshot;
    let newest_backup = s.backups.first().and_then(|b| b.taken_at);
    let backup_style = match newest_backup {
        Some(at) if now - at < chrono::TimeDelta::minutes(15) => Style::default().fg(GOOD),
        Some(_) => Style::default().fg(WARN),
        None => Style::default().fg(BAD),
    };
    let obs: i64 = s.events.iter().map(|e| e.observations).sum();
    let pending: i64 = s.events.iter().map(|e| e.pending).sum();
    let lines = vec![
        kv(
            "database",
            format!(
                "{}{}",
                s.db_location,
                s.db_bytes
                    .map(|b| format!(" ({})", human_bytes(b)))
                    .unwrap_or_default()
            ),
            Style::default().fg(if s.storage_ready { GOOD } else { BAD }),
        ),
        kv(
            "disk",
            app.status
                .disk_free
                .map(|(free, total)| {
                    format!("{} free of {}", human_bytes(free), human_bytes(total))
                })
                .unwrap_or_else(|| "—".into()),
            plain(),
        ),
        kv(
            "events",
            format!(
                "{}{}",
                s.events.len(),
                s.default_event
                    .as_ref()
                    .map(|e| format!(" · default {e}"))
                    .unwrap_or_default()
            ),
            plain(),
        ),
        kv(
            "forms",
            format!("{obs} observations, {pending} pending review"),
            plain(),
        ),
        kv(
            "online",
            format!(
                "{} scouts, {} devices (last {} min)",
                s.online_scouts(now),
                s.online_devices(now),
                DEVICE_ONLINE_WINDOW.num_minutes()
            ),
            plain(),
        ),
        kv(
            "backups",
            format!(
                "{} kept · newest {}",
                s.backups.len(),
                ago(newest_backup, now)
            ),
            backup_style,
        ),
        kv(
            "schema",
            s.schema_version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "—".into()),
            plain(),
        ),
    ];
    frame.render_widget(Paragraph::new(lines).block(card("Data")), data_area);

    let recent: Vec<ListItem> = s
        .recent
        .iter()
        .take(recent_area.height.saturating_sub(2) as usize)
        .map(|r| {
            let color = if r.kind == "sign-in" {
                ACCENT
            } else {
                plain().fg.unwrap_or(Color::Reset)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:>10}  ", ago(Some(r.at), now)),
                    Style::default().fg(DIM),
                ),
                Span::styled(r.what.clone(), Style::default().fg(color)),
            ]))
        })
        .collect();
    let recent = if recent.is_empty() {
        List::new([ListItem::new(Span::styled(
            "nothing yet",
            Style::default().fg(DIM),
        ))])
    } else {
        List::new(recent)
    };
    frame.render_widget(recent.block(card("Recent sign-ins and forms")), recent_area);

    let alarms = app
        .logs
        .alarms(alarms_area.height.saturating_sub(2) as usize);
    let mut items: Vec<ListItem> = alarms
        .iter()
        .map(|l| ListItem::new(log_line(l, alarms_area.width)))
        .collect();
    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            if app.logs.lines.is_empty() {
                format!("no log lines yet ({})", app.logs.source)
            } else {
                "nothing has gone wrong in the lines held".to_string()
            },
            Style::default().fg(DIM),
        )));
    }
    if let Some(trouble) = &app.logs.trouble {
        items.insert(
            0,
            ListItem::new(Span::styled(trouble.clone(), Style::default().fg(BAD))),
        );
    }
    frame.render_widget(
        List::new(items).block(card("Warnings and errors")),
        alarms_area,
    );
}

fn duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else if s < 86_400 {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    } else {
        format!("{} d {} h", s / 86_400, (s % 86_400) / 3600)
    }
}

fn log_line<'a>(l: &'a crate::logs::Line, width: u16) -> Line<'a> {
    let color = match l.level {
        Level::Error => BAD,
        Level::Warn => WARN,
        Level::Info => GOOD,
        Level::Debug | Level::Trace => DIM,
        Level::Other => Color::Magenta,
    };
    let target = l.target.rsplit("::").next().unwrap_or("");
    let budget = (width as usize)
        .saturating_sub(8 + 6 + target.len() + 3)
        .max(10);
    let text = if l.text.chars().count() > budget {
        let cut: String = l.text.chars().take(budget.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        l.text.clone()
    };
    let mut spans = Vec::new();
    if !l.time.is_empty() {
        spans.push(Span::styled(
            format!("{} ", l.time),
            Style::default().fg(DIM),
        ));
    }
    if l.level != Level::Other {
        spans.push(Span::styled(
            format!("{:<5} ", l.level.label()),
            Style::default().fg(color).bold(),
        ));
    }
    if !target.is_empty() {
        spans.push(Span::styled(
            format!("{target}: "),
            Style::default().fg(DIM),
        ));
    }
    spans.push(Span::styled(
        text,
        Style::default().fg(if l.level == Level::Other {
            color
        } else {
            Color::Reset
        }),
    ));
    Line::from(spans)
}

// ── Logs ────────────────────────────────────────────────────────────────────

fn draw_logs(frame: &mut Frame, app: &App, area: Rect) {
    let shown = app.now_lines();
    let height = area.height.saturating_sub(2) as usize;
    let last = shown.len().saturating_sub(1);
    let at = if app.log_follow {
        last
    } else {
        app.log_at.min(last)
    };
    let start = (at + 1).saturating_sub(height);
    let items: Vec<ListItem> = shown[start..=at.min(last)]
        .iter()
        .map(|l| ListItem::new(log_line(l, area.width)))
        .collect();
    let items = if shown.is_empty() {
        vec![ListItem::new(Span::styled(
            match &app.logs.trouble {
                Some(t) => t.clone(),
                None if app.logs.lines.is_empty() => format!("waiting for {}", app.logs.source),
                None => "nothing matches these filters".into(),
            },
            Style::default().fg(DIM),
        ))]
    } else {
        items
    };
    let title = format!(
        "Logs · {} · {} and up · {} · {}{}",
        app.logs.source,
        if app.log_level == Level::Info {
            "info"
        } else {
            app.log_level.label()
        },
        app.log_subject.label(),
        if app.log_follow {
            "following"
        } else {
            "paused"
        },
        if shown.is_empty() {
            String::new()
        } else {
            format!(" · {} of {}", at + 1, shown.len())
        }
    );
    frame.render_widget(List::new(items).block(card(&title)), area);
}

// ── People ──────────────────────────────────────────────────────────────────

fn draw_people(frame: &mut Frame, app: &App, area: Rect) {
    let [accounts_area, devices_area] =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(area);
    let now = Utc::now();
    let s = &app.snapshot;

    let header = Row::new([
        "Name",
        "Email",
        "Roles",
        "Team",
        "Last sign-in",
        "Sessions",
        "Seen",
    ])
    .style(Style::default().fg(DIM).bold());
    let rows: Vec<Row> = s
        .accounts
        .iter()
        .map(|a| {
            let mut roles = Vec::new();
            if a.roles.is_admin {
                roles.push("admin");
            }
            if a.roles.is_lead_scout {
                roles.push("lead");
            }
            if a.roles.is_coach {
                roles.push("coach");
            }
            if roles.is_empty() {
                roles.push("scout");
            }
            let sessions = s.sessions.get(&a.id).map(|x| x.count).unwrap_or(0);
            let scout = s.scouts.iter().find(|x| x.id == a.id);
            let online = scout.is_some_and(|x| x.is_online(now, DEVICE_ONLINE_WINDOW));
            let seen = scout.and_then(|x| x.last_seen_at);
            Row::new(vec![
                Cell::from(a.name.clone()),
                Cell::from(a.email.clone()).style(Style::default().fg(DIM)),
                Cell::from(roles.join(", ")),
                Cell::from(a.team_number.map(|t| t.to_string()).unwrap_or_default()),
                Cell::from(ago(a.last_login_at, now)),
                Cell::from(sessions.to_string()),
                Cell::from(if online {
                    "online".to_string()
                } else {
                    ago(seen, now)
                })
                .style(Style::default().fg(if online { GOOD } else { DIM })),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(14),
        Constraint::Min(22),
        Constraint::Length(18),
        Constraint::Length(6),
        Constraint::Length(14),
        Constraint::Length(8),
        Constraint::Length(12),
    ];
    let mut state = TableState::default();
    if !app.people_devices {
        state.select(Some(app.people_at.min(s.accounts.len().saturating_sub(1))));
    }
    let title = format!(
        "Accounts ({}){}",
        s.accounts.len(),
        if app.people_devices { "" } else { " ◀" }
    );
    let table = Table::new(rows, widths)
        .header(header)
        .block(card(&title))
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, accounts_area, &mut state);

    let header = Row::new(["Device", "Team", "Last seen", "Signed in", "Clock"])
        .style(Style::default().fg(DIM).bold());
    let rows: Vec<Row> = s
        .devices
        .iter()
        .map(|d| {
            let online = d.is_online(now, DEVICE_ONLINE_WINDOW);
            let user = d
                .last_user_id
                .and_then(|id| s.account(id))
                .map(|a| a.name.clone())
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(d.display_name()),
                Cell::from(d.team_number.map(|t| t.to_string()).unwrap_or_default()),
                Cell::from(if online {
                    "online".to_string()
                } else {
                    ago(d.last_seen_at, now)
                })
                .style(Style::default().fg(if online { GOOD } else { DIM })),
                Cell::from(user),
                Cell::from(
                    d.clock_offset_ms
                        .map(|ms| {
                            format!(
                                "{}{:.1} s",
                                if ms >= 0 { "+" } else { "-" },
                                ms.abs() as f64 / 1000.0
                            )
                        })
                        .unwrap_or_default(),
                )
                .style(Style::default().fg(match d.clock_offset_ms {
                    Some(ms) if ms.abs() > 30_000 => BAD,
                    Some(ms) if ms.abs() > 5_000 => WARN,
                    _ => Color::Reset,
                })),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(18),
        Constraint::Length(6),
        Constraint::Length(14),
        Constraint::Min(14),
        Constraint::Length(10),
    ];
    let mut state = TableState::default();
    if app.people_devices {
        state.select(Some(app.devices_at.min(s.devices.len().saturating_sub(1))));
    }
    let title = format!(
        "Devices ({}, {} online){}",
        s.devices.len(),
        s.online_devices(now),
        if app.people_devices { " ◀" } else { "" }
    );
    let table = Table::new(rows, widths)
        .header(header)
        .block(card(&title))
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, devices_area, &mut state);
}

// ── Data ────────────────────────────────────────────────────────────────────

fn draw_data(frame: &mut Frame, app: &App, area: Rect) {
    let [events_area, lower] =
        Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let [tables_area, backups_area] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(lower);
    let now = Utc::now();
    let s = &app.snapshot;

    let header = Row::new([
        "Event", "Name", "Dates", "Matches", "Teams", "Forms", "Pending", "Assigned",
    ])
    .style(Style::default().fg(DIM).bold());
    let rows: Vec<Row> =
        s.events
            .iter()
            .map(|e| {
                let is_default = s.default_event.as_deref() == Some(e.key.as_str());
                Row::new(vec![
                    Cell::from(format!("{}{}", e.key, if is_default { " ★" } else { "" })),
                    Cell::from(e.name.clone()),
                    Cell::from(e.dates.clone()).style(Style::default().fg(DIM)),
                    Cell::from(e.matches.to_string()),
                    Cell::from(e.teams.to_string()),
                    Cell::from(e.observations.to_string()),
                    Cell::from(e.pending.to_string()).style(
                        Style::default().fg(if e.pending > 0 { WARN } else { Color::Reset }),
                    ),
                    Cell::from(e.assignments.to_string()),
                ])
            })
            .collect();
    let widths = [
        Constraint::Length(16),
        Constraint::Min(18),
        Constraint::Length(24),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(9),
    ];
    let mut state = TableState::default();
    state.select(
        (!s.events.is_empty()).then_some(app.data_at.min(s.events.len().saturating_sub(1))),
    );
    let table = Table::new(rows, widths)
        .header(header)
        .block(card(format!(
            "Events ({}) · ★ the pages' default",
            s.events.len()
        )))
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, events_area, &mut state);

    let rows: Vec<Row> = s
        .tables
        .iter()
        .map(|(name, n)| Row::new(vec![Cell::from(name.clone()), Cell::from(n.to_string())]))
        .collect();
    let table = Table::new(rows, [Constraint::Min(16), Constraint::Length(10)])
        .header(Row::new(["Table", "Rows"]).style(Style::default().fg(DIM).bold()))
        .block(card("Tables"));
    frame.render_widget(table, tables_area);

    let rows: Vec<Row> = s
        .backups
        .iter()
        .take(backups_area.height.saturating_sub(3) as usize)
        .map(|b| {
            Row::new(vec![
                Cell::from(b.name.clone()),
                Cell::from(ago(b.taken_at, now)),
                Cell::from(human_bytes(b.bytes)),
            ])
        })
        .collect();
    let folder = app
        .site
        .backup_dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "no folder".into());
    let table = Table::new(
        rows,
        [
            Constraint::Min(28),
            Constraint::Length(16),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(["Backup", "Taken", "Size"]).style(Style::default().fg(DIM).bold()))
    .block(card(format!("Backups · {folder}")));
    frame.render_widget(table, backups_area);
}

// ── Actions ─────────────────────────────────────────────────────────────────

fn draw_actions(frame: &mut Frame, app: &App, area: Rect) {
    let [list_area, detail_area] =
        Layout::vertical([Constraint::Min(8), Constraint::Length(6)]).areas(area);
    let actions = app.actions();
    let at = app.actions_at.min(actions.len().saturating_sub(1));
    let items: Vec<ListItem> = actions
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let danger = matches!(a, crate::app::Action::Poweroff | crate::app::Action::Stop);
            let style = if i == at {
                Style::default().bg(Color::DarkGray).bold()
            } else if danger {
                Style::default().fg(BAD)
            } else {
                plain()
            };
            ListItem::new(Line::from(Span::styled(
                format!("{} {}", if i == at { "▶" } else { " " }, a.title()),
                style,
            )))
        })
        .collect();
    frame.render_widget(List::new(items).block(card("Actions")), list_area);
    let detail = actions
        .get(at)
        .map(|a| a.detail(&app.site))
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(detail)
            .wrap(Wrap { trim: false })
            .block(card("What it does")),
        detail_area,
    );
}

// ── Modals ──────────────────────────────────────────────────────────────────

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(5);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn draw_modal(frame: &mut Frame, modal: &Modal, area: Rect) {
    match modal {
        Modal::Result {
            title,
            ok,
            lines,
            scroll,
        } => {
            let height = (lines.len() as u16 + 4).clamp(6, area.height.saturating_sub(4));
            let rect = centered(area, 90, height);
            frame.render_widget(Clear, rect);
            let color = if *ok { GOOD } else { BAD };
            let text: Vec<Line> = lines.iter().map(|l| Line::from(l.clone())).collect();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .title(Span::styled(
                    format!(" {title} · {} ", if *ok { "done" } else { "not done" }),
                    Style::default().fg(color).bold(),
                ))
                .padding(Padding::new(1, 1, 1, 0));
            frame.render_widget(
                Paragraph::new(Text::from(text))
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0))
                    .block(block),
                rect,
            );
        }
        Modal::Confirm {
            title,
            body,
            word,
            typed,
            ..
        } => {
            let rect = centered(
                area,
                70,
                body.len() as u16 + if word.is_some() { 7 } else { 5 },
            );
            frame.render_widget(Clear, rect);
            let mut text: Vec<Line> = body.iter().map(|l| Line::from(l.clone())).collect();
            if word.is_some() {
                text.push(Line::from(""));
                text.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(ACCENT)),
                    Span::raw(typed.clone()),
                    Span::styled("▏", Style::default().fg(ACCENT)),
                ]));
            }
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(WARN))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default().fg(WARN).bold(),
                ))
                .padding(Padding::new(1, 1, 1, 0));
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
                rect,
            );
        }
        Modal::Form {
            title,
            hint,
            fields,
            active,
            ..
        } => {
            let hint_lines = (hint.len() as u16 / 66) + 1;
            let rect = centered(area, 72, fields.len() as u16 + hint_lines + 5);
            frame.render_widget(Clear, rect);
            let mut text: Vec<Line> = Vec::new();
            for (i, f) in fields.iter().enumerate() {
                let shown = if f.secret {
                    "•".repeat(f.value.chars().count())
                } else {
                    f.value.clone()
                };
                let is_active = i == *active;
                text.push(Line::from(vec![
                    Span::styled(
                        format!("{:<20}", f.label),
                        Style::default().fg(if is_active { ACCENT } else { DIM }),
                    ),
                    Span::styled(
                        shown,
                        if is_active {
                            Style::default().bold()
                        } else {
                            plain()
                        },
                    ),
                    Span::styled(
                        if is_active { "▏" } else { "" },
                        Style::default().fg(ACCENT),
                    ),
                ]));
            }
            text.push(Line::from(""));
            text.push(Line::from(Span::styled(
                hint.clone(),
                Style::default().fg(DIM),
            )));
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default().fg(ACCENT).bold(),
                ))
                .title_alignment(Alignment::Left)
                .padding(Padding::new(1, 1, 1, 0));
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
                rect,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Field, FormKind, Pending};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> App {
        let site = crate::site::Site {
            dir: std::path::PathBuf::from("/tmp/tt"),
            binary: std::path::PathBuf::from("/tmp/tt/tt-web"),
            env: Default::default(),
            database_url: "sqlite::memory:".into(),
            db_path: None,
            port: 8080,
            tls_port: Some(8443),
            backup_dir: None,
            supervisor: crate::site::Supervisor::None,
        };
        let repo = tt_repo_sqlite::SqliteRepo::connect("sqlite::memory:").expect("memory");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(site, repo, tx);
        app.logs.push(crate::logs::Line::parse(
            "2026-10-24T17:03:12.1Z  WARN tt_web::backups: backups are off",
        ));
        app.logs.push(crate::logs::Line::parse(
            "2026-10-24T17:03:13.1Z  INFO tt_web::startup: listening",
        ));
        app
    }

    fn render(app: &App) -> String {
        let backend = TestBackend::new(140, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, app)).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    // The lazy sqlx pool wants a runtime even to be made.
    #[tokio::test]
    async fn every_tab_draws_with_an_empty_database() {
        let mut app = app();
        for tab in Tab::ALL {
            app.tab = tab;
            let text = render(&app);
            assert!(text.contains("TealTeam ops"), "{tab:?} lost the header");
            assert!(
                text.contains("DOWN"),
                "{tab:?}: nothing is running in the test"
            );
        }
        let text = render(&app);
        assert!(text.contains("Restart the server") || text.contains("Start the server"));
    }

    // The lazy sqlx pool wants a runtime even to be made.
    #[tokio::test]
    async fn the_logs_tab_shows_held_lines_and_filters() {
        let mut app = app();
        app.tab = Tab::Logs;
        let text = render(&app);
        assert!(text.contains("backups are off"));
        assert!(text.contains("listening"));
        app.log_level = Level::Warn;
        let text = render(&app);
        assert!(text.contains("backups are off"));
        assert!(!text.contains("listening"));
    }

    // The lazy sqlx pool wants a runtime even to be made.
    #[tokio::test]
    async fn modals_draw_over_the_page() {
        let mut app = app();
        app.modal = Some(Modal::Confirm {
            title: "Power off".into(),
            body: vec!["Type poweroff to confirm.".into()],
            word: Some("poweroff".into()),
            typed: "pow".into(),
            then: Pending::Action(Action::Poweroff),
        });
        let text = render(&app);
        assert!(text.contains("Type poweroff to confirm."));
        assert!(text.contains("> pow"));

        app.modal = Some(Modal::Form {
            title: "Sign in".into(),
            hint: "hint".into(),
            fields: vec![
                Field {
                    label: "Email",
                    value: "a@b".into(),
                    secret: false,
                },
                Field {
                    label: "Password",
                    value: "secret".into(),
                    secret: true,
                },
            ],
            active: 1,
            kind: FormKind::SignIn { then: None },
        });
        let text = render(&app);
        assert!(text.contains("a@b"));
        assert!(!text.contains("secret"), "a password is never drawn");
        assert!(text.contains("••••••"));

        app.modal = Some(Modal::Result {
            title: "Back up now".into(),
            ok: true,
            lines: vec!["It is on the disk.".into()],
            scroll: 0,
        });
        let text = render(&app);
        assert!(text.contains("It is on the disk."));
        assert!(text.contains("done"));
    }
}
