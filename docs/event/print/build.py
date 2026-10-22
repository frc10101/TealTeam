#!/usr/bin/env python3
"""Build the printable event papers as .docx (to edit) and .pdf (to print).

    python3 docs/event/print/build.py

Needs LibreOffice (`soffice`) and nothing else. Each paper is written here
as simple HTML that LibreOffice imports as a Writer document, then saved as
.docx and .pdf next to this script. The scouting sheet is generated from
the season schema (crates/tt-core/seasons/2026.json), so its labels and
field order match the app's form exactly.

The detail behind each paper stays in the Markdown docs it came from:
EVENT_DAY_RUNBOOK.md, SETUP_GUIDE.md (checklists, Chromebook card) and
FTC_BETA_PAPER_KIT.md. Change both when one changes.
"""

import html
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SCHEMA = ROOT / "crates/tt-core/seasons/2026.json"

STYLE = """
<style>
  body { font-family: Arial, Helvetica, sans-serif; font-size: 10pt; }
  h1 { font-family: Arial, Helvetica, sans-serif; font-size: 15pt; margin: 0 0 4pt 0; }
  h2 { font-family: Arial, Helvetica, sans-serif; font-size: 11pt; margin: 3pt 0 1pt 0; }
  p { margin: 0 0 4pt 0; }
  li { margin: 0 0 2pt 0; }
  code { font-family: "DejaVu Sans Mono", "Courier New", monospace; font-size: 9pt; }
  .small { font-size: 8.5pt; }
  .muted { color: #555555; }
</style>
"""


def page(title, body, size="letter", margin="0.5in"):
    return f"""<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{html.escape(title)}</title>
<style>@page {{ size: {size}; margin: {margin}; }}</style>
{STYLE}</head><body>
{body}
</body></html>
"""


def esc(s):
    return html.escape(s, quote=False)


BOX = "&#9744;"  # ☐


# ---------------------------------------------------------------- scouting sheet

def tick_boxes(n):
    """A grid of n small boxes in groups of 5, 20 to a row, plus a total cell."""
    rows = []
    per_row = 20
    for start in range(0, n, per_row):
        cells = []
        for i in range(start, min(start + per_row, n)):
            gap = ' style="border-left: 2pt solid #000"' if i % 5 == 0 and i % per_row else ""
            cells.append(f'<td width="16" height="12"{gap}>&nbsp;</td>')
        rows.append("<tr>" + "".join(cells) + "</tr>")
    grid = (
        '<table border="1" cellspacing="0" cellpadding="0" style="border-collapse: collapse">'
        + "".join(rows)
        + "</table>"
    )
    return grid


def field_html(f):
    label = f"<b>{esc(f['label'])}</b>"
    helptext = f' <span class="small muted">{esc(f["help"])}</span>' if f.get("help") else ""
    t = f["type"]
    if t == "toggle":
        choice = f"{BOX} Yes &nbsp; {BOX} No"
        return f'<tr><td width="62%">{label}{helptext}</td><td>{choice}</td></tr>'
    if t == "select":
        opts = " &nbsp; ".join(f"{BOX} {esc(o['label'])}" for o in f["options"])
        return f'<tr><td colspan="2">{label}{helptext}<br>{opts}</td></tr>'
    if t == "counter":
        top = f"{label} <span class=\"small muted\">(0&#8211;{f['max']})</span>{helptext}"
        return (
            f'<tr><td colspan="2">{top}'
            f'<table cellspacing="0" cellpadding="0"><tr><td>{tick_boxes(f["max"])}</td>'
            f'<td valign="bottom">&nbsp;&nbsp;Total: <b>_____</b></td></tr></table></td></tr>'
        )
    if t == "text":
        lines = 1 if f["max_len"] <= 500 else 2
        rule = "<br>" + "<br>".join("_" * 80 for _ in range(lines))
        return f'<tr><td colspan="2">{label}{helptext}{rule}</td></tr>'
    raise ValueError(f"unknown field type {t}")


