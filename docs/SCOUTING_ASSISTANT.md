# Scouting assistant: ask a question, answered from our data

*Feasibility and design, 2026-10-20. Item **AI1** in `ACTION_ITEMS.md`. Built
first on the FTC beta branch (`ftc-biobuzz`) for the Athens Qualifier
(Sat 2026-10-24), behind a setting that is off unless configured.*

A lead scout or drive coach types a question such as *"Which teams park
reliably in auto?"* or *"Compare 12345 and 23456 for a second pick"*. The
server answers in a sentence or two, using **only** what our scouts recorded
and what the event published, and shows the numbers it was given, so anyone
can check the answer.

This is not the chat that was vetoed (`RefurbishInstructions.md` §1, row 5).
That was people messaging each other. This is a read-only question box over
the data already on the Pi.

---

## 1. Verdict

**Feasible, with one hard limit: where the model runs.** The question box,
the data it reads, and the page are small and ordinary work. The limit comes
from rules we already set:

- **There is no internet at the venue.** The event LAN has no uplink, no
  radio, and no tether (`PI_NETWORK.md`, `EVENT_DAY_RUNBOOK.md` step 5). A
  hosted model such as Claude cannot be reached from the Pi there.
- **Scouting data may not leave by radio** (Open decision 1). A laptop that
  sends our observations to a hosted model over the venue's Wi-Fi or a phone
  hotspot breaks that rule, even if the laptop has a connection.
- **The Pi is the scouting server first.** A language model uses every core
  it is given. If it slows down saving a form, it has cost more than it is
  worth.

So **at the venue the model must run on hardware we bring, wired to the
LAN**, and **a hosted model is for use away from the venue**: the night
before, between event days, or after the event for the debrief and findings.

| Where the model runs | At the venue | Answer time (estimate) | Cost | Verdict |
| --- | --- | --- | --- | --- |
| **A team laptop on the wired LAN**, running `llama-server` (llama.cpp) with a 3B–8B model | Yes | A few seconds on a recent laptop, longer on a weak one | Free | **Best option at the venue**, if someone brings a laptop with a decent CPU, or better, a GPU or Apple silicon. Chromebooks cannot do this |
| **The Pi itself**, `llama-server` with a 1.5B model, held to 2 of the Pi's 4 cores | Yes | 10–15 tokens/s generation[^speed]; reading the data before answering is the slow part and is unmeasured, so probably 30 s to over a minute | Free | **Fallback.** It works, but it is slow, it competes with the server for CPU and memory, and a 1.5B model makes more mistakes |
| **Claude (Anthropic API)**, from the Pi or a laptop with internet | **No** (no uplink; no radio for event data) | Seconds | Cents per question[^cost] | **Best answers, off-site only**: event preparation, the debrief, `FTC_BETA_FINDINGS.md` |

[^speed]: Third-party Pi 5 measurements: about 10–14 tokens/s generation for
    Qwen 2.5 1.5B at Q4_K_M, and 4–6 tokens/s for 3B models. None measures
    prompt processing at these sizes; run `llama-bench` on our Pi before
    relying on it.
[^cost]: Data for one event is a few thousand tokens. At Claude Opus 5.5's
    $4/$20 per million input/output tokens, one question costs about
    $0.01–$0.03. Sonnet 5.5 or Haiku 5.5 cost less, if the team chooses one.

**Recommendation for Athens.** Build it with both backends, off by default.
At the venue, point it at a team laptop running `llama-server` if one is
available, or at the Pi as a fallback. Use Claude only off-site. Treat its
answers as a fast way to look things up, never as the pick list: the pick
list and the team pages stay the source of record.

---

## 2. Keeping answers grounded

Small local models are poor at arithmetic and like to sound sure. The design
does not ask the model to work anything out that the app can work out
itself:

