# Claude Code Pet 🪙 — Architecture & Reference

> An ambient Linux desktop **pet** that tracks your Claude Code sessions. A small
> always-on-top creature sits on your desktop; click it to open a panel that
> shows your recent/running Claude Code sessions and a per-prompt token/cost/time
> scoreboard for each one.

This document explains **the goal, the architecture, how every piece works, the
data model, and the design decisions** behind the project. It is the single
reference for understanding the codebase end-to-end.

---

## 1. The Goal

Claude Code writes a transcript of every session to disk as JSONL. Each line is
one event (your typed prompt, an assistant API response with token usage, tool
calls, metadata, etc.). That data is rich but invisible — there's no built-in way
to glance at *"how many sessions do I have running, which one is waiting on me,
and how many tokens / dollars has each prompt cost?"*

The Pet turns that hidden data into an **ambient, always-visible desktop
companion**:

1. **First click on the pet** → a list of your recent / running sessions
   (live ones marked with a colored ● dot).
2. **Click a session** → its prompt-by-prompt scoreboard: each prompt you typed,
   its output-token cost, and elapsed time, with the heaviest prompts colored.
3. **Next click on the pet** → it jumps straight back to the **last session you
   opened**, with a `‹ Sessions` button to go back to the list.

On top of pure data display, it has **personality** (wanders the screen, demands
petting) and **workflow integration** (open/focus/start sessions in WezTerm,
attention alerts when a session finishes and is waiting on you).

Everything reads Claude Code's own transcripts at
`~/.claude/projects/<project>/*.jsonl` — **there is nothing to configure**.

### Why a GTK app and not conky?
The project started as a conky text overlay, but conky can only *paint* text — it
can't react to clicks or show navigable panels. The pet is a small **GTK3 app**
instead, which gives transparency, click handling, and popups. The
transcript-parsing logic is kept separate and stdlib-only so it can be tested
headlessly.

---

## 2. High-Level Architecture

The project is deliberately split into **two layers** with a hard boundary:

```
┌──────────────────────────────────────────────────────────────┐
│                          pet.py  (GUI)                         │
│   GTK3 / PyGObject / cairo                                     │
│                                                                │
│   ┌─────────┐    ┌──────────┐    ┌──────────────────────────┐ │
│   │  App    │───▶│   Pet    │    │         Panel            │ │
│   │ (glue)  │    │ (sprite, │    │ (session list +          │ │
│   │         │    │ anim,    │    │  prompt-history views)   │ │
│   │ state,  │    │ person-  │    │                          │ │
│   │ polling)│    │ ality)   │    │                          │ │
│   └────┬────┘    └──────────┘    └────────────┬─────────────┘ │
│        │                                       │               │
└────────┼───────────────────────────────────────┼──────────────┘
         │            calls (pure data)           │
         ▼                                         ▼
┌──────────────────────────────────────────────────────────────┐
│                      sessions.py  (data layer)                 │
│   Python STDLIB ONLY. No GUI, no markup.                       │
│                                                                │
│   find_sessions()  parse_session()  wezterm_panes()            │
│   fmt_tokens()  fmt_elapsed()  fmt_cost()  today_totals()      │
└────────────────────────────┬───────────────────────────────────┘
                             │ reads
                             ▼
        ~/.claude/projects/<project-dir>/*.jsonl   (transcripts)
        wezterm cli list --format json             (live panes)
```

- **`sessions.py`** — the **data layer**. Pure Python standard library, no GUI, no
  external packages. It reads the JSONL transcripts and returns plain Python
  dicts. It is robust against partial/empty/garbage files and never raises out of
  its public functions. Can be run standalone (`python3 sessions.py`) as a smoke
  test.
- **`pet.py`** — the **presentation layer**. A GTK3/PyGObject/cairo application
  that renders the data however it likes and handles all interaction, animation,
  and personality. It depends on `sessions.py` but `sessions.py` knows nothing
  about it.

This separation means the parsing logic can be verified without a display
(`python3 sessions.py`), and the GUI never has to worry about transcript edge
cases.

### Runtime state
A small JSON file at `~/.config/claude-pet/state.json` (created at runtime)
persists pet position, last-opened session, pinned sessions, and last-petted
time. Nothing else is written outside the repo except an optional XDG autostart
`.desktop` file.

---

## 3. Files in the Repository

