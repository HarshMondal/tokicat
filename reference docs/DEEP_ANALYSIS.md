# cc-pet — Deep Analysis (current state)

> Authoritative, code-level walkthrough of the application **as it exists today**,
> including the uncommitted work (opencode / GLM / MiniMax providers, brand logos,
> the usage-first panel, the quit chord). Written as the shared reference for the
> UI / workflow changes we're about to discuss.
>
> Companion docs in this folder:
> - `RUST_ARCHITECTURE.md` — the original rewrite design note (predates opencode + multi-provider usage).
> - `ARCHITECTURE.md` — the original Python/GTK3 app design (`pet.py` / `sessions.py`), kept as historical reference.
>
> ⚠️ The top-level `README.md` still describes the **old Python GTK3** behavior
> (e.g. "first click → session list"). The Rust app no longer behaves that way —
> see *Discrepancies* at the end.

---

## 1. What this thing is, in one paragraph

`cc-pet` is an **ambient desktop pet for Linux/X11**: a small (~110 px), always-on-top,
transparent window showing a coin mascot (or your custom GIF). It quietly watches your
AI-coding sessions (Claude Code, Codex, and opencode/GLM/MiniMax), and when you **click
it** a side **panel** opens showing your **usage limits**, your **session list**, and a
per-prompt **token/cost scoreboard**. It has personality — it wanders the screen, and
periodically grows and asks to be petted. It also nudges you (bounce + red `!` badge)
when a session you were running finishes and is waiting on you.

It is a **Cargo workspace** with two crates:

| Crate | Role | Analogy |
|-------|------|---------|
| `petcore` | Pure data/logic layer — no GUI. Parses transcripts, prices tokens, fetches quotas, controls the terminal. | the old `sessions.py` |
| `petgui`  | The `eframe`/`egui` GUI — windows, pet animation, panel rendering, input. | the old `pet.py` |

```
cc-pet/
├── Cargo.toml                 # workspace; shared deps; dev opt-level=2 for smooth dev runs
├── crates/
│   ├── petcore/               # DATA LAYER (lib + `dump` bin)
│   │   └── src/
│   │       ├── lib.rs                  # find_sessions(), parse_session_any()
│   │       ├── providers/
│   │       │   ├── mod.rs              # Provider enum, Session, Prompt, TokenBreakdown
│   │       │   ├── claude.rs           # ~/.claude/projects/*/*.jsonl parser
│   │       │   ├── codex.rs            # ~/.codex/**/rollout-*.jsonl parser
│   │       │   └── opencode.rs         # opencode SQLite token totals
│   │       ├── pricing.rs              # USD-per-Mtok rate table + cost()
│   │       ├── quota.rs                # unified usage snapshot (Claude/Codex/GLM/opencode)
│   │       ├── config.rs               # config.toml (TOML)
│   │       ├── state.rs                # state.json (runtime state)
│   │       ├── fmt.rs                  # fmt_tokens/elapsed/cost, today_totals
│   │       ├── terminal/{mod,wezterm,generic}.rs  # Terminal trait + backends
│   │       └── bin/dump.rs             # headless smoke test
│   └── petgui/                # GUI LAYER (cc-pet bin)
│       └── src/
│           ├── main.rs                 # window/viewport bootstrap, --art-check
│           ├── app.rs                  # App state + the eframe update loop
│           ├── pet.rs                  # pet drawing + click-vs-drag input
│           ├── panel.rs                # the 3 panel views + quit modal
│           ├── art.rs                  # GIF/PNG load → scale → de-matte
│           ├── logos.rs                # per-provider brand PNGs
│           ├── monitors.rs             # xrandr multi-monitor geometry
│           └── theme.rs                # colors, brand accents, heat colors
└── assets/
    ├── pet.gif                # the mascot (optional; falls back to drawn coin)
    └── logos/<provider>.png   # claude/codex/glm/minimax/opencode brand marks
```