def scouting_sheet():
    schema = json.loads(SCHEMA.read_text())
    header = f"""
<h1>Paper scouting sheet</h1>
<p class="small muted">Only when the app is down. One sheet per robot per match. Counters: tick a box per element (or tally beside them), then write the total. The lead types sheets into the app later, in this order.</p>
<table border="1" cellspacing="0" cellpadding="4" width="100%" style="border-collapse: collapse">
<tr><td width="30%"><span class="small">Scout name</span><br>&nbsp;</td>
<td width="12%"><span class="small">Match #</span><br>&nbsp;</td>
<td width="14%"><span class="small">Team #</span><br>&nbsp;</td>
<td width="22%"><span class="small">Alliance</span><br>{BOX} Red &nbsp; {BOX} Blue</td>
<td width="22%"><span class="small">Time started on paper</span><br>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;:</td></tr>
</table>
"""
    sections = []
    for s in schema["sections"]:
        rows = ""
        fields = s["fields"]
        i = 0
        while i < len(fields):
            f = fields[i]
            nxt = fields[i + 1] if i + 1 < len(fields) else None
            # Two short choices with no help text share a row.
            if f["type"] == "select" and not f.get("help") and nxt and nxt["type"] == "select" and not nxt.get("help"):
                a, b = (field_html(x)[len('<tr><td colspan="2">'):-len("</td></tr>")] for x in (f, nxt))
                rows += f'<tr><td width="50%">{a}</td><td>{b}</td></tr>'
                i += 2
                continue
            rows += field_html(f)
            i += 1
        sections.append(
            f'<h2>{esc(s["label"])}</h2>'
            f'<table border="1" cellspacing="0" cellpadding="3" width="100%" style="border-collapse: collapse">{rows}</table>'
        )
    return page("Paper scouting sheet", header + "".join(sections), margin="0.35in")


# ---------------------------------------------------------------- runbook

RUNBOOK = """
<h1>Event day: set up the scouting server</h1>
<p class="small muted">Laminate it and tape it to the Pi's case. First-time setup: SETUP_GUIDE.md. Network detail: PI_NETWORK.md. SSD and backups: PI_STORAGE.md.</p>

<table border="1" cellspacing="0" cellpadding="6" width="100%" style="border-collapse: collapse"><tr><td height="90" align="center" class="muted">[ PHOTO: the Pi, the switch and every cable plugged in correctly, each cable labelled ]</td></tr></table>

<p><b>Kit:</b> Pi (SSD attached) &#183; official 27 W Pi supply &#183; unmanaged switch and its power &#183; one flat Ethernet cable per Chromebook &#183; USB-C-to-Ethernet adapters &#183; gaff tape &#183; USB stick.
<b>No Wi-Fi hotspot, router, mesh, cellular tether or any radio network for event data, ever.</b></p>

<h2>Set up (about 10 minutes)</h2>
<ol>
<li><b>Cable before power.</b> Pi <code>eth0</code> &#8594; switch. Each Chromebook &#8594; switch (adapter only where needed). Tape every cable run flat.</li>
<li><b>Power on in order:</b> switch first, then the Pi. Wait about a minute for the green light to settle.</li>
<li><b>The server starts by itself.</b> <code>systemctl status tealteam</code> should say <code>active (running)</code>. If not: <code>sudo systemctl restart tealteam</code>; <code>journalctl -u tealteam -n 50</code> says why.</li>
<li><b>Check:</b> in the TealTeam folder, <code>deploy/pi/network/status.sh</code>. Every line <code>ok</code>; a <code>FAIL</code> names the fix. <code>no uplink</code> is fine.</li>
<li><b>No phone tether at the event.</b> The Pi runs without an uplink; rankings use the latest local snapshot.</li>
<li><b>From a Chromebook:</b> open <b>http://tealteam.local</b> (else <b>http://tealteam</b> or <b>http://10.101.0.1</b>).</li>
<li><b>Lead scout:</b> sign in, pick the event, check the rankings age. Scouting and lead work happen on the wired Chromebooks; phones only view.</li>
<li><b>Schedule posted:</b> on the Pi, <code>deploy/event/prep-event.sh deploy/profiles/ftc-qual-4robot.env --roster roster.csv --csv athens.csv</code>. Check every match against the printed schedule. If it changes, run it again: observations are kept.</li>
</ol>

<h2>Between match blocks: back up to the stick</h2>
<p>Plug in the stick, then <code>sudo /opt/tealteam/tt-web backup /media/$USER/STICK</code>. Wait for <b>&#8220;It is on the disk.&#8221;</b>, check the observation count looks right, eject, then pull it. (The server also snapshots to the SSD every 10 minutes; the stick is the copy that leaves the building.)</p>

<h2>If something is wrong</h2>
<table border="1" cellspacing="0" cellpadding="3" width="100%" style="border-collapse: collapse">
<tr><td width="34%"><b>Symptom</b></td><td><b>Do this</b></td></tr>
<tr><td>Chromebook can't open the page</td><td>Cable and adapter seated? Switch powered? Wi-Fi off? Try <code>http://10.101.0.1</code>. Run <code>status.sh</code>.</td></tr>
<tr><td><code>status.sh</code>: eth0 FAIL</td><td>Switch unpowered or the Pi's cable is loose.</td></tr>
<tr><td><code>status.sh</code>: app FAIL</td><td><code>sudo systemctl restart tealteam</code>; still failing &#8594; is the SSD plugged in?</td></tr>
<tr><td>Rankings stale (amber)</td><td>No uplink by design. Use the local snapshot; note when upstream data was last updated.</td></tr>
<tr><td>Pi lost power</td><td>Check the Pi's clock (<code>date</code>) once it is back up: it has no clock battery.</td></tr>
<tr><td>App down for more than a match</td><td>Switch to the paper sheets. Write the time on the paper-switch log.</td></tr>
<tr><td>FTA asks about wireless</td><td>There is none. Offer to unplug the phone; nothing else changes.</td></tr>
</table>

<h2>Tear down</h2>
<ol>
<li>Last backup to the stick, and eject it.</li>
<li><code>sudo poweroff</code> and wait for the green light to stop.</li>
<li>Unplug the Pi, then the switch. Count the cables, adapters and the stick.</li>
</ol>
"""