| File | Purpose |
|------|---------|
| `pet.py` | The GTK3 pet + panel application (presentation, animation, personality, WezTerm integration). |
| `sessions.py` | Transcript parser / data layer. Python stdlib only. Also a CLI smoke-test. |
| `assets/pet.gif` | The pet sprite. Drop `pet.gif` (animated), `pet.png`, `pet.jpg`, or `pet.webp` here for custom art; otherwise a built-in coin mascot is drawn. |
| `assets/README.txt` | Notes on the art slot. |
| `install.sh` | Dependency check (GTK3/PyGObject) + optional apt-install + optional login autostart entry. |
| `run.sh` | Convenience launcher — kills any running instance, then starts `pet.py`. |
| `README.md` | User-facing docs (install, features, troubleshooting). |
| `state.json` | *(created at runtime in `~/.config/claude-pet/`)* — not in repo. |

### Not part of the pet app
The following are **external agent-tooling scaffolding** (added by an "odin init"
step) and are unrelated to the pet's functionality:
`.odin/`, `.codex/`, `.gemini/`, `.kilocode/`, `.qwen/`, `.claude/`, `.mcp.json`,
`opencode.json`, `.env.example`. They configure various AI coding assistants and
can be ignored when reasoning about how the pet works.

---

## 4. The Data Layer (`sessions.py`) in Detail

### 4.1 Public API

```python
find_sessions(limit=25, detect_open=True) -> list[dict]   # recent sessions, newest first
parse_session(path)                        -> dict | None   # one session, fully parsed (mtime-cached)
fmt_tokens(n) / fmt_elapsed(seconds) / fmt_cost(cost)       # display helpers
today_totals(sessions_list) / is_today(s)                   # "today" filter helpers
wezterm_panes()                                             # live WezTerm pane info
```

### 4.2 Session discovery — `find_sessions()`
- Globs **one level deep**: `~/.claude/projects/*/*.jsonl`. This is deliberate —
  nested subagent transcripts live at
  `projects/*/<id>/subagents/agent-*.jsonl`, and the shallow glob ensures they
  are **never mistaken for top-level sessions**.
- Sorts paths by file **mtime**, newest first; caps to `limit` (pass `limit=None`
  to return ALL sessions, used by search).
- For each path, calls `parse_session()`, then sets per-call live/open flags:
  - `is_live` = file modified within `LIVE_WINDOW` (default **120s**).
  - `open` / `pane_id` / `working` / `waiting` = filled by matching against live
    WezTerm panes (see §4.6). These are recomputed fresh on every call so they
    are never stale.

### 4.3 Session parsing — `parse_session(path)`
Reads every JSONL line (skipping blanks and unparseable lines — e.g. a partial
last line during a live write) and walks the events to build a session dict. Key
logic:

**Detecting real prompts (`_is_real_prompt`)** — the central heuristic. A "real
prompt" is something *you typed*, not a tool result, command output, or
system-injected message:
- Primary signal: `type == "user"` and `promptSource == "typed"`.
- Legacy fallback (older transcripts without `promptSource`): a user message
  whose text doesn't start with meta prefixes like `<local-command`,
  `<bash-input`, `<command-`, `caveat:`, `[request interrupted`, etc., and isn't
  a `tool_result` block.
- `isSidechain` entries (subagent turns) are excluded.

**Grouping into turns** — each real prompt starts a new "turn." Every subsequent
`assistant` event is attributed to the current turn until the next real prompt.

**Token accounting (the important trap)** — a single assistant response **streams
as several JSONL lines sharing one `message.id`**, with the `usage` block
repeated on each. So tokens are **deduped by `message.id`**: the first time a
given id is seen, its usage is counted; repeats are ignored. Per-prompt number =
summed `usage.output_tokens` over unique message ids.

Session-wide token totals (used for cost) capture the full breakdown per unique
message id:
- `output_tokens`, `input_tokens`
- `cache_read_input_tokens` (`cr`)
- cache-creation split into `ephemeral_5m_input_tokens` (`cw5`) and
  `ephemeral_1h_input_tokens` (`cw1h`); if no breakdown is present, the legacy
  `cache_creation_input_tokens` is treated as 5-minute cache writes.

**Completion / running state**:
- A turn is `completed` if an assistant event for it has
  `stop_reason ∈ {end_turn, stop_sequence, max_tokens}`.
- The newest turn with **no assistant response yet** is marked `running` (this is
  what drives the "running…" badge in the UI).

**Timing**:
- Per-prompt `elapsed` = last assistant timestamp − the prompt's timestamp.
- Session `wall_seconds` = max(all end/start times) − min(start times).

**Metadata harvested along the way**: `cwd`, `gitBranch`, `sessionId`, `slug`,
`aiTitle`, and `model`. The display **title** is chosen in priority order:
`aiTitle` → `slug` → first line of the first prompt (≤60 chars) → first 8 chars
of session id / filename.

