# Claude Code Pet 🪙

An ambient desktop **pet** for Linux that tracks your Claude Code sessions. A small
always-on-top creature sits on your desktop; **click it** to pop open a panel:

- **First click** → a list of your recent / running Claude Code sessions
  (live ones marked with a green ●).
- **Click a session** → its prompt-by-prompt scoreboard: each prompt you typed,
  its output-token cost, and elapsed time, with the heaviest prompts colored.
- **Next time you click the pet** → it jumps straight back to the **last session
  you opened**, with a `‹ Sessions` button to go back to the list.

Drag the pet to move it (position is remembered). `Esc` or the `✕` closes the panel.

Everything lives in this repo. The pet reads Claude Code's own session transcripts
(`~/.claude/projects/<project>/*.jsonl`) — there's nothing to configure.

## Features
- **Live session status** (via WezTerm): the list groups **Active** sessions on top
  — green ● = Claude working, amber ● = open & waiting for you — with the rest under
  **Recent**. (Only sessions opened *in WezTerm* are detectable as active.)
- **Attention alerts**: when a session you're using finishes and is waiting on you, the
  pet **bounces and shows a red `!` badge**. Click it to jump to that session.
- **Open / focus in WezTerm**: each row has a **`⮒ open`** button — if the session is
  already open it **focuses that existing tab**; otherwise it opens a new tab and runs
  `claude --resume <id>`.
- **`+ New` session**: start a fresh `claude` in a recent project folder (or pick one).
- **Search**: filter all sessions by title, folder, or branch.
- **Cost estimate**: approximate USD per session (and a daily total) from token usage —
  pricing is editable in `sessions.py` (`PRICING`).
- **Today filter**: toggle to show just today's sessions with daily prompt/token/cost totals.
- **Pin** sessions (★) to keep them at the top.
- **Click-to-expand** prompts to read the full text; tokens color-coded by weight.
- **Personality**: wanders the screen every 10–30 min; every ~3 h it grows and asks to
  be petted (10 clicks). See the constants near the top of `pet.py`.

## Why not conky?
This started as a conky text overlay, but conky only *paints* text — it can't react
to clicks or show navigable panels. The pet is a small **GTK3 app** instead, which
gives transparency, click handling, and popups. The transcript-parsing logic
(`sessions.py`) is shared, stdlib-only code.

## Files
| File | Purpose |
|------|---------|
| `pet.py` | The GTK3 pet + panel application |
| `sessions.py` | Transcript parser / data layer (Python stdlib only) |
| `assets/` | Drop `pet.gif` / `pet.png` here for custom art |
| `install.sh` | Dependency check + autostart setup |
| `run.sh` | Convenience launcher (kills any running instance first) |
| `state.json` | *(created at runtime in `~/.config/claude-pet/`)* pet position + last session |

## Requirements
- Linux with **X11** (works under GNOME/Mutter, KDE, XFCE, etc.).
- **GTK3 + PyGObject** — on Ubuntu GNOME these are already installed
  (`python3-gi`, `gir1.2-gtk-3.0`, `python3-gi-cairo`). No pip packages.
- Python 3.

If the bindings are missing, `install.sh` offers to apt-install them, or:
```bash
sudo apt install python3-gi gir1.2-gtk-3.0 python3-gi-cairo
```

## Install & run
```bash
cd /home/harsh/Documents/CC-pet-token
chmod +x install.sh run.sh
./install.sh          # checks deps, optionally adds login autostart
./run.sh              # or:  python3 pet.py
```
The pet appears top-right (drag it anywhere). Click it to open the panel.

## Custom art
Drop an image into `assets/` named `pet.gif` (animated), `pet.png`, `pet.jpg`, or
`pet.webp` — it's picked up automatically and scaled to ~110 px. A transparent
background looks best (the pet window is transparent). With no file, a built-in
coin mascot is drawn.

## Autostart on login
`install.sh` can write `~/.config/autostart/claude-pet.desktop` for you. To do it by
hand:
```ini
[Desktop Entry]
Type=Application
Name=Claude Code Pet
Exec=bash -c "sleep 5; exec python3 /home/harsh/Documents/CC-pet-token/pet.py"
Terminal=false
X-GNOME-Autostart-enabled=true
```

## Tweaking
All knobs are constants near the top of the two files:
- `pet.py`: `PET_MAX` (pet size), `REFRESH_SECS` (live refresh, default 2s),
  `PANEL_W`/`PANEL_H` (panel size), `DRAG_THRESHOLD`, and the `CSS` block (colors,
  fonts, rounding, panel opacity in `.pet-panel`).
- `sessions.py`: `LIVE_WINDOW` (seconds a session is considered "running",
  default 120), `TITLE_WIDTH`, and the session list cap in `find_sessions(limit=…)`.

## How the parsing works (and its traps)
- **Active/recent sessions** = most-recently-modified `projects/*/*.jsonl`, globbed
  one level deep so nested subagent files
  (`projects/*/<id>/subagents/agent-*.jsonl`) are never mistaken for a session.
- **Real prompts** = user entries with `promptSource == "typed"` — this filters out
  tool results, `<bash-input>`, command output, and system-injected messages.
  A heuristic fallback covers older transcripts without that field.
- Each prompt is grouped with all assistant API calls until your next prompt.
  Responses stream as several lines sharing one `message.id` with usage repeated,
  so tokens are **deduped by `message.id`**. Per-prompt number = summed
  `usage.output_tokens`.
- Elapsed = last assistant timestamp − your prompt's timestamp.
- **Subagent / Task tokens** live in separate files and are ignored in v1 (a marked
  comment in `sessions.py` shows where to add them).

You can sanity-check the data layer without the GUI:
```bash
python3 sessions.py                 # all recent sessions
python3 sessions.py /path/to.jsonl  # one session
```

## Troubleshooting
**No pet on screen**
- Run it in a terminal to see errors: `python3 pet.py`.
- Make sure you're on X11: `echo $XDG_SESSION_TYPE` should say `x11`.
- The pet may be behind another window briefly — it's set always-on-top; click your
  desktop or drag a window. It starts near the top-right corner.

**Pet looks like a solid box / no transparency**
- True transparency needs a compositor. GNOME/KDE/XFCE composite by default. On a
  bare window manager, run a compositor like `picom &`.

**Panel shows "No Claude Code sessions found"**
- You have no transcripts yet. Start a Claude Code session and send a prompt, then
  click the pet again. Confirm the path:
  ```bash
  find ~/.claude/projects -maxdepth 2 -name '*.jsonl' | head
  ```

**It's tracking the wrong session**
- The session list is ordered by most-recent activity; "live" = modified within
  `LIVE_WINDOW` seconds (default 120). The pet reopens whichever session you last
  clicked. Use `‹ Sessions` to switch.

**Custom GIF doesn't animate**
- Ensure it's a true animated GIF (multi-frame). Static GIFs render as a still image.
