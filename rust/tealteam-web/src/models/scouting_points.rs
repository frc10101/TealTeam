//! Configurable scouting point weights used for team ranking scores.
//!
//! A port of `internal/handlers/scouting_points.go` and
//! `Services/ScoutingPointsService.cs`.
//!
//! Each scouted metric (defense rating, hang level, …) has a set of keyword
//! options, and each option is worth some number of points. A built-in default
//! table ships with the app; rows in `scouting_point_weights` override
//! individual options, so a lead scout can retune what the team values for
//! this game without a redeploy, and an empty table simply means "defaults".
//!
//! A team's score is the sum over all of its approved scouting rows
//! ([`totals_by_team`]), which is why it grows with matches scouted and is
//! shown next to a match count on the rankings table.

use std::collections::HashMap;

use sqlx::PgPool;
use tracing::warn;

/// Points per option, keyed by metric then option: `defense_rating` ->
/// `high` -> `5`.
pub type WeightConfig = HashMap<String, HashMap<String, i32>>;

/// One option of one metric, with its current point value.
#[derive(Debug, Clone)]
pub struct ScoutingPointOption {
    pub key: String,
    pub points: i32,
}

impl ScoutingPointOption {
    /// Display label for the weights form, e.g. `l3` -> `L3`.
    pub fn label(&self) -> String {
        option_label(&self.key)
    }

    /// Name of this option's `<input>` on the weights form.
    pub fn field_name(&self, metric_key: &str) -> String {
        build_weight_field_name(metric_key, &self.key)
    }
}

/// One metric and its options, as rendered on the weights form.
#[derive(Debug, Clone)]
pub struct ScoutingPointSection {
    pub key: String,
    pub label: String,
    pub options: Vec<ScoutingPointOption>,
}

/// The scored fields of a single approved scouting row.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct ScoutingMetricRow {
    pub team_id: i32,
    pub defense_rating: String,
    pub traversal: String,
    pub shooting_speed: String,
    pub capacity: String,
    pub scoring_strategy: String,
    pub hang_level: String,
    pub auto_hang: String,
    pub hang_position: String,
    pub starting_position: String,
}

/// Built-in weights, used for any option the database does not override.
fn default_config() -> WeightConfig {
    let mut cfg = HashMap::new();
    let mut insert = |metric: &str, options: &[(&str, i32)]| {
        cfg.insert(
            metric.to_string(),
            options
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<HashMap<_, _>>(),
        );
    };
    insert("defense_rating", &[("high", 5), ("mid", 3), ("low", 1)]);
    insert("traversal", &[("trench", 3), ("bump", 2)]);
    insert("shooting_speed", &[("fast", 4), ("medium", 2), ("slow", 1)]);
    insert("capacity", &[("high", 4), ("medium", 2), ("low", 1)]);
    insert("scoring_strategy", &[("scoring", 4), ("defending", 3), ("passing", 2)]);
    insert("hang_level", &[("none", 0), ("l1", 2), ("l2", 4), ("l3", 6)]);
    insert("auto_hang", &[("yes", 3), ("no", 0)]);
    insert("hang_position", &[("left", 1), ("center", 2), ("right", 1)]);
    insert("starting_position", &[("left", 1), ("center", 2), ("right", 1)]);
    cfg
}

/// Metrics, their labels and their option order, as shown on the weights
/// form. Also the allow-list: an option not listed here is never rendered or
/// persisted.
const SECTION_ORDER: &[(&str, &str, &[&str])] = &[
    ("defense_rating", "Defense Rating", &["high", "mid", "low"]),
    ("traversal", "Traversal", &["trench", "bump"]),
    ("shooting_speed", "Shooting Speed", &["fast", "medium", "slow"]),
    ("capacity", "Capacity", &["high", "medium", "low"]),
    ("scoring_strategy", "Scoring Strategy", &["scoring", "defending", "passing"]),
    ("hang_level", "Hang Level", &["none", "l1", "l2", "l3"]),
    ("auto_hang", "Auto Hang", &["yes", "no"]),
    ("hang_position", "Hang Position", &["left", "center", "right"]),
    ("starting_position", "Starting Position", &["left", "center", "right"]),
];