**mtime caching**: `parse_session` caches its result keyed by file mtime
(`_cache: path -> (mtime, result)`), so re-parsing an unchanged file is free —
critical because the panel refreshes every couple of seconds.

### 4.4 Cost estimation
`PRICING` is a table of approximate **USD per million tokens** per model family
(`opus`, `sonnet`, `haiku`, `fable`), each with input / output / cache-write-5m /
cache-write-1h / cache-read rates. `_model_family()` maps a model id string to a
family (default `opus`). Session cost =
`(in·in_rate + out·out_rate + cr·cr_rate + cw5·cw5_rate + cw1h·cw1h_rate) / 1e6`.
Edit `PRICING` if Anthropic pricing changes.

### 4.5 The session & prompt dicts

**Session dict** (returned by `parse_session` / `find_sessions`):
```
path, session_id, cwd, project, branch, title, model, model_family,
mtime, is_live, open, pane_id, working, waiting,
total_prompts, total_tokens, tokens{in,out,cr,cw5,cw1h}, cost,
last_prompt_ts, last_completed, wall_seconds, prompts[]
```

**Prompt dict** (each entry in `prompts[]`):
```
index, title, full_text, out_tokens, elapsed, running, completed
```

### 4.6 WezTerm live-status integration
- `wezterm_panes()` runs `wezterm cli list --format json` (4s timeout) and returns
  a list of `{pane_id, title, cwd, glyph}`. Returns `[]` if WezTerm isn't
  reachable, so the feature degrades gracefully.
- `glyph` is the first character of the pane title if it's non-alphanumeric —
  Claude Code shows a **braille spinner** glyph in the tab title while it's
  working. `_SPINNER` is the set of those spinner glyphs.
- `_annotate_open()` matches a session to a pane when the pane's normalized `cwd`
  equals the session's `cwd` **and** the session title appears in the pane title.
  On a match it sets:
  - `open = True`, `pane_id`
  - `working = True` if the pane glyph is a spinner (Claude is actively working)
  - `waiting = True` if it's open, not working, and the last turn completed —
    i.e. Claude finished and is **waiting for you**.

> Only sessions opened *in WezTerm* are detectable as active. Without WezTerm,
> sessions still show; they just never get the working/waiting/open flags.

### 4.7 v1 limitation (documented in code)
**Subagent / Task token usage is ignored in v1.** Those tokens live in separate
`subagents/agent-*.jsonl` files (and are excluded from discovery by the
one-level glob). A marked comment near the top of `sessions.py` explains exactly
where to fold them in for v2: for each `Task` tool_use in a turn, find the
matching subagent file and add its assistant output tokens to that turn.

---

## 5. The Presentation Layer (`pet.py`) in Detail

`pet.py` has three GTK classes plus module-level helpers.

### 5.1 `App` — application glue
- Loads `state.json`, installs the CSS, creates the `Pet` and `Panel` windows.
- Maintains shared state:
  - `attention` — set of session paths whose Claude just finished and is waiting.
  - `_att_working` — last-seen `working` flag per path (for transition detection).
  - `_snapshot` — most recent `find_sessions()` result, shared with the panel.
- `_poll_attention()` (every `ATTENTION_POLL_S` = **5s**): refreshes the snapshot
  and detects sessions that transitioned **working → waiting** within
  `ATTENTION_WINDOW_S` (**600s**). New attention triggers `pet.bounce()`. When a
  session goes back to working, its flag clears.
- `toggle_panel()` opens/closes the panel near the pet.

### 5.2 `Pet` — the sprite window
A borderless, transparent, always-on-top, click-through-to-taskbar-skipping GTK
window holding a single `Gtk.DrawingArea` painted with **cairo**.

**Art pipeline** (`_load_art`, `_process`, `_refresh_frame`, `_fit`, `dematte`):
- Picks up `assets/pet.{gif,png,jpg,jpeg,webp}`; with none, draws a built-in
  coin mascot (`_draw_placeholder`).
- Animated GIFs play via the pixbuf animation **real-time iterator** (advances by
  wall-clock, so timing is deterministic and correct).
- Each unique frame is **scaled to ≤`PET_MAX` (110px)** then **edge-de-matted**
  once and cached by pixel-hash. De-matte (`dematte`) erodes the dark "halo" many
  GIFs/PNGs bake around the subject: dark pixels touching transparency are made
  transparent, `TRIM_ITERS` layers deep, without holing the interior. Toggle with
  `TRIM_EDGE`.

