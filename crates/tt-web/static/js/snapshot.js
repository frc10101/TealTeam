// A fresh device's first sync: one SQLite file into OPFS (S10).
//
// GET /api/sync/snapshot returns a database of what this viewer may see of
// the events asked for, with the cursors to pull from next inside it (its
// sync_state) and in the X-Sync-Cursor header. This writes it to the origin
// private file system as tealteam.sqlite3, where SQLite-WASM (C4) opens it as
// is, and the cursors beside it as tealteam-sync.json, for a sync client that
// has not opened the database yet.
//
// Nothing loads this yet: C4's browser repo is the first user. It is only
//   ttSnapshot.bootstrap({ events: ["2026mslr"] })  -> { changes, upstream, bytes, takenAt }
//   ttSnapshot.local()                              -> the cursors kept, or null
//
// It never replaces a copy already on the device unless told to with
// { replace: true }: that copy may hold what the outbox (C7) has not pushed.
// The file is written with createWritable, which swaps it in only when
// complete, so a download cut off half way leaves the old copy, or none.
//
// Only in a secure context: https, or the device itself. Over the event LAN's
// plain http there is no OPFS at all -- see open decision 9.
(function () {
  "use strict";

  var DB = "tealteam.sqlite3";
  var META = "tealteam-sync.json";
  var MAGIC = "SQLite format 3\u0000";

  function fail(reason, detail) {
    var e = new Error(detail || reason);
    e.reason = reason;
    return e;
  }

  function root() {
    if (!window.isSecureContext) return Promise.reject(fail("insecure", "OPFS needs https"));
    var storage = navigator.storage;
    if (!storage || !storage.getDirectory) return Promise.reject(fail("unsupported", "no OPFS"));
    return storage.getDirectory();
  }

  function exists(dir, name) {
    return dir.getFileHandle(name).then(
      function () { return true; },
      function () { return false; }
    );
  }

  function write(dir, name, data) {
    return dir.getFileHandle(name, { create: true }).then(function (file) {
      if (!file.createWritable) throw fail("unsupported", "this browser cannot write OPFS files here");
      return file.createWritable().then(function (out) {
        return out.write(data).then(function () { return out.close(); });
      });
    });
  }

  function schema() {
    var meta = document.querySelector('meta[name="tt-schema"]');
    return meta ? meta.content : null;
  }

  function local() {
    return root()
      .then(function (dir) { return dir.getFileHandle(META); })
      .then(function (file) { return file.getFile(); })
      .then(function (file) { return file.text(); })
      .then(JSON.parse)
      .catch(function () { return null; });
  }

  function bootstrap(options) {
    options = options || {};
    var dir;
    return root()
      .then(function (d) {
        dir = d;
        return options.replace ? false : exists(dir, DB);
      })
      .then(function (have) {
        if (have) throw fail("exists", "this device already has a copy");
        var query = new URLSearchParams();
        (options.events || []).forEach(function (e) { query.append("event", e); });
        var s = options.schema || schema();
        if (s) query.set("schema", s);
        return fetch("/api/sync/snapshot?" + query, { credentials: "same-origin" });
      })
      .then(function (response) {
        if (response.status === 409) {
          // Another schema (S11): "reload", or "server-behind".
          return response.json().then(function (body) { throw fail(body.action || "schema", body.error); });
        }
        if (!response.ok) throw fail("http", "snapshot: HTTP " + response.status);
        var cursor = (response.headers.get("X-Sync-Cursor") || "").split("-");
        return response.arrayBuffer().then(function (buffer) {
          var head = String.fromCharCode.apply(null, new Uint8Array(buffer, 0, Math.min(16, buffer.byteLength)));
          if (head !== MAGIC) throw fail("not-sqlite", "the answer is not a database");
          return {
            bytes: buffer,
            changes: Number(cursor[0]) || 0,
            upstream: Number(cursor[1]) || 0,
            takenAt: response.headers.get("X-Sync-Taken-At"),
          };
        });
      })
      .then(function (snap) {
        // The database first: cursors with no database behind them would
        // have a client pull past what it never got.
        return write(dir, DB, snap.bytes)
          .then(function () {
            return write(dir, META, JSON.stringify({
              changes: snap.changes,
              upstream: snap.upstream,
              takenAt: snap.takenAt,
              events: options.events || null,
              bytes: snap.bytes.byteLength,
            }));
          })
          .then(function () { return snap; });
      });
  }

  window.ttSnapshot = { bootstrap: bootstrap, local: local, DB: DB, META: META };
})();