# ---------------------------------------------------------------- Chromebook card

CARD = f"""
<table border="1" cellspacing="0" cellpadding="8" width="100%" style="border-collapse: collapse"><tr><td>
<h1>TealTeam on a Chromebook</h1>
<p>Laptop: ______________ &nbsp;&nbsp; Scout: ______________________ &nbsp;&nbsp; Do these in order.</p>
<ol>
<li>Plug the USB-C Ethernet adapter into the Chromebook and the cable into the switch.</li>
<li>Turn <b>Wi-Fi off</b>: click the clock, click the Wi-Fi tile so it is grey. No hotspot, no Bluetooth tether.</li>
<li>Open Chrome and go to <b>http://tealteam.local/</b>. If it does not open, try <b>http://tealteam/</b>, then <b>http://10.101.0.1/</b>. Type <code>http://</code>, not <code>https://</code>.</li>
<li>Press Ctrl+D to bookmark it. Three dots &#8594; <b>Cast, save and share</b> &#8594; <b>Create shortcut</b> (tick <b>Open as window</b> if you like); right-click its icon &#8594; <b>Pin to shelf</b>.</li>
<li><b>Clear old data from earlier events:</b> go to <code>chrome://settings/content/all</code>, search <code>tealteam</code>, delete its data (and any <code>10.101.0.1</code>).</li>
<li><b>Sign in</b> with your name and the temporary password from the sign-in sheet, then change it at <b>Account</b>.</li>
<li><b>Keep the screen on:</b> Settings &#8594; <b>Device</b> &#8594; <b>Power</b> &#8594; <b>While charging</b>: <b>Keep display on</b>. Leave the charger plugged in.</li>
<li>Open your scouting page once so the offline copy loads, then wait for your match.</li>
</ol>
<p><b>If the page will not open:</b> check the cable and adapter, that the switch has power, and that Wi-Fi is off. Tell the lead scout.</p>
</td></tr></table>
"""


CUT = '<p align="center" class="small muted">&#9986; - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -</p>'


def cards():
    return page("Chromebook card", CARD + CUT + CARD)


# ---------------------------------------------------------------- using the laptops

# What a scout and the lead do once the Chromebook is set up. The button names
# are the app's own (submission.html, handoff.html, lead_scout.html,
# assignments.html, review.html); change them here when they change there.

def boxed(body):
    return (
        '<table border="1" cellspacing="0" cellpadding="8" width="100%" style="border-collapse: collapse"><tr><td>'
        + body
        + "</td></tr></table>"
    )


