// Configurable scouting point weights used for team ranking scores.
// Port of internal/handlers/scouting_points.go / Services/ScoutingPointsService.cs.

use std::collections::HashMap;

use sqlx::PgPool;
use tracing::warn;

pub type WeightConfig = HashMap<String, HashMap<String, i32>>;

#[derive(Debug, Clone)]
pub struct ScoutingPointOption {
    pub key: String,
    pub points: i32,
}

impl ScoutingPointOption {
    pub fn label(&self) -> String {
        option_label(&self.key)
    }

    pub fn field_name(&self, metric_key: &str) -> String {
        build_weight_field_name(metric_key, &self.key)
    }
}

#[derive(Debug, Clone)]
pub struct ScoutingPointSection {
    pub key: String,
    pub label: String,
    pub options: Vec<ScoutingPointOption>,
}

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

pub fn normalize_option(value: &str) -> String {
    value.trim().to_lowercase()
}

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

pub fn build_weight_field_name(metric_key: &str, option_key: &str) -> String {
    format!("weight_{metric_key}__{option_key}")
}

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

/// Parses weight fields from the submitted form; None on invalid input.
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
