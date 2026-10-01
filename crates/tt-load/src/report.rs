//! What a run measured, and whether it passed.
//!
//! Latency is reported twice. **Steady** leaves out every request made during
//! a fault and for [`SETTLE`] after it, and around every failed request, so it
//! is the server under load and nothing else; the p95 bar applies to it.
//! **All** keeps everything, so the cost of a fault is in the report too.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::sse::{StreamStats, WATCHDOG};

/// How long after a fault the server is given to recover.
pub const SETTLE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Sample {
    pub route: &'static str,
    /// Since the run started.
    pub at: Duration,
    pub took: Duration,
    pub ok: bool,
}

/// A fault this tool caused.
#[derive(Debug, Clone)]
pub struct Fault {
    pub what: &'static str,
    pub from: Duration,
    pub to: Duration,
}

/// Written to by every scout as the run goes.
pub struct Shared {
    start: Instant,
    pub samples: Mutex<Vec<Sample>>,
    /// Observation id → when its save was confirmed.
    pub acked: Mutex<HashMap<String, Duration>>,
    pub faults: Mutex<Vec<Fault>>,
    /// Saves the server refused with a page rather than a redirect.
    pub refusals: Mutex<Vec<String>>,
    /// Saves that needed more than one attempt.
    pub retried: Mutex<u32>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            samples: Mutex::default(),
            acked: Mutex::default(),
            faults: Mutex::default(),
            refusals: Mutex::default(),
            retried: Mutex::default(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Record a request that began at `started` and has just finished.
    pub fn sample(&self, route: &'static str, started: Instant, ok: bool) {
        let sample = Sample {
            route,
            at: started.duration_since(self.start),
            took: started.elapsed(),
            ok,
        };
        self.samples.lock().unwrap().push(sample);
    }
}

/// What the server holds at the end: observation id → times it was logged.
pub type Stored = HashMap<String, u32>;

/// The bars a run must clear.
pub struct Bars {
    pub p95: Duration,
    /// Whether the faults were this tool's. Against a server someone else is
    /// pulling cables on, a failed request is expected and only reported.
    pub injected: bool,
}

/// `p` (0-1] of `sorted`, nearest rank.
pub fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The spans to leave out of steady latency: each fault and its settling, and
/// around each failed request.
pub fn disturbed(faults: &[Fault], samples: &[Sample]) -> Vec<(Duration, Duration)> {
    let mut spans: Vec<(Duration, Duration)> = faults
        .iter()
        .map(|f| (f.from, f.to + SETTLE))
        .chain(
            samples
                .iter()
                .filter(|s| !s.ok)
                .map(|s| (s.at.saturating_sub(WATCHDOG), s.at + s.took + SETTLE)),
        )
        .collect();
    spans.sort();
    let mut merged: Vec<(Duration, Duration)> = Vec::new();
    for (from, to) in spans {
        match merged.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    merged
}

fn inside(spans: &[(Duration, Duration)], at: Duration) -> bool {
    spans.iter().any(|(from, to)| (*from..=*to).contains(&at))
}

/// A request that began or ended in a span: one already on its way when the
/// cable came out was caught by it too.
fn touched(spans: &[(Duration, Duration)], s: &Sample) -> bool {
    inside(spans, s.at) || inside(spans, s.at + s.took)
}

fn ms(d: Duration) -> String {
    format!("{}", d.as_millis())
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// The report, and whether the run passed.
pub fn summarise(
    shared: &Shared,
    streams: &[StreamStats],
    stored: &Stored,
    bars: &Bars,
) -> (String, bool) {
    let samples = shared.samples.lock().unwrap().clone();
    let faults = shared.faults.lock().unwrap().clone();
    let acked = shared.acked.lock().unwrap().clone();
    let refusals = shared.refusals.lock().unwrap().clone();
    let retried = *shared.retried.lock().unwrap();
    let spans = disturbed(&faults, &samples);
    let mut out = String::new();
    let mut passed = true;
    let mut verdict = |out: &mut String, ok: bool, line: String| {
        passed &= ok;
        let _ = writeln!(out, "{}  {line}", if ok { "PASS" } else { "FAIL" });
    };

    // ── Latency ──
    let _ = writeln!(
        out,
        "Latency, ms                    steady:     n    p50    p95    p99    max   all: p95  failed"
    );
    let mut routes: Vec<&'static str> = samples.iter().map(|s| s.route).collect();
    routes.sort_unstable();
    routes.dedup();
    let mut worst_p95 = (Duration::ZERO, "");
    for route in &routes {
        let of_route = samples.iter().filter(|s| s.route == *route);
        let mut all: Vec<Duration> = of_route.clone().filter(|s| s.ok).map(|s| s.took).collect();
        let mut steady: Vec<Duration> = of_route
            .clone()
            .filter(|s| s.ok && !touched(&spans, s))
            .map(|s| s.took)
            .collect();
        let failed = of_route.filter(|s| !s.ok).count();
        all.sort_unstable();
        steady.sort_unstable();
        let p95 = percentile(&steady, 0.95);
        if p95 > worst_p95.0 {
            worst_p95 = (p95, route);
        }
        let _ = writeln!(
            out,
            "  {route:<36} {:>6} {:>6} {:>6} {:>6} {:>6}   {:>8} {:>7}",
            steady.len(),
            ms(percentile(&steady, 0.5)),
            ms(p95),
            ms(percentile(&steady, 0.99)),
            ms(steady.last().copied().unwrap_or_default()),
            ms(percentile(&all, 0.95)),
            failed,
        );
    }

    // ── Faults and outages ──
    let _ = writeln!(out, "\nFaults injected");
    if faults.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for f in &faults {
        let _ = writeln!(out, "  {} to {}  {}", clock(f.from), clock(f.to), f.what);
    }
    let fault_spans: Vec<(Duration, Duration)> =
        faults.iter().map(|f| (f.from, f.to + SETTLE)).collect();
    let stray: Vec<&Sample> = samples
        .iter()
        .filter(|s| !s.ok && !touched(&fault_spans, s))
        .collect();
    let _ = writeln!(
        out,
        "\nFailed requests outside any injected fault: {}",
        stray.len()
    );
    for s in stray.iter().take(10) {
        let _ = writeln!(out, "  {}  {}", clock(s.at), s.route);
    }

    // ── Saves ──
    let missing: Vec<&String> = acked
        .keys()
        .filter(|id| !stored.contains_key(*id))
        .collect();
    let doubled = stored.values().filter(|n| **n > 1).count();
    let unconfirmed = stored.keys().filter(|id| !acked.contains_key(*id)).count();
    let _ = writeln!(
        out,
        "\nSaves: {} confirmed, {} needed a retry, {} refused; server holds {} \
         ({} it took but never confirmed)",
        acked.len(),
        retried,
        refusals.len(),
        stored.len(),
        unconfirmed,
    );
    for r in refusals.iter().take(5) {
        let _ = writeln!(out, "  refused: {r}");
    }

    // ── Streams ──
    let mut delivery: Vec<Duration> = Vec::new();
    let mut unseen = 0usize;
    for stream in streams {
        for (id, at) in &acked {
            match stream.seen.get(id) {
                Some(seen) => delivery.push(seen.saturating_sub(*at)),
                None => unseen += 1,
            }
        }
    }
    delivery.sort_unstable();
    let drops: Vec<&(Duration, String)> = streams.iter().flat_map(|s| &s.drops).collect();
    let stray_drops: Vec<&&(Duration, String)> = drops
        .iter()
        .filter(|(at, _)| !inside(&fault_spans, *at))
        .collect();
    let mut reasons: HashMap<&str, usize> = HashMap::new();
    for (_, why) in &drops {
        *reasons.entry(why.as_str()).or_default() += 1;
    }
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort();
    let refused = drops
        .iter()
        .filter(|(_, why)| why.starts_with("refused"))
        .count();
    let repeats: u32 = streams.iter().map(|s| s.repeats).sum();
    let _ = writeln!(
        out,
        "\nStreams: {} held, {} opened (one per page), {} dropped {:?}, {} outside a fault",
        streams.len(),
        streams.iter().map(|s| s.opened).sum::<u32>(),
        drops.len(),
        reasons,
        stray_drops.len(),
    );
    let _ = writeln!(
        out,
        "  each save reached each stream in: p50 {} ms, p95 {} ms, max {} ms; {} never arrived, {} arrived twice",
        ms(percentile(&delivery, 0.5)),
        ms(percentile(&delivery, 0.95)),
        ms(delivery.last().copied().unwrap_or_default()),
        unseen,
        repeats,
    );

    // ── Verdict ──
    let _ = writeln!(out);
    verdict(
        &mut out,
        worst_p95.0 <= bars.p95,
        format!(
            "steady p95 {} ms (slowest: {}) within {} ms",
            ms(worst_p95.0),
            worst_p95.1,
            ms(bars.p95)
        ),
    );
    verdict(
        &mut out,
        missing.is_empty(),
        format!(
            "every confirmed save is on the server ({} missing)",
            missing.len()
        ),
    );
    verdict(
        &mut out,
        doubled == 0,
        format!("no save stored twice ({doubled})"),
    );
    verdict(
        &mut out,
        refusals.is_empty(),
        format!("no save refused ({})", refusals.len()),
    );
    verdict(
        &mut out,
        unseen == 0 && repeats == 0,
        format!("every save reached every stream once ({unseen} missed, {repeats} repeated)"),
    );
    verdict(
        &mut out,
        refused == 0,
        format!("no stream turned away ({refused}); a browser's EventSource gives up on one"),
    );
    if bars.injected {
        verdict(
            &mut out,
            stray.is_empty() && stray_drops.is_empty(),
            format!(
                "nothing failed outside a fault ({} requests, {} streams)",
                stray.len(),
                stray_drops.len()
            ),
        );
    }
    (out, passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let v: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        assert_eq!(percentile(&v, 0.95), Duration::from_millis(95));
        assert_eq!(percentile(&v, 0.5), Duration::from_millis(50));
        assert_eq!(percentile(&v, 1.0), Duration::from_millis(100));
        assert_eq!(percentile(&v[..1], 0.95), Duration::from_millis(1));
        assert_eq!(percentile(&[], 0.95), Duration::ZERO);
    }

    #[test]
    fn a_fault_and_a_failure_inside_it_are_one_span() {
        let faults = [Fault {
            what: "cable",
            from: d(100),
            to: d(160),
        }];
        let failed = Sample {
            route: "save",
            at: d(150),
            took: d(10),
            ok: false,
        };
        let lone = Sample {
            at: d(1000),
            ..failed.clone()
        };
        let spans = disturbed(&faults, &[failed, lone]);
        assert_eq!(spans, vec![(d(100), d(190)), (d(965), d(1040))]);
        assert!(inside(&spans, d(189)));
        assert!(!inside(&spans, d(191)));
    }

    #[test]
    fn a_request_on_its_way_when_the_cable_came_out_is_the_fault_s() {
        let spans = [(d(100), d(190))];
        let caught = Sample {
            route: "save",
            at: d(99),
            took: d(10),
            ok: false,
        };
        assert!(touched(&spans, &caught));
        let before = Sample {
            took: Duration::from_millis(5),
            ..caught.clone()
        };
        assert!(!touched(&spans, &before));
    }

    #[test]
    fn a_run_with_a_lost_save_fails() {
        let shared = Shared::new();
        shared.acked.lock().unwrap().insert("a".into(), d(1));
        shared.acked.lock().unwrap().insert("b".into(), d(2));
        let mut stream = StreamStats::default();
        stream.seen.insert("a".into(), d(3));
        stream.seen.insert("b".into(), d(4));
        let bars = Bars {
            p95: Duration::from_millis(500),
            injected: true,
        };

        let whole: Stored = [("a".to_string(), 1), ("b".to_string(), 1)].into();
        let (_, passed) = summarise(&shared, std::slice::from_ref(&stream), &whole, &bars);
        assert!(passed);

        let lost: Stored = [("a".to_string(), 1)].into();
        let (text, passed) = summarise(&shared, &[stream], &lost, &bars);
        assert!(!passed);
        assert!(
            text.contains("FAIL  every confirmed save is on the server (1 missing)"),
            "{text}"
        );
    }
}
