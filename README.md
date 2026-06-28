<div align="center">

<img src="assets/pet.gif" alt="tokicat" width="180" />

# tokicat 🐈

**This cat lives on your desktop and quietly watches your AI coding sessions.**

Usage limits, tokens, and cost — at a glance, on top of everything, with a little personality.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
![Platform: Linux/X11](https://img.shields.io/badge/platform-Linux%2FX11-1f6feb)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)

</div>

---

## What is tokicat?

**tokicat** is a small, always-on-top desktop companion for **Linux / X11**. A cat mascot
sits on your screen and watches your AI-coding sessions across **Claude Code, Codex, and
opencode (GLM / MiniMax)**. Click it and a panel opens with your **usage limits**, your
**session list**, and a per-prompt **token & cost scoreboard**.

It also has personality — it wanders the screen, naps, and asks to be petted — and it
**nudges you** (a bounce and a red `!` badge) the moment a session you were running finishes
and is waiting on you. You can resume or focus any session in your terminal straight from the
panel.

It reads your tools' own session files — there is nothing to sign in to and nothing to send
anywhere.

## Features

- 🐈 **Ambient pet** — wanders, grows, naps, and asks for attention. Pure desktop personality.
- 📊 **Usage at a glance** — live 5h / weekly quota bars for **Claude** and **GLM**, local
  snapshot for **Codex**.
- 🪙 **Token & cost scoreboard** — per-prompt output tokens (heat-colored by weight) with USD
  cost estimates.
- 🔔 **Smart nudges** — bounces and shows a red `!` badge when a session finishes and is
  waiting on you. Click it to jump straight there.
- 🖥️ **Terminal control** — open, focus, or `claude --resume` any session from the panel
  (full live status with WezTerm).
- 🔌 **Multi-provider** — Claude Code, Codex, and opencode (GLM + MiniMax) in one place.
- 📌 **Pin & search** — pin favourite sessions and filter by title, folder, or branch.

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| **Linux with X11** | Window positioning relies on X11. Wayland is unreliable (the pet warns you). |
| **Rust toolchain** | `cargo` + `rustc` (stable). Install via [rustup](https://rustup.rs). |
| **A compositor** | Needed for true transparency. GNOME / KDE / XFCE composite by default; on a bare WM run e.g. `picom &`. |
| **WezTerm** *(optional)* | Enables live working/waiting status, attention nudges, and open/focus. Without it, tokicat can still launch sessions but can't track live status. |

tokicat reads these locations if they exist (all optional — it shows whatever is present):

- Claude Code transcripts — `~/.claude/projects/*/*.jsonl`
- Claude usage + credentials — `~/.claude/.credentials.json` (OAuth token, read-only)
- Codex rollouts — `~/.codex/sessions/**/rollout-*.jsonl`
- opencode database — `~/.local/share/opencode/opencode.db` (opened read-only)

## Install & run

```bash
# 1. Clone
git clone https://github.com/HarshMondal/tokicat.git
cd tokicat

# 2. Run it (debug deps are built optimized, so dev runs stay smooth)
cargo run -p petgui --bin tokicat
```

To build an optimized binary and run it directly:

```bash
cargo build --release -p petgui --bin tokicat
./target/release/tokicat
```

The cat appears near the top-right of your screen. **Drag** it to move (its position is
remembered). **Click** it to open the panel. Press **Esc** or **✕** to close the panel.
To quit tokicat, press **Ctrl + Shift + U** and confirm.

### Other commands

```bash
cargo run -p petgui  --bin tokicat -- --art-check   # verify the sprite pipeline headlessly
cargo run -p petcore --bin dump                     # data-layer smoke test (recent sessions)
cargo run -p petcore --bin dump -- <path.jsonl>     # parse a single session file
cargo run -p petcore --bin dump -- --quota          # print a live quota snapshot
```

## Custom art

Drop an image into `assets/` named `pet.gif` (animated), `pet.png`, `pet.jpg`, or `pet.webp`
and it's picked up automatically, scaled to ~110 px, and de-matted for clean edges on the
transparent window. With no file, a built-in coin mascot is drawn instead.

## Configuration

On first run, tokicat writes a config file with sensible defaults under
`~/.config/cc-pet/config.toml` (XDG). You can tune the terminal backend, quota poll
intervals, pet size and timings, and which providers are enabled. Runtime state (pet position,
pinned sessions) and cached quota responses live in the same directory.

## Project layout

```
tokicat/
├── crates/
│   ├── petcore/   # data layer (no GUI): session parsing, pricing, quotas, terminal control
│   └── petgui/    # eframe/egui GUI: pet animation, panel, rendering  → binary `tokicat`
├── assets/        # pet.gif + per-provider brand logos
└── reference docs/ # in-depth design & architecture notes
```

For a full, code-level walkthrough, see [`reference docs/DEEP_ANALYSIS.md`](reference%20docs/DEEP_ANALYSIS.md).

## Troubleshooting

**No cat on screen** — run it from a terminal to see errors; confirm you're on X11
(`echo $XDG_SESSION_TYPE` should say `x11`). It starts near the top-right; it's always-on-top
but may sit behind a window briefly.

**Looks like a solid box / no transparency** — you need a compositor (see Prerequisites).

**Panel shows no sessions** — you have no transcripts yet. Start a session in your tool and
send a prompt, then click the cat again.

**No live working/waiting status** — live status, attention nudges, and focus require WezTerm.
Under other terminals tokicat can still launch sessions, but can't read their live state.

## License

[MIT](LICENSE) © Harsh Mondal