SCOUT_CARD = boxed("""
<h1>Scouting a match</h1>
<p>Laptop: ______________ &nbsp;&nbsp; Scout: ______________________ &nbsp;&nbsp; Every match, in this order.</p>
<ol>
<li><b>Sign in</b> at <b>http://tealteam.local/</b> (or <b>http://10.101.0.1/</b>) and press <b>Start scouting</b>. Keep this tab open all day.</li>
<li>The page says <b>You are scouting</b> and the team number, with <b>Your next assignment</b> at the top. Find that robot on the field before the match starts: red or blue, and its number.</li>
<li><b>Fill it in while the match runs.</b> Tap <b>+</b> each time it scores. What you type is kept as you go, so a reload loses nothing.</li>
<li>When the match ends, finish the notes and press <b>Save observation</b>. Then follow <b>Your next assignment</b> at the top.</li>
<li><b>Check the chip in the header:</b> <b>Synced</b> means the server has it. <b>Offline &#183; 1 held</b> means it is waiting on this laptop: <b>do not reload, click a link or close the tab</b> (the page will not come back until the server does), never clear the browser, tell the lead scout, and switch to paper.</li>
</ol>
<h2>If something is off</h2>
<ul>
<li><b>Wrong robot, or no assignment?</b> <b>Not your robot? Choose another</b>, or type the number in <b>Not in this match? Team number</b>. Tell the lead scout.</li>
<li><b>Declined</b> by the lead scout: read why, then press <b>Record it again</b>.</li>
<li><b>Missed the robot?</b> Tell the lead scout. Do not guess.</li>
<li><b>Server out of reach for a while?</b> Open <b>Server out of reach?</b>, press <b>Show the code</b>, and let the lead scout scan it.</li>
<li><b>App down?</b> The lead scout hands out paper sheets. Use one per robot per match.</li>
</ul>
""")


LEAD_CARD = boxed("""
<h1>Lead scout during matches</h1>
<p>Lead laptop: ______________ &nbsp;&nbsp; Sign in as the admin, then open <b>Lead Scout</b>.</p>
<h2>Before the first match</h2>
<ol>
<li><b>Lead Scout &#8594; Assignments.</b> Check that it says <b>N of N robots in upcoming matches have a scout</b>. If not, press <b>Hand out open robots</b>.</li>
<li>To change who watches a robot, tap the match, pick the scout and save. Nobody can have two robots in one match.</li>
</ol>
<h2>Every few matches</h2>
<ol>
<li><b>Waiting for review</b> (on <b>Lead Scout</b>): open the oldest. Press <b>Approve and see the next</b>, or write <b>Why decline it?</b> and press <b>Decline</b>. The scout sees the reason and records it again. <b>Put back to waiting</b> undoes a mis-tap.</li>
<li><b>Refused when sent</b>, if it shows: record each one against the right match, or dismiss it.</li>
<li><b>Assignments &#8594; Rotation:</b> swap out a scout who has been on too long.</li>
<li>Back up to the USB stick after every qualification round (the runbook on the Pi).</li>
</ol>
<h2>When things change</h2>
<ul>
<li><b>Schedule changed:</b> <b>Match schedule</b>, import the new CSV (observations are kept), then <b>Assignments</b>. Open each match it lists as changed and save it, then <b>Hand out open robots</b>.</li>
<li><b>Fewer scouts:</b> <b>Hand out open robots</b> again. Our team's robot is left for last.</li>
<li><b>A scout's laptop is offline:</b> <b>Scan a tablet</b>, scan their code, then let them scan the receipt.</li>
<li><b>App down:</b> paper sheets. Note the time in the checklist, and type the sheets in afterwards with <code>PAPER</code> in <i>Rule incidents</i>.</li>
</ul>
""")


def scout_cards():
    return page("Scouting a match", SCOUT_CARD + CUT + SCOUT_CARD)


def lead_cards():
    return page("Lead scout card", LEAD_CARD + CUT + LEAD_CARD)


# ---------------------------------------------------------------- checklists

def checklist(items):
    return "".join(f"<p>{BOX}&nbsp; {i}</p>" for i in items)