1. **The server builds a data sheet** for the selected event, the same
   numbers the team, rankings and pick list pages show: each team's
   observation count and per-field summary (averages and bests for counters,
   tallies for choices, "yes 4 of 6" for toggles, from `profile::summarize`),
   our scout-estimate score and its rank (`ranking::team_scores`, with the
   lead's weight overrides), and the official rank, record, OPR, DPR and
   CCWM when an FTC Events pull has filled them. Only approved observations
   on the current form version count, as on the team page.
2. **Notes follow the notes rule (U13).** The sheet carries a scout's notes
   only when `Notes::for_viewer` would show them to the person asking,
   newest first, capped per team. Nothing a viewer could not open on the
   team page reaches the model.
3. **The model is told to answer only from the sheet,** to name the team
   numbers it relies on, to say how many observations a claim rests on, and
   to say plainly when the sheet does not answer the question.
4. **The page shows the sheet** under the answer, collapsed, with the time
   it was built and the number of observations behind it, so a lead can
   check any claim in seconds.

The model gets **no tools and no write access**. It cannot change data,
assignments or the pick list. That also keeps prompt injection harmless:
the worst a note saying "ignore your instructions" can do is spoil one
answer, which the sheet underneath will contradict.

The sheet is sized for a small model. A 30-team event comes to about 4,000
tokens without notes. Notes are capped so the whole sheet stays under a fixed
budget, and the page says when notes were cut.

---

## 3. What is in the build

- **`tt-core::ask`**: builds the data sheet from the event's teams,
  approved observations, schema, weights and published stats. It is pure
  and unit-tested: the notes rule, the cap, ordering, and an empty event.
- **`tt-web::ask`**: `GET /ask`, the question page, and `POST /ask`, which
  answers. It is for leads, coaches and admins. Without `ASK_BACKEND` set,
  the page says the assistant is not set up on this server, and nothing
  else changes.
- **Two backends**, chosen by `ASK_BACKEND`:
  - `local`: any OpenAI-compatible chat endpoint (`ASK_URL`, for example
    `http://10.101.0.20:8080` for a laptop running `llama-server`, and
    optionally `ASK_MODEL`);
  - `anthropic`: the Anthropic Messages API, using `ANTHROPIC_API_KEY` and
    `ASK_MODEL` (default `claude-opus-5-5`).
- **A hard timeout** (`ASK_TIMEOUT_SECS`, default 90). A slow or missing
  model gives "The assistant did not answer in time" or "The assistant at …
  could not be reached", never a hung page. The rest of the app does not
  wait on it.
- **One question at a time.** A second question while one is running is
  told to wait, so a queue of questions cannot pin the Pi's CPU.

Not built: conversation memory (each question stands alone), streaming the
answer word by word, and asking from a phone with no connection to the Pi.

**Checked:** core tests for the sheet (official and scouted numbers, the
notes rule, the budget counting notes it leaves out rather than cutting one
in half, a note that cannot break out of its line, another form version not
counting, scout-estimate order, and equal scores sharing a rank). Web tests
for the settings, a local model answering, a slow one timing out while a
second question is told to wait, nothing listening, Claude's refusal and its
text blocks, and the role guards. A run of the real binary against a
stand-in model on a seeded 4-team event: the answer and the sheet are on the
page, the sheet carries official rank and record and our notes, another
team's notes are not in it, and the lead scout, pick list and drive coach
pages link to it.

**Not checked:** a real model, local or Claude. How good the answers are,
and how long the Pi takes, are both unknown until someone runs `llama-server`
with a real model against a rehearsal event (B1).

---

## 4. Turning it on

**A team laptop on the LAN (the venue).** On the laptop, wired to the
switch:

```
llama-server -m qwen2.5-7b-instruct-q4_k_m.gguf --host 0.0.0.0 --port 8080 -c 8192
```

Then, in the Pi's `/opt/tealteam/.env`:

```
ASK_BACKEND=local
ASK_URL=http://<laptop's 10.101.0.x address>:8080
```

and `sudo systemctl restart tealteam`. The laptop needs a fixed lease, or its
address checked after it plugs in. Download the model file at home; there is
no internet at the venue.

**The Pi as fallback.** Install llama.cpp and a 1.5B model on the Pi
beforehand, run `llama-server` with `--threads 2` under `nice -n 10` on
`127.0.0.1:8080`, and set `ASK_URL=http://127.0.0.1:8080`.

**Claude, off-site.** `ASK_BACKEND=anthropic` and `ANTHROPIC_API_KEY=…` in
`.env`. Take the key out of `.env` before the event, so the Pi carries no
credential it should not use there.

---

## 5. Risks

| Risk | Mitigation |
| --- | --- |
| A wrong answer gets into a pick decision | The sheet is shown under every answer; the team pages and pick list stay the record; the B1 rehearsal checks answers against the pages |
| The model slows the server on the Pi | Prefer a laptop; on the Pi, 2 threads and `nice`; one question at a time; a timeout |
| Too few observations make a confident-sounding answer | Every team line carries its observation count, and the prompt makes the model say it |
| Notes reach someone who should not read them | The sheet uses the same `Notes::for_viewer` rule as every other page |
| Data leaves the LAN by radio | The `anthropic` backend is for off-site use only; the runbook says to take the key out before the event |
| A small model ignores the instructions | A 7B model on a laptop is markedly better than 1.5B on the Pi; the sheet underneath lets a person check |

## 6. Decisions for the team

1. **Is a laptop that can run a 7B model coming to Athens?** If none is, the
   assistant runs on the Pi, slowly, or not at all.
2. **Which roles may ask?** The build allows leads, coaches and admins.
3. **Claude at all?** The `anthropic` backend costs cents per question and
   needs a key that the team owns.
