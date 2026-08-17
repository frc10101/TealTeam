//! Network status badge: whether the box can reach the internet and how the
//! last FIRST/TBA call went.
//!
//! Shown on pages where it changes what a scout should expect — a red badge
//! explains why the schedule is stale. The classification itself lives in
//! [`crate::services::connectivity::NetworkStatusSnapshot::classify`]; this
//! module only turns it into a label and colours.

use askama::Template;
use chrono::{DateTime, Local, Utc};

use crate::services::connectivity::NetworkStatusSnapshot;

/// The badge, with timestamps for the tooltip.
#[derive(Template)]
#[template(path = "partials/network_status_badge.html")]
pub struct NetworkStatusBadgeFragment {
    pub status: String,
    pub label: String,
    pub css_class: String,
    pub last_sync: String,
    pub last_api_success: String,
    pub last_api_error_text: String,
    pub last_api_error: String,
    pub internet_error: String,
}

impl NetworkStatusBadgeFragment {
    /// Presents a connectivity snapshot under an already-classified status.
    pub fn from_snapshot(status: String, snapshot: &NetworkStatusSnapshot) -> Self {
        let (label, css_class) = match status.as_str() {
            "internet-ok" => ("Internet OK", "bg-teal-900/40 text-teal-200 border-teal-600"),
            "api-error" => ("API Error", "bg-amber-900/40 text-amber-200 border-amber-600"),
            _ => ("Offline", "bg-red-900/40 text-red-200 border-red-600"),
        };

        Self {
            status,
            label: label.to_string(),
            css_class: css_class.to_string(),
            last_sync: format_status_time(snapshot.last_successful_sync),
            last_api_success: format_status_time(snapshot.last_api_success_at),
            last_api_error_text: format_status_time(snapshot.last_api_error_at),
            last_api_error: snapshot.last_api_error.clone(),
            internet_error: snapshot.internet_error.clone(),
        }
    }
}

/// Local-time stamp, or "Never" when it has not happened yet.
fn format_status_time(t: Option<DateTime<Utc>>) -> String {
    match t {
        Some(t) => t.with_timezone(&Local).format("%b %-d, %-I:%M:%S %p").to_string(),
        None => "Never".to_string(),
    }
}
