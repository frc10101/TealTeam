//! The connection chip (I11): this device's link to the TealTeam server.
//!
//! Not the server's internet -- that is [`crate::connectivity`], shown to lead
//! scouts as the server's uplink. A scout's question is "did what I entered get
//! there", and the answer depends only on whether *their tablet* can reach the
//! Pi. Mixing the two up is what made the retired app's "offline mode" so
//! confusing (REBUILD_SPEC.md 6.4).
//!
//! Offline is a state the app observes and reports, never a mode anyone
//! switches on. There is no toggle.
//!
//! The browser does the observing (`static/js/link.js`); this holds the rule
//! and the words, so the page can render every state the script may switch to,
//! and the WASM client (L-items) can call [`Link::classify`] directly.

/// The four states the chip can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Everything entered on this device is on the server.
    Synced,
    /// Connected, and something is on its way.
    Syncing,
    /// The server cannot be reached. `unsent` is what is kept on this device
    /// until it can be -- zero until there is an outbox (C5).
    Offline { unsent: usize },
    /// Connected, but `count` things need a person to decide: entries the
    /// server refused (C10), until a lead scout records or dismisses them.
    NeedsReview { count: usize },
}

impl Link {
    /// Which state to show. Unreachable wins: nothing else can be acted on
    /// until the server answers. Then sending, because a review count read
    /// before the send may be about to change.
    pub fn classify(reachable: bool, sending: bool, unsent: usize, needs_review: usize) -> Link {
        if !reachable {
            Link::Offline { unsent }
        } else if sending || unsent > 0 {
            Link::Syncing
        } else if needs_review > 0 {
            Link::NeedsReview {
                count: needs_review,
            }
        } else {
            Link::Synced
        }
    }

    /// Stable name for the markup and the script.
    pub fn key(self) -> &'static str {
        match self {
            Link::Synced => "synced",
            Link::Syncing => "syncing",
            Link::Offline { .. } => "offline",
            Link::NeedsReview { .. } => "review",
        }
    }

    /// The chip's text. "Offline" alone reads as "lost", and last season that
    /// sent scouts to writing on their hands, so it always says what is safe.
    pub fn label(self) -> String {
        match self {
            Link::Synced => "Synced".into(),
            Link::Syncing => "Syncing…".into(),
            Link::Offline { unsent: 0 } => "Offline · nothing unsent".into(),
            Link::Offline { unsent } => format!("Offline · {unsent} saved"),
            Link::NeedsReview { count: 1 } => "1 needs review".into(),
            Link::NeedsReview { count } => format!("{count} need review"),
        }
    }

    /// One sentence on what the state means to the person holding the device.
    pub fn meaning(self) -> String {
        match self {
            Link::Synced => "Everything you've entered is on the server.".into(),
            Link::Syncing => "Connected to the server, and sending what you entered.".into(),
            Link::Offline { unsent: 0 } => {
                "This device can't reach the TealTeam server. Everything you saved \
                 before is on it; wait for Synced before saving again."
                    .into()
            }
            Link::Offline { unsent } => format!(
                "This device can't reach the TealTeam server. {unsent} saved here \
                 will send when it reconnects."
            ),
            Link::NeedsReview { count: 1 } => {
                "The server refused something saved on a tablet. A lead scout decides \
                 what happens to it."
                    .into()
            }
            Link::NeedsReview { count } => format!(
                "The server refused {count} things saved on a tablet. A lead scout \
                 decides what happens to them."
            ),
        }
    }

    /// CSS modifier, so templates carry no state logic.
    pub fn css_class(self) -> &'static str {
        match self {
            Link::Synced => "badge-teal",
            Link::Syncing => "badge-blue",
            Link::Offline { .. } => "badge-amber",
            Link::NeedsReview { .. } => "badge-red",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_wins_over_everything() {
        assert_eq!(
            Link::classify(false, true, 4, 3),
            Link::Offline { unsent: 4 }
        );
        assert_eq!(
            Link::classify(false, false, 0, 0),
            Link::Offline { unsent: 0 }
        );
    }

    #[test]
    fn connected_states_in_order() {
        assert_eq!(Link::classify(true, true, 0, 3), Link::Syncing);
        assert_eq!(Link::classify(true, false, 2, 3), Link::Syncing);
        assert_eq!(
            Link::classify(true, false, 0, 3),
            Link::NeedsReview { count: 3 }
        );
        assert_eq!(Link::classify(true, false, 0, 0), Link::Synced);
    }

    #[test]
    fn offline_always_says_what_is_safe() {
        assert_eq!(Link::Offline { unsent: 4 }.label(), "Offline · 4 saved");
        assert_eq!(
            Link::Offline { unsent: 0 }.label(),
            "Offline · nothing unsent"
        );
        for unsent in [0, 1, 4] {
            assert_ne!(Link::Offline { unsent }.label(), "Offline");
        }
    }

    #[test]
    fn review_counts_read_as_english() {
        assert_eq!(Link::NeedsReview { count: 1 }.label(), "1 needs review");
        assert_eq!(Link::NeedsReview { count: 3 }.label(), "3 need review");
    }

    #[test]
    fn nothing_calls_it_a_mode() {
        let all = [
            Link::Synced,
            Link::Syncing,
            Link::Offline { unsent: 0 },
            Link::Offline { unsent: 2 },
            Link::NeedsReview { count: 1 },
        ];
        for link in all {
            let words = format!("{} {}", link.label(), link.meaning()).to_lowercase();
            assert!(!words.contains("mode"), "{words}");
            assert!(!words.contains("internet"), "{words}");
        }
    }
}
