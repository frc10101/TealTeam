# The load test (Q3)

Run this before the season, and again after any change to how saving or
syncing works. Thirty scouts' phones save observations, pull changes, and hold
live streams against a real `tt-web` for two hours. Meanwhile the cable is
pulled and the power cut, on purpose. A third check, separate from the
server's, fills a phone's storage.

Everything here is `crates/tt-load` (the server side) and
`crates/tt-web/tests/browser/storage-full.mjs` (the phone).

## What a simulated scout does

Each scout has its own connections, like a phone. Every 15 seconds, give or take a fifth:

1. Open the scouting page for its next match and robot.
2. Save an observation. A save that fails, or hangs for 10 seconds, is sent
   again every 2 seconds with **the same record id**, as a scout tapping Save
   again would.
3. Load the page the save redirects to. That is a new page, so its live
   stream is closed and reopened from `Last-Event-ID`, as a browser does.
4. Pull what changed since its last pull.

The tablet checks in once a minute, as `device.js` does. Scout 0 is also the
lead and refreshes the assignment grid each cycle.

That is about 120 saves a minute across the 30. A real event is about one a
minute: six robots per match, seven minutes apart. The test runs at roughly a
hundred times event load.

A stream that is silent for 35 seconds counts as dropped and is reopened. The
server sends a heartbeat every 15 seconds, so 35 seconds of silence means the
connection is dead.

## What it checks

| Verdict | Fails when |
| --- | --- |
| steady p95 within the bar (500 ms) | the slowest request type's 95th percentile, with faults left out, is over the bar |
| every confirmed save is on the server | a scout was told "saved" and the server does not have it |
| no save stored twice | a retried save made a second row |
| no save refused | the server answered a save with the form and an error |
| every save reached every stream once | a scout's open stream missed a save, or was sent one twice |
| no stream turned away | a stream got a 503 (too many connections). A browser's `EventSource` gives up on a 503 for good |
| nothing failed outside a fault | a request or stream failed when nothing had been done to the server (only checked when the tool injects the faults) |

Latency is reported per request type, twice. **Steady** leaves out
everything from the start of a fault to 30 seconds after it ends, and the
time around any failed request. **All** keeps everything, so the cost of a
fault shows up as well.

## On one machine

```
cargo build --release -p tt-web -p tt-load
target/release/tt-load run --server target/release/tt-web
```

This starts `tt-web` on a fresh database in a new temporary folder, seeds an
event (`2026load`, 36 teams, enough matches), and puts a relay between the
scouts and the server. Every 15 minutes it causes a fault, alternating between two kinds:

- **Cable pulled for 45 seconds.** The relay stops passing bytes. Nothing is
  refused and nothing arrives. Bytes in flight wait and are delivered when
  the cable goes back in. That includes saves whose scout has already given
  up, so the record id gets tested.
- **Power killed for 20 seconds.** SIGKILL, with the cable pulled as well, so
  the server goes silent rather than refusing. It restarts on the same
  database, and the cable goes back in once `/health` answers.

The run prints progress every five minutes and then the report. It writes the
report to `report.txt` beside the database and `server.log`. The exit status
is 0 only if every verdict passed. Run `tt-load help` for the options:
`--duration 20m` and `--faults-every 4m` make a quick check.

## Against the Pi

The numbers that matter come from the Pi, on its SSD, over the event
network. Never point this at the event database:

1. On the Pi, stop the service and start `tt-web` by hand on a scratch
   database on the SSD:
   `DATABASE_URL=sqlite:///srv/tealteam/data/loadtest.db?mode=rwc`.
2. Seed it: `tt-load seed sqlite:///srv/tealteam/data/loadtest.db`. Build
   `tt-load` the same way you build `tt-web` for the Pi.
3. From a laptop on the event network:
   `tt-load run --url http://<pi>:<port>`.
4. Do the faults by hand. At 15 minutes, pull the Pi's Ethernet cable for 45
   seconds. At 30 minutes, pull its power, then plug it back in. Repeat. The
   report lists every failed request with its time, so each fault shows up.
   The "nothing failed outside a fault" verdict is skipped, because the tool
   cannot know when you pulled what.
5. Delete `loadtest.db` and start the service again.

## A phone with full storage

```
cargo build -p tt-web -p tt-load && mkdir -p /tmp/full
cp target/debug/tt-web target/debug/tt-load /tmp/full/
node --experimental-websocket crates/tt-web/tests/browser/storage-full.mjs /tmp/full
```

