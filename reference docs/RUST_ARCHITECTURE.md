# Claude Code Pet — Rust Rewrite (cc-pet)

The pet was rewritten from Python/GTK3 to **Rust** to become a fast single-binary
foundation for a broader "make my Linux life easier" tool. This document describes
the Rust implementation. (The original Python design lives in `ARCHITECTURE.md`; the
`pet.py`/`sessions.py` files remain as a reference.)

## Goal & scope of the rewrite
Same core idea — an ambient desktop pet that surfaces your AI-coding session usage —
plus four new capabilities requested for this iteration:

1. **Multi-screen support** — the pet wanders across all monitors and the panel opens
   on the monitor the pet is on.
2. **Configurable terminal** — terminal control is behind a trait; WezTerm is the full
   backend, with a generic command-template backend for anything else.
3. **Usage quotas (daily/weekly)** — live for Claude, local for Codex.
4. **Codex usage** — Codex sessions are a first-class provider alongside Claude.

## Workspace layout
```
cc-pet/ (cargo workspace)
├── crates/petcore/   # data layer — no GUI (successor to sessions.py)
│   ├── providers/{mod,claude,codex}.rs   # unified Session shape per provider
│   ├── pricing.rs                        # PRICING table + cost (Claude)
│   ├── quota.rs                          # Claude OAuth usage + Codex rate_limits
│   ├── terminal/{mod,wezterm,generic}.rs # Terminal trait + backends
│   ├── config.rs / state.rs              # TOML config + JSON runtime state (XDG)
│   ├── fmt.rs                            # display helpers + today_totals
│   └── bin/dump.rs                       # CLI smoke test (like `python3 sessions.py`)
└── crates/petgui/    # eframe/egui GUI (successor to pet.py)
    ├── app.rs        # App state + update loop, teleport/petting/attention/quota
    ├── pet.rs        # sprite/placeholder draw, hearts, badge, click-vs-drag
    ├── panel.rs      # list / history / quota views (immediate child viewport)
    ├── art.rs        # GIF decode + scale + de-matte -> egui textures
    ├── monitors.rs   # xrandr multi-monitor geometry
    └── theme.rs      # colors mirroring the old CSS
```
**Why egui/eframe:** it sits on `winit`, which gives the precise window positioning,
per-pixel transparency, and always-on-top control the pet needs on X11 — GTK4 would
restrict client-side positioning. The pet and panel are two `winit` viewports.

## Data layer (`petcore`)
- **Claude** (`providers/claude.rs`) — a faithful port of `sessions.py`: one-level glob
  of `~/.claude/projects/*/*.jsonl`, real prompts via `promptSource=="typed"` (legacy
  fallback), token **dedupe by `message.id`**, cost via `PRICING`, mtime cache. Verified
  to produce identical tokens/elapsed/cost to the Python version.
- **Codex** (`providers/codex.rs`) — parses `~/.codex/sessions/**/rollout-*.jsonl`.
  Prompts = `event_msg`/`user_message`; tokens accumulate from `token_count` events
  (`info.total_token_usage`). Cost is left at 0 (Codex isn't in the Claude price table).
- `find_sessions()` merges both providers, newest first. `parse_session_any()` dispatches
  by path.

## Quotas (`quota.rs`)
- **Claude** — `GET https://api.anthropic.com/api/oauth/usage` with
  `Authorization: Bearer <accessToken>` (from `~/.claude/.credentials.json`,
  `claudeAiOauth`), `anthropic-beta: oauth-2025-04-20`, and `User-Agent: claude-code/…`
  (the UA matters — without it the endpoint 429s aggressively). Response windows
  `five_hour`/`seven_day`/`seven_day_{opus,sonnet}` give `utilization` (%) + `resets_at`.
  Cached to `~/.config/cc-pet/claude-usage.json` with a TTL (`claude_poll_secs`, default
  300s). If the token is expired, stale data is shown with a "run `claude` to refresh"
  note (we never rotate the refresh token to avoid corrupting Claude's credentials).
  The fetch runs on a **background thread** so the pet never freezes.
- **Codex** — read directly from the newest rollout file's `rate_limits`:
  `primary` (5-hour window) and `secondary` (weekly). No network.
- The GUI shows them as bars (green / amber ≥70% / red ≥90%) with reset countdowns
  under a **Usage** view in the panel.

## Terminal control (`terminal/`)
`trait Terminal { list_panes, supports_live_status, focus, spawn }`.
- `WezTermBackend` — full support: `wezterm cli list` for open/working/waiting detection
  (Claude's braille spinner in the tab title ⇒ "working"), `spawn`/`activate-pane`.
- `GenericTerminal` — launch-only via a configurable command template (`{cwd}`/`{cmd}`).
- Chosen via `config.toml` `[terminal] backend`.

## Config & state (XDG, `~/.config/cc-pet/`)
- `config.toml` — terminal backend/templates, quota poll interval + optional manual
  Claude limits, pet sizes/timers, provider enables. Written with defaults on first run.
- `state.json` — pet position, pins, last session, last-pet time (migrates from the old
  `~/.config/claude-pet/state.json`).
- `claude-usage.json` — cached Claude usage response.

## Build & run
```bash
cargo run -p petgui --bin cc-pet         # the pet (X11)
cargo run -p petcore --bin dump          # data-layer smoke test
cargo run -p petcore --bin dump -- --quota   # live Claude + local Codex quotas
cargo run -p petgui --bin cc-pet -- --art-check   # verify the sprite pipeline
```
Requires X11 (winit can't freely position windows on Wayland). Drop `assets/pet.gif`
for custom art; otherwise a coin mascot is drawn.

## Verification status
- petcore Claude parse → **token/elapsed/cost parity** confirmed vs `python3 sessions.py`.
- Codex parsing, live Claude quota, and local Codex quota confirmed via `dump --quota`.
- Sprite pipeline (3 GIF frames, scale, de-matte) confirmed via `--art-check`.
- App launches always-on-top/transparent at the correct geometry; config/state/usage
  caches are written. (Live pixel rendering can't be screenshotted headlessly because
  egui uses an OpenGL surface — verify visually on the desktop.)