**Drawing (`_on_draw`)** composites, in order:
1. The sprite (or placeholder), offset by a gentle sinusoidal **bob**, an upward
   **bounce** (attention hop), and a decaying side **wiggle**, scaled by `scale`.
2. A "pet me!" / "more! N" / "yay! ♥" banner while in petting/celebrate states.
3. Floating **heart** particles.
4. A status indicator in the corner: a red **`!` badge** (with count) when any
   session needs attention, **or** a small green dot if any session is live.

**Animation tick (`_tick`, every `TICK_MS` = 40ms)**: advances the bob counter,
the GIF frame, the position/scale animation, heart particles, and the decaying
wiggle/bounce; then queues a redraw.

**Position/scale animation (`_start_anim` / `_step_anim`)**: a single descriptor
interpolates center-x, center-y, and scale with a smoothstep ease, resizing and
moving the window each frame. Used for teleport glides and grow/shrink.

**Personality state machine** — `state ∈ {normal, petting, celebrate}`:
- *Wandering*: every `TELEPORT_MIN_S`–`TELEPORT_MAX_S` (10–30 min) the pet glides
  to a random on-screen spot (`GLIDE_MS` = 900ms) — but never while busy, being
  petted, or while the panel is open.
- *Petting*: every `PET_INTERVAL_S` (~3h, checked every `PET_CHECK_S`=60s) it
  enters "petting" — grows (`PET_GROW_SCALE`=2.6×) and glides to screen center,
  shows "pet me!", and **clicks become pets** instead of opening the panel. After
  `PETS_REQUIRED` (10) clicks it **celebrates** (hearts + wiggle), then shrinks
  back home and records `last_pet_time` in state.

**Click vs drag (`_on_press`/`_on_motion`/`_on_release`)**:
- Movement beyond `DRAG_THRESHOLD` (6px) is a **drag** → moves the window and
  saves `pet_x`/`pet_y` to state.
- A clean click → toggles the panel (or counts as a pet when in petting state).

**Debug mode**: `CLAUDE_PET_DEBUG=1` shrinks all the personality timers (teleport
every few seconds, petting after ~20s) so behaviors are easy to test.

### 5.3 `Panel` — the session list & prompt-history window
A second borderless, transparent, always-on-top window. Has **two views** driven
by `self.view`:

**`list` view (`show_list` / `_populate_results`)** — the session browser:
- A persistent **search bar** filters by title / project / branch / cwd
  (`_matches`). When searching, it pulls ALL sessions (`limit=None`).
- A **"Today"** toggle filters to today's sessions and shows daily totals in the
  footer (`today_totals`).
- A **"+ New"** button pops a menu of recent project folders (or a folder chooser)
  to start a fresh `claude` session in a WezTerm tab.
- Results are grouped into sections: **PINNED** (★, persisted in state) →
  **ACTIVE** (open in WezTerm; sorted working → waiting → other) → **RECENT** (or
  **MATCHES** when searching).
- Each **session row** shows a status dot (green=working / amber=waiting /
  grey=off), the title, a pin toggle, a **`⮒ open`/`⮒ focus`** button, and a meta
  line (`project ⎇ branch … cost · N · tok`).

**`history` view (`show_history` / `_prompt_row`)** — the per-session scoreboard:
- Header with a `‹ Sessions` back button and the session title.
- One row per prompt: a caret (if expandable), the index, the prompt text
  (2-line preview, **click to expand** to the full selectable text), and a
  right-hand metric column showing either a green **"running…"** badge or the
  output-token count (color-coded by heat) + elapsed time.
- **Token heat** (`heat_class`): tokens ≥66% of the session's max prompt →
  `tok-heavy` (red), ≥33% → `tok-mid` (amber), else `tok-light` (blue).
- Footer summary box: total prompts, total output tokens, wall-clock time.

**Live refresh (`_refresh`, every `REFRESH_SECS`=2s while visible)** rebuilds the
current view in place, preserving scroll position (`keep_scroll`) and search
focus. The mtime cache in `sessions.py` keeps this cheap.

**Opening sessions (`open_in_wezterm`)**:
- If the session is already open → `wezterm cli activate-pane` focuses that tab.
- Otherwise → `wezterm cli spawn` a new tab in the session's cwd running
  `claude --resume <id> ; exec bash` (so the tab stays open), falling back to a
  brand-new `wezterm start` window.
- Opening or clicking a session **clears its attention flag**.

### 5.4 Module-level helpers worth knowing
- `load_state` / `save_state` — JSON read/write of `~/.config/claude-pet/state.json`.
- `apply_rgba` / `clear_transparent` — give windows a true RGBA visual and clear
  the cairo surface to full transparency (needs a compositor to actually show).