**Dependency stack:** `eframe`/`egui` 0.34 (glow/x11) for the GUI; `image` (gif/png/jpeg/webp)
for sprites; `ureq` for the quota HTTP calls; `rusqlite` (bundled) to read opencode's DB;
`serde`/`serde_json`/`toml` for config/parsing; `chrono` for timestamps; `glob` +
`directories` for file discovery; xrandr/wezterm are shelled out to (not linked).

---

## 2. The data model (`petcore::providers::mod.rs`)

Everything the GUI renders is a **`Session`** — one unified shape regardless of provider.

```rust
enum Provider { Claude, Codex, Zai, Minimax, Opencode }
//   label():        claude / codex / glm / minimax / opencode   (asset & config key)
//   display_name(): Claude / Codex / GLM Coding Plan / MiniMax / opencode
```

- `Zai` = z.ai / GLM Coding Plan, `Minimax`, and `Opencode` exist mainly for the **usage
  screen** (opencode is an umbrella whose card *nests* GLM + MiniMax). Sessions in the
  list are only ever `Claude` or `Codex`.

```rust
struct Session {
    provider, path, session_id, cwd, project, branch,
    title, model, model_family, mtime,
    // live/terminal flags (filled later by terminal annotation):
    is_live, open, pane_id, working, waiting,
    total_prompts, total_tokens,        // total_tokens = Σ per-prompt output tokens
    tokens: TokenBreakdown,             // input/output/cache_read/cache_write_5m/_1h
    cost,                               // USD estimate (0 for Codex)
    last_prompt_ts, last_completed, wall_seconds,
    prompts: Vec<Prompt>,
}

struct Prompt { index, title, full_text, out_tokens, elapsed, running, completed }
```

A **`Prompt`** is one *turn*: the text you typed plus all the assistant work until your
next prompt. `title` = first non-empty line; `full_text` = the whole thing (for expand).

---

## 3. Parsing — how sessions are read

### 3.1 Claude (`providers/claude.rs`) — the faithful port of `sessions.py`

- **Discovery:** `glob("~/.claude/projects/*/*.jsonl")` — exactly **one level deep**, so
  nested subagent files (`projects/*/<id>/subagents/agent-*.jsonl`) are never mistaken
  for top-level sessions.
- **Per-file parse** (`parse_session`) is **mtime-cached** (a `LazyLock<Mutex<HashMap>>`):
  re-parsing an unchanged file is free.
- **Real prompts** = entries with `type=="user"`, not `isSidechain`, and
  `promptSource=="typed"`. For older transcripts without `promptSource`, a heuristic
  fallback excludes tool-results and meta lines (`<bash-input>`, `<local-command>`,
  `caveat:`, `[request interrupted`, …).
- **Token accounting:** assistant responses stream as several JSONL lines that **share one
  `message.id`** with usage repeated, so tokens are **deduped by message id**. Per-prompt
  number = summed `usage.output_tokens`. Session-wide `TokenBreakdown` also tracks input /
  cache-read / cache-write (5m vs 1h), used only for cost.
- **Title** priority: `aiTitle` → `slug` → first prompt's first line (≤60 chars) →
  session-id prefix → filename.
- **Completion / running:** a turn is `completed` if an assistant `stop_reason` is
  `end_turn`/`stop_sequence`/`max_tokens`. The newest turn with **no** assistant response
  yet is `running`.
- **Elapsed** = last assistant ts − prompt ts. **`wall_seconds`** = max end − min start.
- **Cost** via `pricing::cost()` using the model family detected from the assistant `model`.

### 3.2 Codex (`providers/codex.rs`)

- **Discovery:** `glob("~/.codex/sessions/**/rollout-*.jsonl")` + `archived_sessions/…`.
- Lines are `{timestamp, type, payload}`. `session_meta` gives id/cwd/model.
- **Prompts** = `event_msg` payloads of type `user_message`.
- **Tokens** = `event_msg` `token_count` payloads; `info.total_token_usage.output_tokens`
  is **cumulative**, so per-turn output = `cum_at_next_turn − cum_at_this_turn`.