This runs in headless Chromium. It fills localStorage until it refuses, turns
the origin's quota down to 4 MB, and fills that with OPFS files. Then it
checks the following:

- The scouting page works.
- The draft (C3) fails quietly.
- The save reaches the server.
- The offline shell (C1) does not install and nothing breaks.
- A snapshot (S10) that does not fit is refused with reason `full`.
- Once space is freed, both the snapshot and the shell succeed.

## Results

**2026-10-01, on a desktop** (32 cores, NVMe), release build, the defaults:
30 scouts, 2 hours, 7 faults.

Every verdict passed. The faults were the cable pulled at 0:15, 0:45, 1:15, and
1:45, and the power killed at 0:30, 1:00, and 1:30.

| Request | Steady n | p50 ms | p95 ms | p99 ms | max ms | Failed, all in faults |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| scouting page | 13,030 | 2 | 3 | 4 | 77 | 167 |
| save | 13,030 | 0 | 0 | 4 | 516 | 209 |
| page after save | 13,030 | 1 | 2 | 4 | 21 | 0 |
| pull | 13,030 | 0 | 0 | 1 | 7 | 0 |
| stream open | 13,060 | 0 | 0 | 1 | 11 | 0 |
| assignment grid | 434 | 39 | 85 | 89 | 98 | 0 |
| heartbeat | 2,904 | 0 | 0 | 1 | 56 | 44 |

- **Saves:** 14,228 confirmed, and the server holds exactly those 14,228.
  118 needed more than one attempt. None was lost, stored twice, or refused.
- **Streams:** 14,468 opened, one per page. 210 dropped, every one during a
  fault: 120 went silent while the cable was out, and 90 errored when the
  power was cut. Each save reached each of the 30 streams with a p50 of 2.5 s
  and a p95 of 3.0 s. That is the change log's two-second lag plus a
  one-second poll. The slowest took 48 s, across a fault. None was missed or
  sent twice.
- **No request or stream failed outside a fault.**

**The timed backups did not run in that run.** The server never creates its
backup folder (Q4), and the tool did not create one either. It does now. A
second run of 25 minutes, with backups on, is below.

**The same day: 25 minutes with backups on.** The cable was pulled at 0:08
and the power killed at 0:16. Every verdict passed: 2,964 saves confirmed
and held, and nothing failed outside a fault. The timed backup landed at
0:10, in the middle of the load, and the steady p95 stayed within 7 ms for
every request. Killing the power restarts the ten-minute timer, so the next
backup was due after the run ended.

The assignment grid is the slowest page, and it grows with the schedule. Its
p95 was 7 ms with 135 matches and 85 ms with the two-hour run's 610. Watch
that number on the Pi.

**The Pi has not been tested.** The desktop numbers show the design holds:
nothing was lost, duplicated, or missed, and every stream recovered. They say
little about the Pi's latency. Run the section above on the Pi before the
first event.

## `synchronous=FULL` (Q3b)

The server used to run SQLite with `synchronous=NORMAL`, and under that
setting a power cut can roll back the last commits: a scout told "saved"
could lose the save. It now runs `synchronous=FULL`, which flushes every
commit to the disk before answering. The cost is an fsync per commit.

**2026-10-01, the same desktop** (NVMe, btrfs), release build:

| Measure | NORMAL | FULL |
| --- | ---: | ---: |
| One commit on its own, 1,000 in a row: median | 0.01 ms | 0.56 ms |
| the same, p95 / p99 | 0.03 / 0.03 ms | 0.63 / 1.09 ms |
| `tt-load`, 10 minutes, 30 scouts, no faults: save p50 / p95 / p99 | 0 / 0 / 4 ms | 2 / 2 / 6 ms |
| the same, heartbeat p50 / p95 (it writes too) | 0 / 0 ms | 2 / 2 ms |
| the same, every other request p95 | 2 ms or under | 2 ms or under |

Both runs passed every verdict (1,197 and 1,206 saves). A save costs about
two milliseconds more, against a 500 ms bar, at a hundred times event load.
The Pi's SSD will be slower to fsync than this desktop's drive. Read the save
row when running the test on the Pi.

## What this does not test

- **A real power cut.** SIGKILL loses what the process held, but not data
  the kernel had yet to write to disk. The database now runs
  `synchronous=FULL` (above), so a confirmed save should be on the disk
  before the scout hears about it. Only pulling the plug on the Pi proves it,
  and only if the SSD honours the flush.
- Wi-Fi, a phone's CPU, or iOS Safari.
- More than one lead editing assignments or the pick list. The scouts here
  only save observations.