- `monitor_geometry` — work-area of the current monitor, for clamping/placement.
- `HeatBar` — a small rounded cairo sparkline widget (also reused to draw the
  pet's banner pill).

---

## 6. State File (`~/.config/claude-pet/state.json`)
Persisted keys:
| Key | Meaning |
|-----|---------|
| `pet_x`, `pet_y` | Last pet window position. |
| `last_session` | Path of the last session opened in the panel (so the next click reopens it). |
| `pinned` | List of pinned session paths (shown in the PINNED section). |
| `last_pet_time` | Unix time the pet was last successfully petted (drives the ~3h cycle). |

---

## 7. Requirements & Install

**Requirements**
- Linux with **X11** (works under GNOME/Mutter, KDE, XFCE, …). True transparency
  needs a compositor (GNOME/KDE/XFCE composite by default; on a bare WM run
  `picom &`).
- **GTK3 + PyGObject**: `python3-gi`, `gir1.2-gtk-3.0`, `python3-gi-cairo`. No pip
  packages.
- Python 3.
- *(Optional)* **WezTerm** — required only for live working/waiting status and
  open/focus/new-session integration.

**Install & run**
```bash
cd /home/harsh/Documents/CC-pet-token
chmod +x install.sh run.sh
./install.sh          # checks deps, optionally apt-installs, optionally adds login autostart
./run.sh              # or:  python3 pet.py
```
`install.sh` can also write `~/.config/autostart/claude-pet.desktop` to launch the
pet on login.

**Verify the data layer without the GUI**
```bash
python3 sessions.py                 # all recent sessions
python3 sessions.py /path/to.jsonl  # one session
```

---

## 8. Tuning Knobs (constants near the top of each file)

**`pet.py`**
| Constant | Default | Effect |
|----------|---------|--------|
| `PET_MAX` | 110 | Max pet sprite size (px). |
| `DRAG_THRESHOLD` | 6 | Px of movement before a press counts as a drag. |
| `REFRESH_SECS` | 2 | Panel live-refresh cadence. |
| `TICK_MS` | 40 | Animation tick interval. |
| `PANEL_W` / `PANEL_H` | 480 / 540 | Panel size. |
| `TRIM_EDGE` / `TRIM_DARK` / `TRIM_ITERS` | True / 70 / 2 | Edge de-matte settings. |
| `TELEPORT_MIN_S` / `TELEPORT_MAX_S` | 600 / 1800 | Wander interval range. |
| `GLIDE_MS` | 900 | Teleport glide duration. |
| `PET_INTERVAL_S` | 10800 (~3h) | How often it demands petting. |
| `PETS_REQUIRED` | 10 | Clicks to satisfy it. |
| `PET_GROW_SCALE` | 2.6 | How big it grows while demanding petting. |
| `ATTENTION_POLL_S` | 5 | Attention-check cadence. |
| `ATTENTION_WINDOW_S` | 600 | Only nag about sessions touched within this window. |
| `CSS` block | — | All colors, fonts, rounding, panel opacity. |

**`sessions.py`**
| Constant | Default | Effect |
|----------|---------|--------|
| `LIVE_WINDOW` | 120 | Seconds a session counts as "running" (is_live). |
| `TITLE_WIDTH` | 46 | Per-prompt title truncation width. |
| `PRICING` | — | USD-per-million-tokens table per model family. |
| `find_sessions(limit=…)` | 25 | Session list cap. |

---

## 9. Design Decisions & Notable Traps (summary)
- **Two-layer split** keeps parsing testable and GUI-agnostic. `sessions.py` is
  stdlib-only and never raises out of its public functions.
- **One-level glob** for discovery so subagent transcripts aren't counted as
  sessions.
- **Dedupe tokens by `message.id`** — streamed responses repeat usage across many
  lines sharing one id.
- **`promptSource == "typed"`** is the reliable way to isolate human prompts, with
  a meta-prefix heuristic fallback for old transcripts.
- **mtime-keyed parse cache** makes the 2-second panel refresh effectively free.
- **WezTerm spinner glyph** in the tab title is the signal for "Claude is
  working"; absence + completed last turn ⇒ "waiting for you" ⇒ attention alert.
- **Graceful degradation**: no WezTerm → no live flags but everything else works;
  no transcripts → friendly empty state; no art file → built-in coin mascot; no
  compositor → opaque box but still functional.
- **Subagent tokens are out of scope in v1** — the hook for v2 is marked in code.
