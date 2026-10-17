//! What scouts saw a team do, summed up (U11, U12).
//!
//! # One rule (U12)
//!
//! The retired team page showed the *most common* answer for some fields and
//! the *most recent* for others, and nobody had chosen which: it was an
//! accident (REBUILD_SPEC.md 5.4). Here every field is summarised over every
//! approved observation on the current form version, by one rule per kind of
//! field:
//!
//! - a **choice** is a tally, most common first, so a lead sees "Played 5,
//!   Disabled 1" rather than a mode that hides how split the scouts were;
//! - a **counter** is its average and its best;
//! - a **yes/no** is how many times out of how many.
//!
//! Free text is not summarised: notes are read, not counted, and who may read
//! them is its own rule (U13).

use crate::season::{FieldKind, Payload, SeasonSchema, Value};

/// One field, summed up.
#[derive(Debug, Clone, PartialEq)]
pub enum Summary {
    /// Each option chosen at least once, with how often: most common first,
    /// ties in form order.
    Choice(Vec<(String, usize)>),
    Counter {
        average: f64,
        best: i64,
    },
    Toggle {
        yes: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldSummary {
    pub label: String,
    /// How many observations answered it.
    pub answered: usize,
    /// `None` when nobody answered it.
    pub summary: Option<Summary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionSummary {
    pub label: String,
    pub fields: Vec<FieldSummary>,
}

/// Sum up `payloads`, section by section in form order. Callers pass only
/// payloads recorded on `schema`'s version: another version's fields mean
/// different things.
pub fn summarize(schema: &SeasonSchema, payloads: &[&Payload]) -> Vec<SectionSummary> {
    schema
        .sections
        .iter()
        .map(|section| SectionSummary {
            label: section.label.clone(),
            fields: section
                .fields
                .iter()
                .filter(|f| !matches!(f.kind, FieldKind::Text { .. }))
                .map(|field| {
                    let answers: Vec<&Value> =
                        payloads.iter().filter_map(|p| p.get(&field.key)).collect();
                    let summary = (!answers.is_empty()).then(|| match &field.kind {
                        FieldKind::Select { options } => {
                            let mut tally: Vec<(String, usize)> = options
                                .iter()
                                .map(|o| {
                                    let n = answers
                                        .iter()
                                        .filter(|a| a.as_text() == Some(o.key.as_str()))
                                        .count();
                                    (o.label.clone(), n)
                                })
                                .filter(|(_, n)| *n > 0)
                                .collect();
                            // Stable: equal counts keep form order.
                            tally.sort_by_key(|entry| std::cmp::Reverse(entry.1));
                            Summary::Choice(tally)
                        }
                        FieldKind::Counter { .. } => {
                            let counts: Vec<i64> = answers
                                .iter()
                                .filter_map(|a| match a {
                                    Value::Count(n) => Some(*n),
                                    _ => None,
                                })
                                .collect();
                            Summary::Counter {
                                average: counts.iter().sum::<i64>() as f64
                                    / counts.len().max(1) as f64,
                                best: counts.iter().copied().max().unwrap_or(0),
                            }
                        }
                        FieldKind::Toggle { .. } => Summary::Toggle {
                            yes: answers
                                .iter()
                                .filter(|a| matches!(a, Value::Flag(true)))
                                .count(),
                        },
                        FieldKind::Text { .. } => unreachable!("filtered out above"),
                    });
                    FieldSummary {
                        label: field.label.clone(),
                        answered: answers.len(),
                        summary,
                    }
                })
                .collect(),
        })
        .filter(|s| !s.fields.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::current_season;

    fn payload(pairs: &[(&str, Value)]) -> Payload {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn field<'a>(sections: &'a [SectionSummary], label: &str) -> &'a FieldSummary {
        sections
            .iter()
            .flat_map(|s| &s.fields)
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("{label}"))
    }

    fn three() -> Vec<Payload> {
        vec![
            payload(&[
                ("robot_status", Value::Text("disabled".into())),
                ("launch_made", Value::Count(4)),
                ("teleop_park", Value::Flag(true)),
            ]),
            payload(&[
                ("robot_status", Value::Text("played".into())),
                ("launch_made", Value::Count(9)),
                ("teleop_park", Value::Flag(false)),
            ]),
            payload(&[
                ("robot_status", Value::Text("played".into())),
                ("launch_made", Value::Count(5)),
            ]),
        ]
    }

    #[test]
    fn a_choice_is_a_tally_most_common_first() {
        let schema = current_season().unwrap();
        let observations = three();
        let refs: Vec<&Payload> = observations.iter().collect();
        let summary = summarize(&schema, &refs);
        assert_eq!(
            field(&summary, "Robot status").summary,
            Some(Summary::Choice(vec![
                ("Played".into(), 2),
                ("Disabled".into(), 1)
            ]))
        );
    }

    #[test]
    fn a_counter_is_its_average_and_best_and_a_toggle_is_how_often() {
        let schema = current_season().unwrap();
        let observations = three();
        let refs: Vec<&Payload> = observations.iter().collect();
        let summary = summarize(&schema, &refs);

        assert_eq!(
            field(&summary, "Teleop launches made").summary,
            Some(Summary::Counter {
                average: 6.0,
                best: 9
            })
        );
        let broke = field(&summary, "Parked at the end");
        assert_eq!(broke.summary, Some(Summary::Toggle { yes: 1 }));
        assert_eq!(broke.answered, 2, "the third left it out");
    }

    #[test]
    fn nothing_answered_is_none_not_zero_and_notes_are_not_summarised() {
        let schema = current_season().unwrap();
        let summary = summarize(&schema, &[]);
        let auto = field(&summary, "Auto launches made");
        assert_eq!((auto.answered, &auto.summary), (0, &None));
        assert!(
            !summary
                .iter()
                .flat_map(|s| &s.fields)
                .any(|f| f.label == "Anything the numbers miss"),
            "free text is U13's to show"
        );
        assert!(
            summary.iter().all(|s| !s.fields.is_empty()),
            "a section of only notes is dropped"
        );
    }

    #[test]
    fn a_tied_tally_keeps_form_order() {
        // Disabled before no show in time, but no show comes first on the form.
        let observations = [
            payload(&[("robot_status", Value::Text("disabled".into()))]),
            payload(&[("robot_status", Value::Text("no_show".into()))]),
        ];
        let refs: Vec<&Payload> = observations.iter().collect();
        let summary = summarize(&current_season().unwrap(), &refs);
        assert_eq!(
            field(&summary, "Robot status").summary,
            Some(Summary::Choice(vec![
                ("No show".into(), 1),
                ("Disabled".into(), 1)
            ]))
        );
    }

    #[test]
    fn a_counter_average_keeps_its_fraction() {
        let observations = [
            payload(&[("launch_made", Value::Count(4))]),
            payload(&[("launch_made", Value::Count(5))]),
        ];
        let refs: Vec<&Payload> = observations.iter().collect();
        let summary = summarize(&current_season().unwrap(), &refs);
        assert_eq!(
            field(&summary, "Teleop launches made").summary,
            Some(Summary::Counter {
                average: 4.5,
                best: 5
            })
        );
    }

    #[test]
    fn never_ticked_is_zero_of_n_not_unanswered() {
        let observations = [
            payload(&[("teleop_park", Value::Flag(false))]),
            payload(&[("teleop_park", Value::Flag(false))]),
        ];
        let refs: Vec<&Payload> = observations.iter().collect();
        let summary = summarize(&current_season().unwrap(), &refs);
        let broke = field(&summary, "Parked at the end");
        assert_eq!(
            (broke.answered, &broke.summary),
            (2, &Some(Summary::Toggle { yes: 0 }))
        );
    }
}