/// Options are compared case- and whitespace-insensitively, since they come
/// from form input and from three different ports.
pub fn normalize_option(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Turns an option key into a display label: `auto_hang` -> `Auto Hang`.
pub fn option_label(option_key: &str) -> String {
    option_key
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Form field name for one weight, e.g. `weight_hang_level__l3`. The double
/// underscore keeps the two keys separable when parsing the form back.
pub fn build_weight_field_name(metric_key: &str, option_key: &str) -> String {
    format!("weight_{metric_key}__{option_key}")
}

/// Scores one scouting row: the sum of the configured points for each of its
/// metric values. Unknown or blank values contribute zero.
pub fn calculate_scouting_points(row: &ScoutingMetricRow, cfg: &WeightConfig) -> i32 {
    let get = |metric: &str, option: &str| -> i32 {
        cfg.get(metric)
            .and_then(|options| options.get(&normalize_option(option)))
            .copied()
            .unwrap_or(0)
    };

    get("defense_rating", &row.defense_rating)
        + get("traversal", &row.traversal)
        + get("shooting_speed", &row.shooting_speed)
        + get("capacity", &row.capacity)
        + get("scoring_strategy", &row.scoring_strategy)
        + get("hang_level", &row.hang_level)
        + get("auto_hang", &row.auto_hang)
        + get("hang_position", &row.hang_position)
        + get("starting_position", &row.starting_position)
}

/// Scouting point totals and scouted-match counts, keyed by team id.
#[derive(Debug, Clone, Default)]
pub struct TeamPointTotals {
    pub points: HashMap<i32, i32>,
    pub matches: HashMap<i32, i32>,
}

/// Folds every scouting row at an event into per-team totals and counts.
pub fn totals_by_team(rows: &[ScoutingMetricRow], cfg: &WeightConfig) -> TeamPointTotals {
    let mut totals = TeamPointTotals::default();
    for row in rows {
        *totals.points.entry(row.team_id).or_insert(0) += calculate_scouting_points(row, cfg);
        *totals.matches.entry(row.team_id).or_insert(0) += 1;
    }
    totals
}

/// Defaults with any database overrides applied. Overrides for metrics or
/// options the app does not know about are ignored, and an unreadable table
/// logs a warning and yields the defaults.
pub async fn load_effective_config(pool: &PgPool) -> WeightConfig {
    let mut config = default_config();

    let rows: Result<Vec<(String, String, i32)>, _> =
        sqlx::query_as("SELECT metric_key, option_key, points FROM scouting_point_weights")
            .fetch_all(pool)
            .await;

    match rows {
        Ok(rows) => {
            for (metric_key, option_key, points) in rows {
                let metric_key = normalize_option(&metric_key);
                let option_key = normalize_option(&option_key);
                if let Some(options) = config.get_mut(&metric_key) {
                    options.insert(option_key, points);
                }
            }
        }
        Err(e) => warn!("failed to load scouting point config; using defaults: {e}"),
    }

    config
}

/// The effective config as ordered form sections.
pub async fn load_sections(pool: &PgPool) -> Vec<ScoutingPointSection> {
    let cfg = load_effective_config(pool).await;
    let mut sections = Vec::new();
    for (key, label, option_keys) in SECTION_ORDER {
        let mut options = Vec::new();
        for option_key in *option_keys {
            if let Some(points) = cfg.get(*key).and_then(|m| m.get(*option_key)) {
                options.push(ScoutingPointOption {
                    key: option_key.to_string(),
                    points: *points,
                });
            }
        }
        if !options.is_empty() {
            sections.push(ScoutingPointSection {
                key: key.to_string(),
                label: label.to_string(),
                options,
            });
        }
    }
    sections
}

/// Upserts every option in one transaction, so a save is all-or-nothing.
pub async fn persist_sections(
    pool: &PgPool,
    sections: &[ScoutingPointSection],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for section in sections {
        for option in &section.options {
            sqlx::query(
                "INSERT INTO scouting_point_weights (metric_key, option_key, points)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (metric_key, option_key)
                 DO UPDATE SET points = EXCLUDED.points, updated_at = CURRENT_TIMESTAMP",
            )
            .bind(&section.key)
            .bind(&option.key)
            .bind(option.points)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Reads the submitted weights back onto `base_sections`.
///
/// Blank fields keep their current value. `None` means the form contained a
/// non-integer or an out-of-range value (outside -100..=100), in which case
/// nothing is saved.
pub fn parse_sections_from_form(
    form: &HashMap<String, String>,
    base_sections: &[ScoutingPointSection],
) -> Option<Vec<ScoutingPointSection>> {
    let mut parsed = Vec::new();
    for section in base_sections {
        let mut parsed_options = Vec::new();
        for option in &section.options {
            let field_name = build_weight_field_name(&section.key, &option.key);
            let value = form.get(&field_name).map(|v| v.trim()).unwrap_or("");
            let mut parsed_value = option.points;
            if !value.is_empty() {
                let int_value: i32 = value.parse().ok()?;
                if !(-100..=100).contains(&int_value) {
                    return None;
                }
                parsed_value = int_value;
            }
            parsed_options.push(ScoutingPointOption {
                key: option.key.clone(),
                points: parsed_value,
            });
        }
        parsed.push(ScoutingPointSection {
            key: section.key.clone(),
            label: section.label.clone(),
            options: parsed_options,
        });
    }
    Some(parsed)
}
