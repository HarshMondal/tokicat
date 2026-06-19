#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Data layer for the Claude Code desktop pet.

Reads Claude Code session transcripts (JSONL under
~/.claude/projects/<project-dir>/*.jsonl) and returns plain Python data
structures describing each session and its per-prompt scoreboard. NO GUI here,
NO external markup — the pet (pet.py) renders this however it likes.

STANDARD LIBRARY ONLY. Robust against partial/empty/garbage files and
missing/null fields (never raises out of the public functions).

Public API:
    find_sessions(limit=15) -> list[dict]   # recent sessions, newest first
    parse_session(path)      -> dict | None  # one session, fully parsed (mtime-cached)
    fmt_tokens(n) / fmt_elapsed(seconds)     # display helpers

Session dict:
    path, session_id, cwd, project, branch, title, mtime, is_live,
    total_prompts, total_tokens, wall_seconds, prompts[]
Prompt dict:
    index, title, full_text, out_tokens, elapsed, running, completed
"""

import os
import re
import glob
import json
import subprocess
from datetime import datetime, timezone

PROJECTS_DIR = os.path.expanduser("~/.claude/projects")
LIVE_WINDOW = 120          # session counts as "running" if touched within this many seconds
TITLE_WIDTH = 46           # per-prompt title truncation (panel is wider than the old conky line)

# Approximate USD pricing per MILLION tokens (edit if Anthropic pricing changes).
# cache-write = 1.25x input (5-min TTL) / 2x input (1-hour TTL); cache-read = 0.1x input.
PRICING = {
    "opus":   {"in": 5.0,  "out": 25.0, "cw5": 6.25, "cw1h": 10.0, "cr": 0.5},
    "sonnet": {"in": 3.0,  "out": 15.0, "cw5": 3.75, "cw1h": 6.0,  "cr": 0.3},
    "haiku":  {"in": 1.0,  "out": 5.0,  "cw5": 1.25, "cw1h": 2.0,  "cr": 0.1},
    "fable":  {"in": 10.0, "out": 50.0, "cw5": 12.5, "cw1h": 20.0, "cr": 1.0},
}


def _model_family(model):
    m = (model or "").lower()
    for fam in ("fable", "opus", "sonnet", "haiku"):
        if fam in m:
            return fam
    return "opus"            # sensible default for unknown / missing model ids

# NOTE (v2): Subagent / Task token usage lives in SEPARATE files under
#   ~/.claude/projects/<project>/<session-id>/subagents/agent-*.jsonl
# Ignored in v1 (and excluded from session discovery via the one-level glob).
# To fold them in later: for each Task tool_use in a turn, find the matching
# subagents/agent-*.jsonl and add its assistant output_tokens to that turn.

_cache = {}                # path -> (mtime, parsed_session_dict)


# ---------------------------------------------------------------------------
# Low-level parsing helpers
# ---------------------------------------------------------------------------
def _read_entries(path):
    try:
        f = open(path, "r", encoding="utf-8", errors="replace")
    except OSError:
        return
    with f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except (ValueError, TypeError):
                continue                       # partial last line, etc.
            if isinstance(obj, dict):
                yield obj


def _parse_ts(value):
    if not value or not isinstance(value, str):
        return None
    try:
        dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    except (ValueError, TypeError):
        return None


_META_PREFIXES = (
    "<local-command", "<bash-input", "<bash-stdout", "<bash-stderr",
    "<command-", "[request interrupted", "caveat:",
)


def _message_text(msg):
    """Human text of a user message, or None if it's a tool_result / empty."""
    content = msg.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        for block in content:
            if isinstance(block, dict) and block.get("type") == "tool_result":
                return None
        texts = [b.get("text", "") for b in content
                 if isinstance(b, dict) and b.get("type") == "text"]
        joined = "".join(texts).strip()
        return joined or None
    return None


def _is_real_prompt(entry, has_promptsource):
    """A prompt the human actually typed (promptSource=='typed'), with a legacy fallback."""
    if entry.get("type") != "user" or entry.get("isSidechain"):
        return False
    if has_promptsource:
        return entry.get("promptSource") == "typed"
    msg = entry.get("message")
    if not isinstance(msg, dict):
        return False
    text = _message_text(msg)
    if not text:
        return False
    return not text.lstrip().lower().startswith(_META_PREFIXES)


def _first_line(text, width=None):
    """First non-empty line of text. Truncated to `width` chars if given;
    width=None returns the full line (the GUI handles wrapping/eliding)."""
    line = ""
    for ln in (text or "").splitlines():
        ln = ln.strip()
        if ln:
            line = ln
            break
    if not line:
        line = "(empty prompt)"
    if width is not None and len(line) > width:
        line = line[: width - 1] + "…"
    return line


# ---------------------------------------------------------------------------
# Display helpers (shared with the GUI)
# ---------------------------------------------------------------------------
def fmt_tokens(n):
    if n is None:
        return "-"
    if n >= 1_000_000:
        return "{:.1f}M".format(n / 1_000_000)
    if n >= 1000:
        return "{:.1f}k".format(n / 1000)
    return str(int(n))


def fmt_elapsed(seconds):
    if seconds is None:
        return "-"
    seconds = max(0, int(round(seconds)))
    if seconds < 60:
        return "{}s".format(seconds)
    if seconds < 3600:
        m, s = divmod(seconds, 60)
        return "{}m{:02d}s".format(m, s)
    h, rem = divmod(seconds, 3600)
    return "{}h{:02d}m".format(h, rem // 60)


# ---------------------------------------------------------------------------
# Session parsing
# ---------------------------------------------------------------------------
def parse_session(path):
    """Fully parse one session into a dict (mtime-cached). Returns None on error."""
    try:
        mtime = os.path.getmtime(path)
    except OSError:
        return None
    cached = _cache.get(path)
    if cached and cached[0] == mtime:
        return cached[1]

    entries = list(_read_entries(path))

    has_promptsource = any(
        e.get("type") == "user" and "promptSource" in e for e in entries
    )

    ai_title = None
    cwd = None
    branch = None
    session_id = None
    slug = None
    first_prompt_text = None
    model = None
    tok = {"in": 0, "out": 0, "cr": 0, "cw5": 0, "cw1h": 0}   # session token totals

    turns = []
    current = None
    for entry in entries:
        etype = entry.get("type")

        # Cheap metadata harvested along the way.
        if etype == "ai-title" and entry.get("aiTitle"):
            ai_title = entry.get("aiTitle")
        if cwd is None and entry.get("cwd"):
            cwd = entry.get("cwd")
        if branch is None and entry.get("gitBranch"):
            branch = entry.get("gitBranch")
        if session_id is None and entry.get("sessionId"):
            session_id = entry.get("sessionId")
        if slug is None and entry.get("slug"):
            slug = entry.get("slug")

        if _is_real_prompt(entry, has_promptsource):
            text = _message_text(entry.get("message") or {}) or ""
            if first_prompt_text is None:
                first_prompt_text = text
            current = {
                "index": len(turns) + 1,
                "title": _first_line(text),          # full first line; GUI wraps it
                "full_text": text,
                "prompt_dt": _parse_ts(entry.get("timestamp")),
                "out_tokens": 0,
                "last_dt": None,
                "_ids": set(),
                "completed": False,
            }
            turns.append(current)
            continue

        if etype == "assistant" and current is not None:
            msg = entry.get("message")
            if not isinstance(msg, dict):
                continue
            ts = _parse_ts(entry.get("timestamp"))
            if ts is not None:
                current["last_dt"] = ts
            if msg.get("model"):
                model = msg.get("model")
            usage = msg.get("usage") or {}
            mid = msg.get("id")
            out = usage.get("output_tokens")
            if mid is not None and mid not in current["_ids"]:
                current["_ids"].add(mid)
                if isinstance(out, (int, float)):
                    current["out_tokens"] += int(out)
                # Session-wide token totals for the cost estimate (deduped by id).
                tok["out"] += int(out or 0)
                tok["in"] += int(usage.get("input_tokens") or 0)
                tok["cr"] += int(usage.get("cache_read_input_tokens") or 0)
                cc = usage.get("cache_creation") or {}
                cw1h = cc.get("ephemeral_1h_input_tokens")
                cw5 = cc.get("ephemeral_5m_input_tokens")
                if cw1h is not None or cw5 is not None:
                    tok["cw1h"] += int(cw1h or 0)
                    tok["cw5"] += int(cw5 or 0)
                else:                       # no breakdown: treat as 5-min cache writes
                    tok["cw5"] += int(usage.get("cache_creation_input_tokens") or 0)
            if msg.get("stop_reason") in ("end_turn", "stop_sequence", "max_tokens"):
                current["completed"] = True

    # Finalize prompts.
    prompts = []
    for i, t in enumerate(turns):
        newest = (i == len(turns) - 1)
        has_resp = bool(t["_ids"])
        elapsed = None
        if t["prompt_dt"] and t["last_dt"]:
            elapsed = (t["last_dt"] - t["prompt_dt"]).total_seconds()
        prompts.append({
            "index": t["index"],
            "title": t["title"],
            "full_text": t["full_text"],
            "out_tokens": t["out_tokens"],
            "elapsed": elapsed,
            "running": newest and not has_resp,
            "completed": t["completed"],
        })

    starts = [t["prompt_dt"] for t in turns if t["prompt_dt"]]
    ends = [t["last_dt"] for t in turns if t["last_dt"]] + starts
    wall = (max(ends) - min(starts)).total_seconds() if starts and ends else None

    title = ai_title or slug or (
        _first_line(first_prompt_text, 60) if first_prompt_text else None
    ) or (session_id[:8] if session_id else os.path.basename(path)[:8])

    project = os.path.basename(cwd) if cwd else _project_from_path(path)

    fam = _model_family(model)
    p = PRICING[fam]
    cost = (tok["in"] * p["in"] + tok["out"] * p["out"] + tok["cr"] * p["cr"]
            + tok["cw5"] * p["cw5"] + tok["cw1h"] * p["cw1h"]) / 1_000_000.0

    last_prompt_dt = turns[-1]["prompt_dt"] if turns else None

    result = {
        "path": path,
        "session_id": session_id or os.path.splitext(os.path.basename(path))[0],
        "cwd": cwd,
        "project": project,
        "branch": branch,
        "title": title,
        "model": model,
        "model_family": fam,
        "mtime": mtime,
        "is_live": False,                       # filled in by find_sessions()
        "open": False,                          # filled in by find_sessions() (WezTerm)
        "pane_id": None,
        "working": False,
        "waiting": False,
        "total_prompts": len(prompts),
        "total_tokens": sum(p2["out_tokens"] for p2 in prompts),
        "tokens": dict(tok),                    # full breakdown for cost
        "cost": cost,
        "last_prompt_ts": last_prompt_dt.timestamp() if last_prompt_dt else None,
        "last_completed": (not prompts[-1]["running"]) if prompts else True,
        "wall_seconds": wall,
        "prompts": prompts,
    }
    _cache[path] = (mtime, result)
    return result


def fmt_cost(cost):
    if cost is None:
        return "-"
    if cost >= 100:
        return "${:.0f}".format(cost)
    if cost >= 1:
        return "${:.2f}".format(cost)
    return "${:.3f}".format(cost)


def _project_from_path(path):
    """Derive a readable project name from the encoded directory slug."""
    try:
        slug = os.path.basename(os.path.dirname(path))      # e.g. -home-harsh-Documents-Foo
        parts = [p for p in slug.split("-") if p]
        return parts[-1] if parts else slug
    except Exception:
        return "session"


# Braille spinner glyphs Claude Code shows in the tab title while it's working.
_SPINNER = set("⠁⠂⠃⠄⠅⠆⠇⠈⠉⠊⠋⠌⠍⠎⠏⠐⠑⠒⠓⠔⠕⠖⠗⠘⠙⠚⠛⠜⠝⠞⠟"
               "⠠⠡⠢⠣⠤⠥⠦⠧⠨⠩⠪⠫⠬⠭⠮⠯⠰⠱⠲⠳⠴⠵⠶⠷⠸⠹⠺⠻⠼⠽⠾⠿⡀⢀")


def _norm_cwd(uri):
    """'file://host/home/harsh/Foo/' -> '/home/harsh/Foo'."""
    if not uri:
        return ""
    s = re.sub(r"^file://[^/]*", "", uri)        # strip scheme + host
    return s.rstrip("/")


def wezterm_panes():
    """Return open WezTerm panes as list of {pane_id, title, cwd, glyph}.
    Empty list if WezTerm/mux isn't reachable."""
    try:
        out = subprocess.run(["wezterm", "cli", "list", "--format", "json"],
                             capture_output=True, timeout=4, text=True)
        if out.returncode != 0:
            return []
        data = json.loads(out.stdout)
    except Exception:
        return []
    panes = []
    for p in data:
        title = (p.get("title") or "").strip()
        glyph = title[0] if title and not title[0].isalnum() else ""
        panes.append({
            "pane_id": p.get("pane_id"),
            "title": title,
            "cwd": _norm_cwd(p.get("cwd")),
            "glyph": glyph,
        })
    return panes


def _annotate_open(s, panes):
    """Set open/pane_id/working/waiting on a session from matching WezTerm panes."""
    scwd = (s.get("cwd") or "").rstrip("/")
    title = (s.get("title") or "").lower()
    for pane in panes:
        if pane["cwd"] and scwd and pane["cwd"] == scwd and title and title in pane["title"].lower():
            s["open"] = True
            s["pane_id"] = pane["pane_id"]
            s["working"] = pane["glyph"] in _SPINNER
            s["waiting"] = (not s["working"]) and s.get("last_completed", True)
            return
    # not open in any pane


def find_sessions(limit=25, detect_open=True):
    """Recent top-level sessions, newest first. `limit` caps the count;
    pass limit=None to return ALL sessions (used by search).

    Globs one level deep so subagent files are never included. is_live and the
    WezTerm open/working/waiting flags are set fresh on each call."""
    try:
        paths = glob.glob(os.path.join(PROJECTS_DIR, "*", "*.jsonl"))
    except Exception:
        return []
    paths_with_mtime = []
    for p in paths:
        try:
            paths_with_mtime.append((os.path.getmtime(p), p))
        except OSError:
            continue
    paths_with_mtime.sort(reverse=True)
    if limit is not None:
        paths_with_mtime = paths_with_mtime[: max(1, limit)]

    panes = wezterm_panes() if detect_open else []
    now = datetime.now(timezone.utc).timestamp()
    out = []
    for mtime, p in paths_with_mtime:
        s = parse_session(p)
        if not s:
            continue
        s = dict(s)                              # don't mutate the cached copy
        s["is_live"] = (now - mtime) <= LIVE_WINDOW
        s["open"] = False
        s["pane_id"] = None
        s["working"] = False
        s["waiting"] = False
        if panes:
            _annotate_open(s, panes)
        out.append(s)
    return out


def today_totals(sessions_list):
    """Sum prompts/tokens/cost across sessions whose last activity is today (local)."""
    today = datetime.now().date()
    n = t = 0
    cost = 0.0
    for s in sessions_list:
        try:
            if datetime.fromtimestamp(s["mtime"]).date() == today:
                n += s["total_prompts"]
                t += s["total_tokens"]
                cost += s.get("cost") or 0.0
        except Exception:
            continue
    return {"prompts": n, "tokens": t, "cost": cost}


def is_today(s):
    try:
        return datetime.fromtimestamp(s["mtime"]).date() == datetime.now().date()
    except Exception:
        return False


# ---------------------------------------------------------------------------
# CLI smoke test (no GUI): python3 sessions.py [path]
# ---------------------------------------------------------------------------
if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1:
        s = parse_session(os.path.expanduser(sys.argv[1]))
        sess = [s] if s else []
    else:
        sess = find_sessions()
    if not sess:
        print("no sessions found under", PROJECTS_DIR)
    for s in sess:
        live = " [LIVE]" if s.get("is_live") else ""
        print("\n== {} ({}{}){}".format(
            s["title"], s["project"],
            "@" + s["branch"] if s.get("branch") else "", live))
        for p in s["prompts"][-12:]:
            tok = "running..." if p["running"] else "{:>6} tok  {:>7}".format(
                fmt_tokens(p["out_tokens"]), fmt_elapsed(p["elapsed"]))
            print("  {:>2} {:<46} {}".format(p["index"], p["title"], tok))
        print("  -- {} prompts | {} tok | {}".format(
            s["total_prompts"], fmt_tokens(s["total_tokens"]),
            fmt_elapsed(s["wall_seconds"])))