CHECKLISTS = (
    """
<h1>TealTeam event checklists &#8212; FTC Athens Qualifier, Sat 2026-10-24</h1>
<p class="small muted">1 Pi, 4 scout Chromebooks, 1 lead Chromebook, phones for viewing. Profile <code>deploy/profiles/ftc-qual-4robot.env</code>. Every command runs on the Pi.</p>
<h2>Friday</h2>
"""
    + checklist([
        "Check for a new Team Update that changes scoring.",
        "Chromebooks and phones wiped (old site data cleared) and charging.",
        "Phones: open <code>http://tealteam.local/</code> once on the wired network so the offline copy loads.",
        "Printed: runbook (laminated), one Chromebook card per laptop, paper scouting sheets for a full day, this page.",
        "Backup taken and a restore checked.",
    ])
    + "<h2>Night before</h2>"
    + checklist([
        "Pi built from the final tag; <code>status.sh</code> clean; <code>check-backup</code> ok; backup on the stick.",
        "Packed: switch, Pi supply, one cable per Chromebook, USB-C Ethernet adapters, gaff tape, USB stick.",
        "<code>roster.csv</code> filled in: real names, the lead as admin, the 4 scouts, team numbers.",
        "Phones charged; offline copy loaded.",
    ])
    + "<h2>At the venue</h2>"
    + checklist([
        "Cable before power; switch first, then the Pi; wait for the green light.",
        "<code>deploy/pi/network/status.sh</code> has no <code>FAIL</code>. Pi clock (<code>date</code>) is right.",
        "One Chromebook opens <code>http://tealteam.local/</code>; then all of them, per the card.",
        "Schedule posted (save as <code>athens.csv</code>, header <code>match,red1,red2,blue1,blue2</code>, <code>*</code> = surrogate): run <code>prep-event.sh &#8230; --roster roster.csv --csv athens.csv</code>.",
        "Every match checked against the printed schedule.",
        "Print <code>signin-sheet-*.txt</code>, hand it out; shred it after the event.",
        "Assignments says &#8220;N of N robots in upcoming matches have a scout&#8221;.",
        "One scout, one match, check the result; then the full crew.",
        "Backup to the stick after every qualification round.",
    ])
    + """
<h2>If the app goes down: paper</h2>
<p>Scouts switch to the paper sheets (one per robot per match). The lead writes each switch below.
Once the app is back, the <b>lead</b> types each sheet in, in the sheet's order, and adds <code>PAPER</code> to that observation's <i>Rule incidents</i>. Keep the sheets after entry.</p>
<table border="1" cellspacing="0" cellpadding="4" width="100%" style="border-collapse: collapse">
<tr><td width="18%"><b>Time to paper</b></td><td width="18%"><b>Time back</b></td><td width="18%"><b>Matches</b></td><td><b>What happened</b></td></tr>
"""
    + "<tr><td>&nbsp;</td><td></td><td></td><td></td></tr>" * 5
    + "</table>"
)


# ---------------------------------------------------------------- build

PAPERS = {
    "scouting-sheet": scouting_sheet,
    "event-day-runbook": lambda: page("Event day runbook", RUNBOOK, margin="0.45in"),
    "chromebook-card": cards,
    "scout-card": scout_cards,
    "lead-card": lead_cards,
    "event-checklists": lambda: page("Event checklists", CHECKLISTS),
}


def main():
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    if not soffice:
        sys.exit("build.py: needs LibreOffice (soffice) on the PATH")
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        profile = f"-env:UserInstallation=file://{tmp}/lo-profile"
        # `build.py scout-card lead-card` builds only those; no names, all.
        wanted = sys.argv[1:] or list(PAPERS)
        unknown = [n for n in wanted if n not in PAPERS]
        if unknown:
            sys.exit(f"build.py: no paper named {', '.join(unknown)}; try {', '.join(PAPERS)}")
        for name in wanted:
            make = PAPERS[name]
            src = tmp / f"{name}.html"
            src.write_text(make())
            for fmt in ("docx:MS Word 2007 XML", "pdf:writer_pdf_Export"):
                subprocess.run(
                    [soffice, profile, "--headless", "--infilter=HTML (StarWriter)",
                     "--convert-to", fmt, "--outdir", str(HERE), str(src)],
                    check=True, stdout=subprocess.DEVNULL,
                )
            print(f"wrote docs/event/print/{name}.docx and .pdf")


if __name__ == "__main__":
    main()
