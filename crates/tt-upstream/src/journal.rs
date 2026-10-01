//! The `upstream` log's feed (S1).
//!
//! The clients hand every response with new content to a [`Recorder`], after
//! it has parsed. They stay storage-free, because S4 compiles them to wasm32
//! for the browser. On the server the recorder is [`journal`]: a channel into
//! a task that appends to the `upstream` table. On a client it will be
//! whatever builds the bundle it pushes to the Pi (S5), which appends the
//! same rows there with its own `via`.
//!
//! The log sits beside the sync, not in its way: the sync still writes the
//! tables the pages read, exactly as before. A full or failing log costs a
//! warning, never a sync.

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
#[cfg(not(target_arch = "wasm32"))]
use tokio::{sync::mpsc, task::JoinHandle};
#[cfg(not(target_arch = "wasm32"))]
use tracing::warn;
#[cfg(not(target_arch = "wasm32"))]
use tt_repo::{NewUpstream, Repo};

/// A response worth logging: it parsed, and it is not a 304.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// `"first"` or `"tba"`.
    pub api: &'static str,
    /// The request, query included, parameters sorted.
    pub path: String,
    pub etag: Option<String>,
    pub body: Arc<str>,
    pub fetched_at: DateTime<Utc>,
}

/// Where a client sends what it fetched. Cheap to clone.
#[derive(Clone)]
pub struct Recorder(Arc<dyn Fn(Fetched) + Send + Sync>);

impl Recorder {
    pub fn new(record: impl Fn(Fetched) + Send + Sync + 'static) -> Self {
        Self(Arc::new(record))
    }

    pub(crate) fn record(&self, fetched: Fetched) {
        (self.0)(fetched)
    }
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Recorder")
    }
}

/// A recorder that appends to `repo`'s upstream log as `via = "pi"`, and the
/// task doing it. The task ends once every clone of the recorder is dropped.
#[cfg(not(target_arch = "wasm32"))]
pub fn journal<R: Repo + Send + Sync + 'static>(repo: Arc<R>) -> (Recorder, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Fetched>();
    let task = tokio::spawn(async move {
        while let Some(fetched) = rx.recv().await {
            let entry = NewUpstream {
                api: fetched.api.to_string(),
                path: fetched.path,
                etag: fetched.etag,
                body: fetched.body.to_string(),
                fetched_at: fetched.fetched_at,
                via: "pi".into(),
            };
            if let Err(e) = repo.append_upstream(&entry).await {
                warn!("upstream log: {} {}: {e}", entry.api, entry.path);
            }
        }
    });
    let recorder = Recorder::new(move |fetched| {
        // Closed only when the task is gone, at shutdown.
        let _ = tx.send(fetched);
    });
    (recorder, task)
}