- **Completion** = a `task_complete` event.
- **Cost is always 0** (Codex isn't in the Claude price table).
- Empty rollouts (no user prompts) are skipped to keep the list useful.

### 3.3 opencode (`providers/opencode.rs`) — *totals only, no sessions*

- Reads `~/.local/share/opencode/opencode.db` (SQLite) **read-only + immutable**
  (`mode=ro&immutable=1`) so a running opencode never blocks us and we never lock it.
- A single JSON1 `json_extract` query sums `tokens.input` / `tokens.output` / `cost` from
  the `message` table, **grouped by `providerID`** (e.g. `zai-coding-plan`, `minimax`).
- Returns `HashMap<String, (in, out, cost)>`. This feeds **only the usage screen**, not
  the session list.

### 3.4 Merge (`lib.rs`)

- `find_sessions(limit)` = Claude sessions ++ Codex sessions, sorted newest-first by
  `mtime`, truncated to `limit` per the merged list.
- `parse_session_any(path)` dispatches by path (`/.codex/` → Codex, else Claude).
- `is_live` is set fresh each call: `now − mtime ≤ LIVE_WINDOW` (**120 s**).

---

## 4. Pricing (`pricing.rs`)

USD **per million tokens**, by model family (matched by substring in the model id):

| family | input | output | cache-write 5m | cache-write 1h | cache-read |
|--------|------:|-------:|---------------:|---------------:|-----------:|
| opus   | 5.00  | 25.00  | 6.25 | 10.00 | 0.50 |
| sonnet | 3.00  | 15.00  | 3.75 |  6.00 | 0.30 |
| haiku  | 1.00  |  5.00  | 1.25 |  2.00 | 0.10 |
| fable  | 10.00 | 50.00  | 12.50| 20.00 | 1.00 |

Unknown/missing model → **opus** (a deliberately conservative default). `cost()` is a
straight dot-product of the `TokenBreakdown` against these rates ÷ 1e6.

---

## 5. Usage / Quotas (`quota.rs`) — the most intricate subsystem

The panel's default screen. Produces a `QuotaSnapshot { sections: Vec<ProviderSection> }`,
one **card** per provider. A card has a `badge` (plan tier), a `note` (status caption),
`windows` (percentage bars), an optional `summary` (token totals row), and `children`
(nested sub-providers).

`snapshot(cfg)` assembles sections for each **enabled** provider:

### 5.1 Claude — live, cached
- `GET https://api.anthropic.com/api/oauth/usage`
- Auth = `Bearer <accessToken>` read from `~/.claude/.credentials.json` (`claudeAiOauth`).
- Required headers: `anthropic-beta: oauth-2025-04-20` **and** `User-Agent: claude-code/1.0`
  — without the UA the endpoint 429s aggressively.
- Windows surfaced: `five_hour` → "5h", `seven_day` → "weekly",
  `seven_day_opus` → "weekly · opus", `seven_day_sonnet` → "weekly · sonnet". Each carries
  `utilization` (%) and `resets_at`.
- **Cached** to `~/.config/cc-pet/claude-usage.json` honoring TTL `claude_poll_secs`
  (default **300 s**).
- **Token expired** → we deliberately **do not** refresh/rotate (that would corrupt
  Claude's own credentials); we show stale data + note *"run any `claude` command to refresh"*.

### 5.2 Codex — local snapshot, no network
- Reads the **newest** rollout file, finds the most recent `payload.rate_limits` block.
- `primary` → 5h, `secondary` → weekly (labels also derived from `window_minutes`:
  300→5h, 10080→weekly). `plan_type` becomes the badge.
- **Rollover handling:** if a window's `resets_at` has already passed since that snapshot
  was recorded, the stored percent is stale → forced to **0%** and flagged `reset`.
- Note = "as of <local time of the snapshot>".

### 5.3 opencode — umbrella card nesting GLM + MiniMax
- `opencode_section()` builds children:
  - **GLM (z.ai)** `zai_child`: **live** quota, cached to `zai-usage.json` (TTL
    `zai_poll_secs`, default 300 s). Endpoint
    `GET <zai_base_url>/api/monitor/usage/quota/limit`, auth = the **raw API key, NO
    "Bearer" prefix**, key pulled from opencode's `account.json`
    (`serviceID == "zai-coding-plan"`). `data.limits[]` map to windows: TOKENS_LIMIT
    unit3/num5 → "5h tokens", unit6/num1 → "weekly tokens", plus TIME_LIMIT → "MCP tools".
    `data.level` = badge. Also gets opencode token totals as its `summary`.
  - **MiniMax**: opencode token **totals only** (no public quota API) → just a `summary`.
- If neither child has data, the whole opencode card is omitted.

### 5.4 Generic cache mechanics
- Live providers (Claude, GLM) share a `UsageCache` JSON shape `{fetched_at, windows,
  note, badge}` under `~/.config/cc-pet/`. The TTL gate prevents over-fetching; when fresh,
  the cache is rendered directly; when stale, a fetch is attempted and falls back to the
  cached windows + a note on failure.

---

## 6. Terminal control (`terminal/`)

A `trait Terminal { list_panes, supports_live_status, focus, spawn }` so the terminal is
swappable (config `[terminal] backend`).

- **`WezTermBackend`** (default, full support):
  - `wezterm cli list --format json` → panes (id, title, cwd, leading non-alnum glyph).
  - **Live status detection:** a session is matched to a pane when the pane's normalized
    cwd equals the session cwd **and** the pane title contains the session title. Then:
    `working` if the pane title's leading glyph is one of Claude's **braille spinner**
    chars; `waiting` if not working but the session's last turn completed.
  - `focus` = `wezterm cli activate-pane`; `spawn` = `wezterm cli spawn` (falls back to a
    new `wezterm start` window).
- **`GenericTerminal`** (launch-only): renders a config command template with `{cwd}`/`{cmd}`
  placeholders. No pane querying ⇒ **no live working/waiting**, `focus` always fails (caller
  falls back to spawning).

Helpers in `terminal/mod.rs`:
- `open_session(term, s)` — if the session is already `open`, focus its pane; else spawn a
  tab in its cwd running `claude --resume <id>` (or `codex resume <id>`) `; exec bash`
  (drops to a shell so the tab survives).
- `new_session(term, cwd)` — spawn a fresh `claude ; exec bash`.
- `annotate_open(sessions, panes)` — the function that sets `open/pane_id/working/waiting`
  on each session by matching against the live pane list.

---

## 7. Config & state (XDG, `~/.config/cc-pet/`)

### `config.toml` (`config.rs`) — written with defaults on first run
- `[terminal]` `backend` ("wezterm"/"generic"), `spawn_template`, `new_template`.
- `[quota]` `claude_poll_secs`=300, optional manual `claude_daily/weekly_token_limit`,
  `zai_poll_secs`=300, `zai_base_url`="https://api.z.ai" (CN: "https://open.bigmodel.cn").
- `[pet]` `max_size`=110, `refresh_secs`=2, `teleport_min_secs`=600,
  `teleport_max_secs`=1800, `pet_interval_secs`=10800 (3 h), `pets_required`=10.
- `[providers]` `claude`/`codex`/`zai`/`opencode` — all default **true**.

### `state.json` (`state.rs`) — runtime state, JSON
- `pet_x`, `pet_y` (last position), `last_session` (path), `pinned` (Vec of paths),
  `last_pet_time`. One-time migration from the old `~/.config/claude-pet/state.json`.

### Cache files
- `claude-usage.json`, `zai-usage.json` — cached quota responses.

---

## 8. The GUI — windows, loop, and rendering (`petgui`)

### 8.1 Bootstrap (`main.rs`)
- `--art-check` → headless sprite-pipeline self-test, then exit.
- Refuses to run with no `DISPLAY`/`WAYLAND_DISPLAY`; warns on Wayland (positioning is
  unreliable there — the pet is an X11 design).
- Creates the **pet viewport**: 110×110, **no decorations, transparent, always-on-top,
  not resizable, no taskbar entry**, initial position `(1500, 60)`.

### 8.2 Two viewports
1. **Pet window** — the 110 px transparent always-on-top sprite (root viewport).
2. **Panel window** — a separate **immediate child viewport** (480×540, also undecorated/
   transparent/always-on-top) created on demand via `show_viewport_immediate`. It shares
   `&mut self`, so both windows render from one `App`.

`PANEL_W = 480`, `PANEL_H = 540`.

### 8.3 The update loop (`app.rs :: eframe::App::ui`)
Runs every frame (`request_repaint_after(33ms)` ≈ 30 fps). Per frame, in order:
1. First-frame bootstrap: schedule first teleport, place the pet at top-right if no saved
   position, push window geometry.
2. Sync `self.pos` from the real OS window rect (unless mid-animation).
3. Advance the GIF frame by elapsed delay.
4. Step animations (`step_anim`), hearts, decay wiggle/bounce.
5. Celebrate → after a beat, shrink back home (finishing the petting cycle).
6. **Attention poll** every `ATTENTION_POLL` = **5 s** (`poll_attention`).
7. **Quota refresh** (async, TTL-gated): every **20 s** while the panel is open, **120 s**
   otherwise. `drain_quota()` picks up the background thread's result.
8. **Panel data refresh** every `refresh_secs` (2 s) while the panel is open.
9. **Teleport** when `t ≥ next_teleport`; reschedule 10–30 min out.
10. **Petting check** every `PET_CHECK` = **60 s**: if idle and ≥ `pet_interval_secs`
    (3 h) since last pet, enter the petting routine.
11. Handle the **quit chord** (Ctrl+Shift+U).
12. Draw the pet (`ui_pet`); if open, draw the panel viewport.

### 8.4 Asynchrony
- The only background thread is the **quota fetch** (`refresh_quota` spawns a thread that
  writes into an `Arc<Mutex<Option<QuotaSnapshot>>>`, guarded by an `AtomicBool` in-flight
  flag), so the blocking Claude/GLM HTTP calls never freeze the pet.
- All session parsing runs on the UI thread but is cheap (mtime-cached, capped lists).

---

## 9. The pet itself (`pet.rs`)

- **Drawing:** if a sprite texture exists, draw the current GIF frame scaled by `self.scale`;
  otherwise draw the **coin placeholder** (filled circle + edge stroke + two eyes + a smile
  arc).
- **Motion:** a gentle sinusoidal **bob**, a **wiggle** on petting, and a **bounce** when
  attention is pending — all overlaid on the sprite position.
- **Hearts:** small heart particles spawned on petting/celebrate, floating up and fading.
- **Banner:** while petting, a pill above the pet reads "pet me!" → "more! N" → "yay! ♥".
- **Badge** (top-right corner of the pet):
  - red **`!`** (or a count) when there are pending **attention** sessions, OR
  - a small green dot when any session is **live** and the pet isn't grown.
- **Input (`handle_input`)** — state-dependent:
  - *Normal:* drag → move the window (`StartDrag`, save pos on release); **click → toggle
    the panel**.
  - *Petting:* click → counts as one pet (`on_pet`).
  - *Celebrate:* clicks ignored.

### Personality state machine (`PetState`)
```
Normal ──(3h idle, 60s checker)──▶ Petting ──(10 clicks)──▶ Celebrate
   ▲                                                            │
   └──────────────(grow → shrink home, record last_pet)◀───────┘
```
- **Teleport:** every 10–30 min the pet glides to a random point on a random monitor
  (skipped while petting/animating/panel-open).
- **Petting:** grows to 2.6×, centered on its monitor (clamped to stay fully on-screen),
  asks for `pets_required` (10) clicks; on completion it celebrates (extra hearts) then
  shrinks back home and records `last_pet_time`.

---

## 10. The panel (`panel.rs`) — three views

The panel viewport renders one of three `PanelView`s into a rounded translucent frame.
**Esc** or **✕** closes it; the **Ctrl+Shift+U** quit modal can overlay any view.

### 10.1 `Quota` — "Usage limits" *(this is the DEFAULT view when you click the pet)*
- Header: `‹ Sessions` (go to list), title, `⟳` (force refetch), `✕`.
- "Fetching usage…" until the first snapshot arrives.
- One **card per provider** (`quota_card`), each with:
  - a **brand accent stripe** down the left edge + a **logo/monogram chip** + name + plan
    **badge** + a right-aligned status **note** (amber if stale).
  - **percentage bars** (`quota_bar`): label, %, and "resets in …" / "reset since last run".
    Bar color is the **brand accent** until usage is high, then **amber ≥70%**, **red ≥90%**.
  - an optional **token-totals row** (`quota_summary`: input / output / cost).
  - **nested children** (GLM, MiniMax) under the opencode card.

### 10.2 `List` — "Claude sessions"
- Header buttons: `Today` filter, `+ New`, `◔ Usage`, `✕`. Plus a **search** box
  (title/folder/branch/cwd; searching refetches the full pool).
- Sessions grouped into **PINNED → ACTIVE → RECENT** (or "MATCHES" while searching).
  ACTIVE = currently open in the terminal, sorted working → waiting → other.
- Each **row** (`session_row`):
  - a status **dot** (green working / amber waiting / grey off),
  - the **title** (click → History),
  - a **★/☆ pin** toggle and a **`⮒ open` / `⮒ focus`** button (opens/focuses in the terminal),
  - a second line: project `⎇ branch`, a `codex` chip for Codex sessions, prompt/token
    counts, and a green cost (hidden for Codex).
- **Footer:** either today's totals (when `Today` is on), a transient action hint
  (e.g. "opened …"), or the legend `N sessions · ● working ● waiting · ⮒ open · ★ pin`.

### 10.3 `History` — per-session prompt scoreboard
- Header: `‹ Sessions`, session title, `✕`.
- A scrollable list of **prompt rows**, each: a `▸/▾` caret (if expandable), the index, the
  prompt title (click to expand to full text), and on the right either a **`running…`**
  chip or **`<tokens> tok`** (colored by **heat**: red ≥66% of the heaviest, amber ≥33%,
  blue otherwise) + **elapsed**.
- **Footer summary card:** total prompts · output tokens · wall-clock.

### 10.4 Quit modal (`quit_modal`)
- The **only** way to quit is **Ctrl+Shift+U** (checked in *both* viewports since either
  may hold focus). It forces the panel open to host a centered "Quit cc-pet?" confirmation
  (reassuring the user tracking resumes from session files). Enter/✓ quits, Esc/Cancel
  dismisses. There is intentionally no window close button / taskbar entry.

---

## 11. Supporting GUI modules

- **`art.rs`** — locates `assets/pet.{gif,png,jpg,jpeg,webp}` across dev/installed layouts
  (env `CC_PET_ASSETS`, exe-relative, compile-time repo path). GIFs decode all frames with
  per-frame delays; everything is **scaled** to ≤ `max` on the long edge and **de-matted**
  (`dematte`: erodes the dark anti-alias halo 2 layers deep so a dark-edged sprite doesn't
  show a box on the transparent window).
- **`logos.rs`** — loads `assets/logos/<provider-label>.png` into textures; missing logos
  are fine (panel falls back to the drawn monogram chip).
- **`monitors.rs`** — enumerates monitors via `xrandr --query`, **scaling physical pixels
  down by `pixels_per_point`** so geometry matches egui's logical-point coordinate space
  (critical on HiDPI). Falls back to a single egui-reported monitor. `monitor_at(x,y)`
  picks the screen for clamping/teleport/panel placement.
- **`theme.rs`** — the dark palette, per-provider **brand accents** (Claude coral, Codex
  teal, GLM blue, MiniMax rose, opencode violet), `pct_color` (accent → amber → red),
  `heat_color` (token weight), and the coin colors.

---

## 12. End-to-end workflows (the "why it does what it does")

**A. Glance at usage:** click the pet → panel opens **straight to Usage limits** → bars
for Claude/Codex/opencode(GLM+MiniMax). (Async fetch keeps the pet responsive; cached with
TTLs to respect rate limits.)

**B. Resume a session:** Usage → `‹ Sessions` (or `◔/+ New`) → List → click `⮒ open` on a
row. If it's already open in WezTerm it **focuses that tab**; otherwise it spawns a new tab
running `claude --resume <id>`.

**C. Inspect spend:** List → click a session title → History → per-prompt tokens (heat-
colored) + elapsed, with a totals card. Heaviest prompts are visually obvious.

**D. Get nudged:** while you work elsewhere, the 5 s attention poll notices a session you
were running transitioned working → waiting → the pet **bounces + shows a red `!`**. Click
the badge/row to jump to it.

**E. Ambient life:** every 10–30 min it teleports; every 3 h it grows and asks for 10 pets,
celebrates, and settles back.

**F. Quit:** Ctrl+Shift+U → confirm.

---

## 13. Build & run

```bash
cargo run -p petgui  --bin cc-pet                 # the pet (X11)
cargo run -p petcore --bin dump                   # data-layer smoke test (recent sessions)
cargo run -p petcore --bin dump -- <path.jsonl>   # parse one session
cargo run -p petcore --bin dump -- --quota        # live quota snapshot in the terminal
cargo run -p petgui  --bin cc-pet -- --art-check  # verify the sprite pipeline headlessly
```
Requires **X11** (winit can't freely position windows on Wayland). Drop `assets/pet.gif`
for custom art; otherwise the coin mascot is drawn.

---

## 14. Discrepancies & things to know before changing UI

These are gaps between the docs/older design and the **actual current code** — worth
deciding on as we discuss changes:

1. **Panel opens to *Usage limits*, not the session list.** `toggle_panel()` sets
   `PanelView::Quota`. The README's "first click → list" is stale. (If the intent is
   "usage first", the docs should say so; if not, this is a one-line change.)
2. **There is no last-session shortcut on open.** The old design re-opened the last viewed
   session on the next click; the current click always lands on the Usage screen.
   `state.last_session` is still saved but only used to remember, not to auto-open.
3. **No in-app quit button** by design — only the Ctrl+Shift+U chord. New users won't
   discover it; there's no menu/affordance hinting at it.
4. **opencode = totals only.** opencode sessions don't appear in the session **list** (only
   Claude/Codex do); opencode/GLM/MiniMax show up **only** on the Usage screen.
5. **Codex cost is always $0** (not priced), and Codex rows hide the cost figure.
6. **Live status needs WezTerm.** Under the generic backend there's no working/waiting dot,
   no attention nudges, and `⮒ open` can't focus — it always spawns.
7. **README still documents the Python app** (GTK3, `pet.py`/`sessions.py`, conky history).
   The Rust app is the real one now; `pet.py`/`sessions.py` remain only as reference.
8. **Manual quota fallback is configured but unused.** `claude_daily/weekly_token_limit`
   exist in config but nothing reads them yet (the live API path supersedes them).
9. **Hardcoded constants** worth knowing live in `app.rs` (animation/poll timings: 5 s
   attention, 20/120 s quota, 60 s pet-check, 0.9 s teleport glide, 2.6× grow) and the
   initial window position `(1500, 60)` in `main.rs`.
```
