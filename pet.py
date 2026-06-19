#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Claude Code desktop pet (GTK3 / PyGObject).

A small always-on-top creature that sits on your desktop. Click it to pop up a
panel:
  * first time -> a list of your recent / running Claude Code sessions
  * click a session -> its prompt-by-prompt scoreboard (tokens + elapsed)
  * next time you click the pet -> it jumps straight back to the last session you
    opened (with a "Sessions" button to go back to the list)

Drag the pet to reposition it (position is remembered). Esc or the pet itself
closes the panel.

Art:  drop  assets/pet.gif  (animated) or  assets/pet.png/.jpg  next to this file
and the pet uses it automatically. With no art file, a built-in placeholder
mascot is drawn so the pet is always visible.

Dependencies: python3-gi, gir1.2-gtk-3.0, python3-gi-cairo  (all preinstalled on
Ubuntu GNOME). No pip packages. Data comes from sessions.py (stdlib only).
"""

import os
import sys
import json
import math
import time
import random
import shlex
import subprocess

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib, Pango  # noqa: E402
import cairo  # noqa: E402

import sessions  # local data layer

# ---------------------------------------------------------------------------
# Paths / constants
# ---------------------------------------------------------------------------
HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "assets")
STATE_DIR = os.path.expanduser("~/.config/claude-pet")
STATE_FILE = os.path.join(STATE_DIR, "state.json")

PET_MAX = 110          # max pet sprite size (px); art is scaled to fit
DRAG_THRESHOLD = 6     # px of movement before a press counts as a drag, not a click
REFRESH_SECS = 2       # live refresh cadence while the panel is open
TICK_MS = 40           # animation/bob timer interval (ms)
PANEL_W = 480
PANEL_H = 540

# Edge de-matte: many GIFs/PNGs have a dark "halo" baked around the art (the
# subject was anti-aliased against a black background, or has a black contour).
# We erode dark pixels that touch transparency, peeling that ring off without
# holing the interior. Done once at load, on the already-scaled (small) frames.
TRIM_EDGE = True       # set False to disable and show art exactly as-is
TRIM_DARK = 70         # a colour channel below this counts as "dark"
TRIM_ITERS = 2         # how many 1px layers of dark edge to peel

# --- Pet "personality": wandering + petting -------------------------------
TELEPORT_MIN_S = 10 * 60       # random teleport: shortest wait
TELEPORT_MAX_S = 30 * 60       # ............... longest wait
GLIDE_MS = 900                 # how long the smooth glide across screen takes

PET_INTERVAL_S = 3 * 3600      # how often it demands petting (~3 hours)
PET_CHECK_S = 60               # how often we check whether it's petting time
PETS_REQUIRED = 10             # clicks needed to satisfy it
PET_GROW_SCALE = 2.6           # how much bigger it gets while demanding petting
GROW_MS = 500                  # grow-to-center / shrink-back animation time

ATTENTION_POLL_S = 5           # how often to check if a session needs attention
ATTENTION_WINDOW_S = 600       # only nag about sessions touched within this window

# Debug: set CLAUDE_PET_DEBUG=1 to shrink all timers so the behaviours are easy
# to see/test (teleport every few seconds, petting after ~20s).
if os.environ.get("CLAUDE_PET_DEBUG"):
    TELEPORT_MIN_S, TELEPORT_MAX_S = 4, 8
    PET_INTERVAL_S, PET_CHECK_S = 20, 3

# Token-heat colours for the scoreboard rows (CSS classes defined below).
CSS = b"""
.pet-panel { background-color: rgba(20,22,28,0.97);
             border-radius: 16px;
             border: 1px solid rgba(255,255,255,0.07); }
.panel-pad { padding: 18px 18px 14px 18px; }

/* header */
.hdr-title { font-weight: bold; font-size: 17px; color: #F1F3F5; }
.hdr-sub   { font-size: 12px; color: #828A93; }
.navbtn    { background: rgba(255,255,255,0.07); border: none; color: #9AE6B4;
             padding: 5px 13px; border-radius: 9px; font-size: 13px; }
.navbtn:hover { background: rgba(255,255,255,0.16); }
.closebtn  { background: transparent; border: none; color: #828A93;
             padding: 0 6px; font-size: 18px; }
.closebtn:hover { color: #FF6B6B; }
.rule      { background-color: rgba(255,255,255,0.07); min-height: 1px; }

/* session list rows */
list, row { background: transparent; }
row { padding: 2px 0; }
row:hover { background-color: rgba(255,255,255,0.06); border-radius: 10px; }
.sess-name { color: #ECEFF2; font-weight: bold; font-size: 14px; }
.sess-meta { color: #828A93; font-size: 12px; }
.chip      { background: rgba(255,255,255,0.08); color: #C6CBD2;
             border-radius: 7px; padding: 0 7px; font-size: 12px; }
.live-dot  { color: #7CE38B; font-weight: bold; font-size: 15px; }
.openbtn   { background: rgba(124,227,139,0.14); border: none; color: #9AE6B4;
             border-radius: 8px; padding: 2px 9px; font-size: 12px; min-height: 0; }
.openbtn:hover { background: rgba(124,227,139,0.30); color: #C9F7D2; }
.newbtn    { background: rgba(138,180,248,0.16); border: none; color: #8AB4F8;
             border-radius: 8px; padding: 3px 11px; font-size: 13px; min-height: 0; }
.newbtn:hover { background: rgba(138,180,248,0.30); color: #BBD2FB; }
.pinbtn    { background: transparent; border: none; color: #5A626D;
             padding: 0 4px; font-size: 13px; min-height: 0; }
.pinbtn:hover { color: #FFD166; }
.pinned    { color: #FFD166; }
.section   { color: #828A93; font-size: 10px; font-weight: bold; letter-spacing: 1px;
             margin: 8px 2px 2px 2px; }
.dot-work  { color: #7CE38B; font-weight: bold; font-size: 13px; }
.dot-wait  { color: #FFD166; font-weight: bold; font-size: 13px; }
.dot-off   { color: #4A4F58; font-size: 13px; }
.cost      { color: #9AE6B4; font-size: 11px; }
.todaybtn  { background: rgba(255,255,255,0.06); border: none; color: #C6CBD2;
             border-radius: 8px; padding: 2px 10px; font-size: 11px; min-height: 0; }
.todaybtn-on { background: rgba(138,180,248,0.28); color: #BBD2FB; }
.searchbar entry, .searchbar { background: rgba(255,255,255,0.06); color: #ECEFF2;
             border-radius: 10px; border: none; font-size: 13px; padding: 4px 8px; }
.searchbar image { color: #828A93; }

/* prompt history rows */
.prow        { border-radius: 9px; padding: 6px 8px; }
.prow:hover  { background-color: rgba(255,255,255,0.05); }
.idx         { color: #5A626D; font-family: monospace; font-size: 13px; }
.ptitle      { color: #DCE0E5; font-size: 14px; }
.ptitle-live { color: #F1F3F5; font-size: 14px; font-weight: bold; }
.tok-heavy   { color: #FF6B6B; font-family: monospace; font-size: 13px; font-weight: bold; }
.tok-mid     { color: #FFD166; font-family: monospace; font-size: 13px; font-weight: bold; }
.tok-light   { color: #8AB4F8; font-family: monospace; font-size: 13px; }
.elapsed     { color: #828A93; font-family: monospace; font-size: 12px; }
.running     { color: #101218; background: #7CE38B; font-weight: bold;
               font-size: 12px; border-radius: 7px; padding: 2px 9px; }

/* footer summary */
.foot-box   { background: rgba(255,255,255,0.05); border-radius: 10px;
              padding: 9px 14px; margin-top: 8px; }
.foot-num   { color: #F1F3F5; font-weight: bold; font-size: 16px; }
.foot-lbl   { color: #828A93; font-size: 11px; }
.empty      { color: #828A93; font-size: 14px; }
"""


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------
def load_state():
    try:
        with open(STATE_FILE, "r", encoding="utf-8") as f:
            d = json.load(f)
            return d if isinstance(d, dict) else {}
    except (OSError, ValueError):
        return {}


def save_state(state):
    try:
        os.makedirs(STATE_DIR, exist_ok=True)
        with open(STATE_FILE, "w", encoding="utf-8") as f:
            json.dump(state, f)
    except OSError:
        pass


def open_in_wezterm(session):
    """If the session is already open in WezTerm, focus that tab/pane. Otherwise
    open a NEW tab in the session's directory and resume it (claude --resume <id>),
    falling back to a new WezTerm window. Returns True on success."""
    # 0) already open -> just focus its existing pane/tab
    pane_id = session.get("pane_id")
    if session.get("open") and pane_id is not None:
        try:
            r = subprocess.run(["wezterm", "cli", "activate-pane", "--pane-id", str(pane_id)],
                               capture_output=True, timeout=4)
            if r.returncode == 0:
                return True
        except Exception:
            pass
        # fall through to spawning a fresh one if activation failed

    cwd = session.get("cwd")
    if not cwd or not os.path.isdir(cwd):
        cwd = os.path.expanduser("~")
    sid = session.get("session_id") or ""
    if sid:
        # resume the session; drop to a shell afterwards so the tab stays open
        inner = "claude --resume {} ; exec bash".format(shlex.quote(sid))
    else:
        inner = "exec claude"
    return _wezterm_spawn(cwd, inner)


def new_session_in_wezterm(cwd):
    """Open a NEW WezTerm tab in `cwd` running a fresh `claude` session."""
    if not cwd or not os.path.isdir(cwd):
        cwd = os.path.expanduser("~")
    return _wezterm_spawn(cwd, "claude ; exec bash")


def _wezterm_spawn(cwd, inner):
    prog = ["bash", "-lc", inner]
    try:                                           # new tab in the running WezTerm
        r = subprocess.run(["wezterm", "cli", "spawn", "--cwd", cwd, "--"] + prog,
                           capture_output=True, timeout=8)
        if r.returncode == 0:
            return True
    except Exception:
        pass
    try:                                           # fallback: brand-new window
        subprocess.Popen(["wezterm", "start", "--cwd", cwd, "--"] + prog)
        return True
    except Exception:
        return False


def find_art():
    """Return a path to a pet image in assets/, or None for the placeholder."""
    for name in ("pet.gif", "pet.png", "pet.jpg", "pet.jpeg", "pet.webp"):
        p = os.path.join(ASSETS, name)
        if os.path.isfile(p):
            return p
    return None


def dematte(pb, dark=TRIM_DARK, iters=TRIM_ITERS):
    """Erode the dark edge halo of a pixbuf: dark pixels touching transparency
    are made transparent, `iters` layers deep. Interior dark pixels are kept,
    so a black cat outline gets trimmed without holes punched in the cat.
    Operates on small (scaled) frames, so it's cheap. Returns a new pixbuf."""
    if not pb.get_has_alpha():
        pb = pb.add_alpha(False, 0, 0, 0)
    w, h = pb.get_width(), pb.get_height()
    rs, nc = pb.get_rowstride(), pb.get_n_channels()
    if nc < 4:
        return pb
    buf = bytearray(pb.get_pixels())

    def alpha(x, y):
        if 0 <= x < w and 0 <= y < h:
            return buf[y * rs + x * nc + 3]
        return 0                                  # off-canvas counts as transparent

    for _ in range(iters):
        clear = []
        for y in range(h):
            base = y * rs
            for x in range(w):
                i = base + x * nc
                if buf[i + 3] == 0:
                    continue
                if buf[i] < dark and buf[i + 1] < dark and buf[i + 2] < dark:
                    if (alpha(x - 1, y) < 128 or alpha(x + 1, y) < 128 or
                            alpha(x, y - 1) < 128 or alpha(x, y + 1) < 128 or
                            alpha(x - 1, y - 1) < 128 or alpha(x + 1, y - 1) < 128 or
                            alpha(x - 1, y + 1) < 128 or alpha(x + 1, y + 1) < 128):
                        clear.append(i + 3)
        if not clear:
            break
        for i in clear:
            buf[i] = 0
    return GdkPixbuf.Pixbuf.new_from_bytes(
        GLib.Bytes.new(bytes(buf)), GdkPixbuf.Colorspace.RGB, True, 8, w, h, rs)


def clear_transparent(cr):
    """Paint the whole cairo surface fully transparent (correct CLEAR), then
    leave the context in OVER mode for normal drawing on top."""
    cr.save()
    cr.set_source_rgba(0, 0, 0, 0)
    cr.set_operator(cairo.OPERATOR_SOURCE)
    cr.paint()
    cr.restore()
    cr.set_operator(cairo.OPERATOR_OVER)


# Heat colours (r,g,b 0..1) matching the .tok-* CSS classes.
HEAT_RGB = {
    "tok-heavy": (1.00, 0.42, 0.42),
    "tok-mid":   (1.00, 0.82, 0.40),
    "tok-light": (0.54, 0.71, 0.97),
}


class HeatBar(Gtk.DrawingArea):
    """A small rounded sparkline bar: filled fraction = ratio, tinted by colour."""

    def __init__(self, ratio, rgb, width=64, height=7):
        super().__init__()
        self.ratio = max(0.0, min(1.0, ratio))
        self.rgb = rgb
        self.set_size_request(width, height)
        self.set_valign(Gtk.Align.CENTER)
        self.connect("draw", self._draw)

    @staticmethod
    def _round_rect(cr, x, y, w, h, r):
        if w <= 0:
            return
        r = min(r, w / 2, h / 2)
        cr.new_sub_path()
        cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
        cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
        cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
        cr.arc(x + r, y + r, r, math.pi, 1.5 * math.pi)
        cr.close_path()

    def _draw(self, _w, cr):
        a = self.get_allocated_width()
        h = self.get_allocated_height()
        # track
        cr.set_source_rgba(1, 1, 1, 0.08)
        self._round_rect(cr, 0, 0, a, h, h / 2)
        cr.fill()
        # fill
        r, g, b = self.rgb
        cr.set_source_rgba(r, g, b, 0.92)
        self._round_rect(cr, 0, 0, a * self.ratio, h, h / 2)
        cr.fill()
        return False


def apply_rgba(window):
    """Give a window a transparent (RGBA) visual so rounded/alpha bits show."""
    screen = window.get_screen()
    visual = screen.get_rgba_visual()
    if visual is not None:
        window.set_visual(visual)
    window.set_app_paintable(True)


def monitor_geometry(window):
    """Work-area geometry of the monitor the window is on (non-deprecated path)."""
    try:
        display = window.get_display()
        monitor = (display.get_monitor_at_window(window.get_window())
                   if window.get_window() else None) or display.get_monitor(0)
        return monitor.get_geometry()
    except Exception:
        # Fallback for very old GTK; values only used for clamping.
        class _G:
            x = y = 0
            width = 1920
            height = 1080
        return _G()


def heat_class(out_tokens, max_out):
    if max_out > 0 and out_tokens >= max_out * 0.66 and out_tokens > 0:
        return "tok-heavy"
    if out_tokens >= max_out * 0.33 and out_tokens > 0:
        return "tok-mid"
    return "tok-light"


# ---------------------------------------------------------------------------
# The pet window
# ---------------------------------------------------------------------------
class Pet(Gtk.Window):
    def __init__(self, app):
        super().__init__(type=Gtk.WindowType.TOPLEVEL)
        self.app = app

        self.set_decorated(False)
        self.set_resizable(False)
        self.set_skip_taskbar_hint(True)
        self.set_skip_pager_hint(True)
        self.set_keep_above(True)
        self.set_accept_focus(False)
        self.set_type_hint(Gdk.WindowTypeHint.UTILITY)
        self.stick()
        apply_rgba(self)

        # Art state. Playback uses the animation iterator's REAL-TIME advance()
        # (deterministic), while each unique frame is scaled + edge-trimmed once
        # and cached by pixel-hash. No art -> _cur stays None -> placeholder.
        self.anim_iter = None          # GdkPixbuf.PixbufAnimationIter for GIFs
        self._cur = None               # current ready-to-blit (scaled, trimmed) pixbuf
        self._cache = {}               # frame pixel-hash -> processed pixbuf
        self._load_art()

        if self._cur is not None:
            size = max(self._cur.get_width(), self._cur.get_height())
        else:
            size = PET_MAX
        self.base_size = max(48, min(PET_MAX, size))   # size at scale 1.0
        self.scale = 1.0
        self.size = self.base_size

        # Personality state machine: "normal" | "petting" | "celebrate".
        # Position/scale transitions run through a single anim descriptor.
        self.state = "normal"
        self._anim = None          # {t,dur,cx0,cy0,cx1,cy1,s0,s1,done}
        self._pets = 0
        self._hearts = []          # floating heart particles
        self._wiggle = 0.0         # decaying wiggle amplitude (px)
        self._bounce = 0.0         # decaying bounce amplitude (px), for attention
        self._home = None          # (cx, cy) to return to after petting
        last = self.app.state.get("last_pet_time")
        self._last_pet = last if isinstance(last, (int, float)) else time.time()

        self.area = Gtk.DrawingArea()
        self.area.set_size_request(self.size, self.size)
        self.area.add_events(
            Gdk.EventMask.BUTTON_PRESS_MASK
            | Gdk.EventMask.BUTTON_RELEASE_MASK
            | Gdk.EventMask.POINTER_MOTION_MASK
        )
        self.area.connect("draw", self._on_draw)
        self.area.connect("button-press-event", self._on_press)
        self.area.connect("button-release-event", self._on_release)
        self.area.connect("motion-notify-event", self._on_motion)
        self.add(self.area)

        # Drag bookkeeping.
        self._press = None
        self._dragging = False

        # Restore saved position (default: top-right-ish).
        st = self.app.state
        x = st.get("pet_x")
        y = st.get("pet_y")
        self.move(x if isinstance(x, int) else self._default_x(),
                  y if isinstance(y, int) else 60)

        # Animation / liveliness tick.
        self._bob = 0
        GLib.timeout_add(TICK_MS, self._tick)

        # Wander + petting schedulers.
        self._schedule_teleport()
        GLib.timeout_add_seconds(PET_CHECK_S, self._check_petting)

    # ---- art -------------------------------------------------------------
    def _load_art(self):
        """Set up animation playback (or a single static frame) from assets/.
        On any failure, leaves _cur None so the placeholder mascot is drawn."""
        path = find_art()
        if not path:
            return
        try:
            if path.lower().endswith(".gif"):
                anim = GdkPixbuf.PixbufAnimation.new_from_file(path)
                if not anim.is_static_image():
                    self.anim_iter = anim.get_iter(None)   # real-time iterator
                    self._refresh_frame()                  # prime _cur
                    return
                self._cur = self._process(anim.get_static_image())
            else:
                self._cur = self._process(GdkPixbuf.Pixbuf.new_from_file(path))
        except Exception:
            self.anim_iter = None
            self._cur = None

    def _process(self, pb):
        """Scale to display size, then edge-trim (de-matte). Result is cached
        upstream by frame hash, so this runs once per unique frame."""
        pb = self._fit(pb)
        if TRIM_EDGE:
            pb = dematte(pb)
        return pb

    def _refresh_frame(self):
        """Update _cur from the current animation frame, using the de-matte cache."""
        if self.anim_iter is None:
            return
        pb = self.anim_iter.get_pixbuf()
        key = hash(bytes(pb.get_pixels()))
        cached = self._cache.get(key)
        if cached is None:
            cached = self._process(pb)
            self._cache[key] = cached
        self._cur = cached

    def _fit(self, pb):
        w, h = pb.get_width(), pb.get_height()
        if max(w, h) <= PET_MAX:
            return pb
        scale = PET_MAX / float(max(w, h))
        return pb.scale_simple(max(1, int(w * scale)), max(1, int(h * scale)),
                               GdkPixbuf.InterpType.BILINEAR)

    def _default_x(self):
        geo = monitor_geometry(self)
        return geo.x + geo.width - PET_MAX - 40

    # ---- drawing ---------------------------------------------------------
    def _on_draw(self, _area, cr):
        clear_transparent(cr)
        bob = 3 * math.sin(self._bob * TICK_MS / 400.0)        # gentle vertical bob
        bob -= self._bounce * abs(math.sin(self._bob * 0.6))   # attention hop (upward)
        wig = self._wiggle * math.sin(self._bob * 0.9)         # decaying side wiggle
        sc = self.scale

        pb = self._cur
        if pb is not None:
            pw, ph = pb.get_width() * sc, pb.get_height() * sc
            ox = (self.size - pw) / 2 + wig
            oy = (self.size - ph) / 2 + bob * sc
            cr.save()
            cr.translate(ox, oy)
            cr.scale(sc, sc)
            Gdk.cairo_set_source_pixbuf(cr, pb, 0, 0)
            cr.paint()
            cr.restore()
        else:
            cr.save()
            cr.translate(self.size / 2 + wig, self.size / 2)
            cr.scale(sc, sc)
            cr.translate(-self.base_size / 2, -self.base_size / 2)
            self._draw_placeholder(cr, bob)
            cr.restore()

        if self.state in ("petting", "celebrate"):
            self._draw_pet_prompt(cr)
        self._draw_hearts(cr)

        # Attention badge: red "!" bubble when a session is waiting on you.
        if self.app.attention:
            n = len(self.app.attention)
            bx, by, r = self.size - 12, 12, 10
            cr.set_source_rgba(1.0, 0.30, 0.32, 0.97)
            cr.arc(bx, by, r, 0, 2 * math.pi)
            cr.fill()
            cr.set_source_rgba(1, 1, 1, 1)
            cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
            cr.set_font_size(14)
            label = "!" if n == 1 else str(n)
            ext = cr.text_extents(label)
            cr.move_to(bx - ext.width / 2 - ext.x_bearing, by - ext.height / 2 - ext.y_bearing)
            cr.show_text(label)
        elif self.scale < 1.2 and self.app.any_live():
            # Tiny green dot if any session is live (only at normal size).
            cr.set_source_rgba(0.49, 0.89, 0.55, 0.95)
            cr.arc(self.size - 10, 10, 5, 0, 2 * math.pi)
            cr.fill()
        return False

    def _draw_pet_prompt(self, cr):
        """A pulsing 'pet me!' banner + remaining-count while it wants petting."""
        remaining = max(0, PETS_REQUIRED - self._pets)
        text = "pet me!" if self.state == "petting" else "yay! ♥"
        if self.state == "petting" and remaining < PETS_REQUIRED:
            text = "more! {}".format(remaining)
        pulse = 0.5 + 0.5 * abs(math.sin(self._bob * 0.12))
        cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
        cr.set_font_size(15)
        ext = cr.text_extents(text)
        tx = (self.size - ext.width) / 2
        ty = 22
        # soft dark pill behind the text
        cr.set_source_rgba(0.06, 0.07, 0.09, 0.55 + 0.25 * pulse)
        HeatBar._round_rect(cr, tx - 10, ty - 16, ext.width + 20, 24, 12)
        cr.fill()
        cr.set_source_rgba(1.0, 0.45, 0.62, 0.85 + 0.15 * pulse)
        cr.move_to(tx, ty)
        cr.show_text(text)

    def _draw_hearts(self, cr):
        for h in self._hearts:
            a = max(0.0, min(1.0, h["life"]))
            self._heart_path(cr, h["x"], h["y"], h["r"])
            cr.set_source_rgba(1.0, 0.36, 0.52, a)
            cr.fill()

    @staticmethod
    def _heart_path(cr, x, y, r):
        cr.new_sub_path()
        cr.move_to(x, y + r * 0.3)
        cr.curve_to(x, y - r * 0.3, x - r, y - r * 0.3, x - r, y + r * 0.2)
        cr.curve_to(x - r, y + r * 0.6, x, y + r * 0.9, x, y + r * 1.2)
        cr.curve_to(x, y + r * 0.9, x + r, y + r * 0.6, x + r, y + r * 0.2)
        cr.curve_to(x + r, y - r * 0.3, x, y - r * 0.3, x, y + r * 0.3)
        cr.close_path()

    def _draw_placeholder(self, cr, bob):
        """A simple coin/token mascot, drawn so the pet is visible without art."""
        s = self.size
        cx, cy, r = s / 2.0, s / 2.0 + bob, s * 0.40
        # coin body
        cr.set_source_rgba(1.0, 0.82, 0.30, 0.97)
        cr.arc(cx, cy, r, 0, 2 * math.pi)
        cr.fill()
        cr.set_source_rgba(0.85, 0.62, 0.10, 1.0)
        cr.set_line_width(3)
        cr.arc(cx, cy, r, 0, 2 * math.pi)
        cr.stroke()
        # eyes
        cr.set_source_rgba(0.15, 0.12, 0.05, 1.0)
        cr.arc(cx - r * 0.35, cy - r * 0.12, r * 0.12, 0, 2 * math.pi)
        cr.fill()
        cr.arc(cx + r * 0.35, cy - r * 0.12, r * 0.12, 0, 2 * math.pi)
        cr.fill()
        # smile
        cr.set_line_width(2.5)
        cr.arc(cx, cy + r * 0.05, r * 0.45, 0.15 * math.pi, 0.85 * math.pi)
        cr.stroke()

    # ---- liveliness / animation tick ------------------------------------
    def _tick(self):
        self._bob += 1
        if self.anim_iter is not None:
            # Real-time advance (deterministic). Re-derive the current frame only
            # when it actually changes, to keep the de-matte cache cheap.
            if self.anim_iter.advance(None):
                self._refresh_frame()
        self._step_anim()
        self._step_hearts()
        if self._wiggle > 0.05:
            self._wiggle *= 0.85
        else:
            self._wiggle = 0.0
        # Bounce while any session needs attention (re-energize as it decays).
        if self.app.attention:
            if self._bounce < 1.0:
                self._bounce = 14.0
        if self._bounce > 0.3:
            self._bounce *= 0.90
        else:
            self._bounce = 0.0
        self.area.queue_draw()
        return True

    def bounce(self):
        self._bounce = 16.0

    # ---- position / scale animation -------------------------------------
    def _center(self):
        x, y = self.get_position()
        return (x + self.size / 2.0, y + self.size / 2.0)

    def _start_anim(self, cx1, cy1, s1, dur, done=None):
        cx0, cy0 = self._center()
        self._anim = {"t": 0.0, "dur": float(dur), "cx0": cx0, "cy0": cy0,
                      "cx1": cx1, "cy1": cy1, "s0": self.scale, "s1": s1,
                      "done": done}

    def _step_anim(self):
        a = self._anim
        if a is None:
            return
        a["t"] += TICK_MS
        f = min(1.0, a["t"] / a["dur"])
        e = f * f * (3 - 2 * f)                    # smoothstep ease in/out
        self.scale = a["s0"] + (a["s1"] - a["s0"]) * e
        self.size = max(8, int(round(self.base_size * self.scale)))
        cx = a["cx0"] + (a["cx1"] - a["cx0"]) * e
        cy = a["cy0"] + (a["cy1"] - a["cy0"]) * e
        self.area.set_size_request(self.size, self.size)
        self.resize(self.size, self.size)
        self.move(int(cx - self.size / 2), int(cy - self.size / 2))
        if f >= 1.0:
            done = a["done"]
            self._anim = None
            if done:
                done()

    def _clamp_center(self, cx, cy, size):
        geo = monitor_geometry(self)
        half = size / 2.0
        cx = max(geo.x + half, min(cx, geo.x + geo.width - half))
        cy = max(geo.y + half, min(cy, geo.y + geo.height - half))
        return cx, cy

    # ---- random teleport (smooth glide) ---------------------------------
    def _schedule_teleport(self):
        delay = random.randint(TELEPORT_MIN_S, TELEPORT_MAX_S) * 1000
        GLib.timeout_add(delay, self._do_teleport)

    def _do_teleport(self):
        # Don't wander while busy, being petted, or while the panel is open.
        if (self.state == "normal" and self._anim is None
                and not self.app.panel.get_visible()):
            geo = monitor_geometry(self)
            cx = random.uniform(geo.x + self.size, geo.x + geo.width - self.size)
            cy = random.uniform(geo.y + self.size, geo.y + geo.height - self.size)
            cx, cy = self._clamp_center(cx, cy, self.size)
            self._start_anim(cx, cy, 1.0, GLIDE_MS)
        self._schedule_teleport()
        return False                               # one-shot; rescheduled above

    # ---- petting cycle ---------------------------------------------------
    def _check_petting(self):
        if (self.state == "normal" and self._anim is None
                and time.time() - self._last_pet >= PET_INTERVAL_S):
            self._enter_petting()
        return True                                # keep checking

    def _enter_petting(self):
        if self.app.panel.get_visible():
            self.app.panel.hide_panel()
        self.state = "petting"
        self._pets = 0
        self._home = self._center()                # remember where to go back
        geo = monitor_geometry(self)
        cx = geo.x + geo.width / 2.0
        cy = geo.y + geo.height / 2.0
        self._start_anim(cx, cy, PET_GROW_SCALE, GROW_MS)

    def _on_pet(self):
        self._pets += 1
        self._wiggle = 10.0
        self._spawn_hearts(3)
        if self._pets >= PETS_REQUIRED:
            self._celebrate()

    def _celebrate(self):
        self.state = "celebrate"
        self._spawn_hearts(14)
        self._wiggle = 16.0
        cx, cy = self._home or self._center()

        def done():
            self.state = "normal"
            self._last_pet = time.time()
            self.app.state["last_pet_time"] = self._last_pet
            save_state(self.app.state)

        def shrink_back():
            self._start_anim(cx, cy, 1.0, GROW_MS, done)
            return False                           # one-shot timer

        # brief pause to enjoy the hearts, then shrink + return home
        GLib.timeout_add(450, shrink_back)

    def _spawn_hearts(self, n):
        for _ in range(n):
            self._hearts.append({
                "x": self.size / 2 + random.uniform(-self.size * 0.25, self.size * 0.25),
                "y": self.size * 0.5 + random.uniform(-10, 10),
                "vy": random.uniform(0.6, 1.6),
                "r": random.uniform(4, 9),
                "life": 1.0,
            })

    def _step_hearts(self):
        for h in self._hearts:
            h["y"] -= h["vy"] * (TICK_MS / 16.0)
            h["life"] -= TICK_MS / 900.0
        self._hearts = [h for h in self._hearts if h["life"] > 0]

    # ---- mouse: click vs drag -------------------------------------------
    def _on_press(self, _w, event):
        if event.button == 1:
            wx, wy = self.get_position()
            self._press = (event.x_root, event.y_root, wx, wy)
            self._dragging = False
        return False

    def _on_motion(self, _w, event):
        # No dragging while it's grown/centered for petting.
        if not self._press or self.state != "normal":
            return False
        x0, y0, wx, wy = self._press
        dx, dy = event.x_root - x0, event.y_root - y0
        if self._dragging or abs(dx) > DRAG_THRESHOLD or abs(dy) > DRAG_THRESHOLD:
            self._dragging = True
            self._anim = None                      # cancel any glide while dragging
            self.move(int(wx + dx), int(wy + dy))
        return False

    def _on_release(self, _w, event):
        if event.button != 1 or not self._press:
            return False
        self._press = None

        # While it wants petting, clicks are pets — they don't open the panel.
        if self.state == "petting":
            self._on_pet()
            return False
        if self.state == "celebrate":
            return False                           # busy being happy

        if self._dragging:
            x, y = self.get_position()
            self.app.state["pet_x"] = int(x)
            self.app.state["pet_y"] = int(y)
            save_state(self.app.state)
        else:
            self._anim = None                      # stop a glide so the panel anchors
            self.app.toggle_panel(self)
        return False


# ---------------------------------------------------------------------------
# The panel window (sessions list + prompt history)
# ---------------------------------------------------------------------------
class Panel(Gtk.Window):
    def __init__(self, app):
        super().__init__(type=Gtk.WindowType.TOPLEVEL)
        self.app = app
        self.view = "list"          # "list" | "history"
        self.session_path = None    # currently shown session
        self._refresh_id = None
        self._expanded = set()      # prompt indices expanded in the history view
        self._expanded_for = None   # which session path _expanded belongs to

        self.set_decorated(False)
        self.set_skip_taskbar_hint(True)
        self.set_skip_pager_hint(True)
        self.set_keep_above(True)
        self.set_type_hint(Gdk.WindowTypeHint.DIALOG)
        self.set_default_size(PANEL_W, PANEL_H)
        self.stick()
        apply_rgba(self)

        self.connect("draw", self._draw_bg)
        self.connect("key-press-event", self._on_key)
        self.connect("delete-event", lambda *_: self.hide_panel() or True)

        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        outer.get_style_context().add_class("pet-panel")
        outer.get_style_context().add_class("panel-pad")
        self.add(outer)

        # Header (changes per view)
        self.header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        outer.pack_start(self.header, False, False, 0)

        # Persistent search bar (visible only in the session-list view).
        self._search_query = ""
        self._today_only = False
        self.search_entry = Gtk.SearchEntry()
        self.search_entry.set_placeholder_text("search title, folder or branch…")
        self.search_entry.get_style_context().add_class("searchbar")
        self.search_entry.connect("search-changed", self._on_search)
        self.searchbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        self.searchbar.set_margin_top(8)
        self.searchbar.pack_start(self.search_entry, True, True, 0)
        outer.pack_start(self.searchbar, False, False, 0)

        # Body (scrollable)
        self.scroll = Gtk.ScrolledWindow()
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroll.set_vexpand(True)
        outer.pack_start(self.scroll, True, True, 8)

        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        self.scroll.add(self.body)

        # Footer (rebuilt per view: a summary box for history, a hint for the list)
        self.footer = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        outer.pack_start(self.footer, False, False, 0)

    # ---- transparent rounded background ---------------------------------
    def _draw_bg(self, _w, cr):
        # Clear the window to fully transparent; the rounded dark panel is then
        # painted by the .pet-panel CSS, so the corners stay see-through.
        clear_transparent(cr)
        return False

    def _on_key(self, _w, event):
        if event.keyval == Gdk.KEY_Escape:
            self.hide_panel()
        return False

    # ---- show / hide -----------------------------------------------------
    def open_near(self, pet):
        # Decide initial view from saved state.
        last = self.app.state.get("last_session")
        self.show_all()                            # realize everything first...
        if self.view == "history" and self.session_path:
            self.show_history(self.session_path)   # ...then each view sets its own
        elif last and os.path.isfile(last):        #    search-bar visibility last
            self.show_history(last)
        else:
            self.show_list()
        self._place_near(pet)
        self.present()
        if self._refresh_id is None:
            self._refresh_id = GLib.timeout_add_seconds(REFRESH_SECS, self._refresh)

    def hide_panel(self):
        if self._refresh_id is not None:
            GLib.source_remove(self._refresh_id)
            self._refresh_id = None
        self.hide()

    def _place_near(self, pet):
        try:
            px, py = pet.get_position()
            pw, _ph = pet.get_size()
            geo = monitor_geometry(self)
            # Prefer left of the pet; if not enough room, go right.
            x = px - PANEL_W - 12
            if x < geo.x + 8:
                x = px + pw + 12
            x = max(geo.x + 8, min(x, geo.x + geo.width - PANEL_W - 8))
            y = max(geo.y + 8, min(py, geo.y + geo.height - PANEL_H - 8))
            self.move(int(x), int(y))
        except Exception:
            pass

    # ---- refresh tick ----------------------------------------------------
    def _refresh(self):
        if not self.get_visible():
            self._refresh_id = None
            return False
        if self.view == "history" and self.session_path:
            self.show_history(self.session_path, keep_scroll=True)
        else:
            # Only rebuild the results — leave the search box (and its focus) alone.
            self._populate_results(keep_scroll=True)
        return True

    def _clear(self, container):
        for child in container.get_children():
            container.remove(child)

    # ---- view: session list ---------------------------------------------
    def show_list(self, keep_scroll=False):
        self.view = "list"
        self._clear(self.header)
        title = Gtk.Label(xalign=0, label="Claude sessions")
        title.get_style_context().add_class("hdr-title")
        self.header.pack_start(title, True, True, 0)
        # "Today" filter toggle + "+ New" session button
        today = Gtk.Button(label="Today")
        today.get_style_context().add_class("todaybtn")
        if self._today_only:
            today.get_style_context().add_class("todaybtn-on")
        today.set_valign(Gtk.Align.CENTER)
        today.connect("clicked", lambda *_: self._toggle_today())
        self.header.pack_start(today, False, False, 0)
        newb = Gtk.Button(label="+ New")
        newb.get_style_context().add_class("newbtn")
        newb.set_valign(Gtk.Align.CENTER)
        newb.set_tooltip_text("Start a new Claude Code session in a WezTerm tab")
        newb.connect("clicked", self._new_session_menu)
        self.header.pack_start(newb, False, False, 0)
        self._add_close()
        self.header.show_all()
        self.searchbar.show_all()                  # search bar only in list view
        self._populate_results(keep_scroll=keep_scroll)

    def _toggle_today(self):
        self._today_only = not self._today_only
        self.show_list()                           # rebuild header (button state) + results

    def _on_search(self, entry):
        self._search_query = entry.get_text().strip().lower()
        if self.view == "list":
            self._populate_results()

    def _pinned(self):
        p = self.app.state.get("pinned")
        return set(p) if isinstance(p, list) else set()

    def _toggle_pin(self, path):
        pins = self._pinned()
        pins.discard(path) if path in pins else pins.add(path)
        self.app.state["pinned"] = sorted(pins)
        save_state(self.app.state)
        self._populate_results(keep_scroll=True)

    def _populate_results(self, keep_scroll=False):
        """(Re)build the grouped session list (Active on top, then Recent)."""
        adj = self.scroll.get_vadjustment().get_value() if keep_scroll else 0
        self._clear(self.body)

        q = self._search_query
        pool = sessions.find_sessions(limit=None if q else 40)
        results = [s for s in pool if self._matches(s, q)] if q else pool
        if self._today_only:
            results = [s for s in results if sessions.is_today(s)]

        pins = self._pinned()
        max_tok = max((s["total_tokens"] for s in results), default=0)

        if not results:
            msg = ("No sessions match." if q else
                   "No sessions today." if self._today_only else
                   "No Claude Code sessions found yet.")
            self._empty(msg)
        else:
            pinned = [s for s in results if s["path"] in pins]
            active = [s for s in results if s["path"] not in pins and s.get("open")]
            # working first, then waiting, then other open
            active.sort(key=lambda s: (0 if s["working"] else 1 if s["waiting"] else 2))
            rest = [s for s in results if s["path"] not in pins and not s.get("open")]

            def section(label, items):
                if not items:
                    return
                hdr = Gtk.Label(xalign=0, label=label)
                hdr.get_style_context().add_class("section")
                self.body.pack_start(hdr, False, False, 0)
                lb = Gtk.ListBox()
                lb.set_selection_mode(Gtk.SelectionMode.NONE)
                lb.connect("row-activated", self._on_session_row)
                for s in items:
                    lb.add(self._session_row(s, max_tok, pins))
                self.body.pack_start(lb, False, False, 0)

            section("PINNED", pinned)
            section("ACTIVE", active)
            section("RECENT" if not q else "MATCHES", rest)

        self._set_list_footer(results)
        self.body.show_all()
        if keep_scroll:
            GLib.idle_add(lambda: self.scroll.get_vadjustment().set_value(adj))

    def _set_list_footer(self, results):
        if self._today_only:
            t = sessions.today_totals(results)
            self._set_footer_hint("today: {} prompts · {} tok · {}".format(
                t["prompts"], sessions.fmt_tokens(t["tokens"]), sessions.fmt_cost(t["cost"])))
        else:
            self._set_footer_hint("{} sessions · ● working  ● waiting · ⮒ open · ★ pin"
                                  .format(len(results)))

    @staticmethod
    def _matches(s, q):
        hay = " ".join([
            s.get("title") or "", s.get("project") or "",
            s.get("branch") or "", s.get("cwd") or "",
        ]).lower()
        return q in hay

    def _session_row(self, s, max_tok, pins):
        row = Gtk.ListBoxRow()
        row._session_path = s["path"]
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        for fn in (box.set_margin_top, box.set_margin_bottom,
                   box.set_margin_start, box.set_margin_end):
            fn(6)

        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        # status dot
        dot = Gtk.Label(label="●")
        if s.get("working"):
            dot.get_style_context().add_class("dot-work")
            dot.set_tooltip_text("Claude is working")
        elif s.get("waiting"):
            dot.get_style_context().add_class("dot-wait")
            dot.set_tooltip_text("open · waiting for you")
        else:
            dot.get_style_context().add_class("dot-off")
        top.pack_start(dot, False, False, 0)

        name = Gtk.Label(xalign=0, label=s["title"])
        name.get_style_context().add_class("sess-name")
        name.set_ellipsize(Pango.EllipsizeMode.END)
        top.pack_start(name, True, True, 0)

        # pin toggle
        pinb = Gtk.Button(label="★" if s["path"] in pins else "☆")
        pinb.get_style_context().add_class("pinbtn")
        if s["path"] in pins:
            pinb.get_style_context().add_class("pinned")
        pinb.set_valign(Gtk.Align.CENTER)
        pinb.set_tooltip_text("Pin / unpin")
        pinb.connect("clicked", lambda *_a, p=s["path"]: self._toggle_pin(p))
        top.pack_end(pinb, False, False, 0)

        # open / focus button
        openb = Gtk.Button(label="⮒ focus" if s.get("open") else "⮒ open")
        openb.get_style_context().add_class("openbtn")
        openb.set_tooltip_text("Focus the existing WezTerm tab" if s.get("open")
                               else "Open in a new WezTerm tab and resume this session")
        openb.set_valign(Gtk.Align.CENTER)
        openb.connect("clicked", lambda *_a, sess=s: self._open_session(sess))
        top.pack_end(openb, False, False, 0)
        box.pack_start(top, False, False, 0)

        # meta line: project ⎇ branch ............ cost · prompts
        chip = Gtk.Label(label="{}{}".format(
            s["project"], "  ⎇ " + s["branch"] if s.get("branch") else ""))
        chip.get_style_context().add_class("sess-meta")
        chip.set_xalign(0)
        chip.set_ellipsize(Pango.EllipsizeMode.END)
        meta = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        meta.pack_start(chip, True, True, 0)
        cost = Gtk.Label(label=sessions.fmt_cost(s.get("cost")))
        cost.get_style_context().add_class("cost")
        cost.set_tooltip_text("estimated cost ({})".format(s.get("model_family", "?")))
        meta.pack_end(cost, False, False, 0)
        count = Gtk.Label(label="{} · {} tok".format(
            s["total_prompts"], sessions.fmt_tokens(s["total_tokens"])))
        count.get_style_context().add_class("sess-meta")
        meta.pack_end(count, False, False, 0)
        box.pack_start(meta, False, False, 0)

        row.add(box)
        return row

    def _on_session_row(self, _lb, row):
        path = getattr(row, "_session_path", None)
        if path:
            self.app.clear_attention(path)         # opening it clears its alert
            self.show_history(path)
            self.app.state["last_session"] = path
            save_state(self.app.state)

    def _open_session(self, s):
        self.app.clear_attention(s["path"])
        ok = open_in_wezterm(s)
        verb = "focused" if s.get("open") else "opened"
        if ok:
            self._set_footer_hint("{} “{}” in WezTerm…".format(verb, s["project"]))
        else:
            self._set_footer_hint("couldn't reach WezTerm — is it running?")

    def _new_session_menu(self, btn):
        """Popup of recent project folders to start a fresh `claude` session in."""
        menu = Gtk.Menu()
        seen, count = set(), 0
        for s in self.app._snapshot or sessions.find_sessions(limit=25):
            cwd = s.get("cwd")
            if not cwd or cwd in seen:
                continue
            seen.add(cwd)
            count += 1
            item = Gtk.MenuItem(label="{}  ({})".format(s["project"], cwd))
            item.connect("activate", lambda _i, d=cwd: self._start_new(d))
            menu.append(item)
            if count >= 10:
                break
        sep = Gtk.SeparatorMenuItem()
        menu.append(sep)
        other = Gtk.MenuItem(label="Choose folder…")
        other.connect("activate", lambda *_: self._choose_folder())
        menu.append(other)
        menu.show_all()
        menu.popup_at_widget(btn, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)

    def _start_new(self, cwd):
        ok = new_session_in_wezterm(cwd)
        self._set_footer_hint("started new session in {}".format(os.path.basename(cwd))
                              if ok else "couldn't reach WezTerm")

    def _choose_folder(self):
        dlg = Gtk.FileChooserDialog(title="New session in folder…", parent=self,
                                    action=Gtk.FileChooserAction.SELECT_FOLDER)
        dlg.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Open", Gtk.ResponseType.OK)
        if dlg.run() == Gtk.ResponseType.OK:
            self._start_new(dlg.get_filename())
        dlg.destroy()

    def _toggle_prompt(self, index):
        if index in self._expanded:
            self._expanded.discard(index)
        else:
            self._expanded.add(index)
        if self.session_path:
            self.show_history(self.session_path, keep_scroll=True)
        return True

    # ---- view: prompt history -------------------------------------------
    def show_history(self, path, keep_scroll=False):
        self.view = "history"
        self.session_path = path
        if self._expanded_for != path:            # reset expansions on session change
            self._expanded = set()
            self._expanded_for = path
        self.searchbar.hide()                      # search bar is list-view only
        adj = self.scroll.get_vadjustment().get_value() if keep_scroll else 0

        s = sessions.parse_session(path)
        self._clear(self.header)
        back = Gtk.Button(label="‹ Sessions")
        back.get_style_context().add_class("navbtn")
        back.connect("clicked", lambda *_: self.show_list())
        self.header.pack_start(back, False, False, 0)

        if s:
            t = Gtk.Label(xalign=0, label=s["title"])
            t.get_style_context().add_class("hdr-title")
            t.set_ellipsize(Pango.EllipsizeMode.END)
            self.header.pack_start(t, True, True, 0)
        self._add_close()

        self._clear(self.body)
        if not s or not s["prompts"]:
            self._empty("No prompts in this session yet.")
            self._clear(self.footer)
        else:
            max_out = max((p["out_tokens"] for p in s["prompts"]), default=0)
            for p in s["prompts"]:                 # show ALL prompts (scrollable)
                self.body.pack_start(self._prompt_row(p, max_out), False, False, 0)
            self._set_footer_summary(s)

        self.header.show_all()        # header is rebuilt each call -> must re-show
        self.body.show_all()
        if keep_scroll:
            GLib.idle_add(lambda: self.scroll.get_vadjustment().set_value(adj))

    def _prompt_row(self, p, max_out):
        full = p["full_text"] or p["title"]
        expandable = ("\n" in full) or (len(full) > 90)
        expanded = expandable and (p["index"] in self._expanded)

        ev = Gtk.EventBox()
        ev.get_style_context().add_class("prow")
        if expandable:
            ev.set_tooltip_text("click to {}".format("collapse" if expanded else "expand"))
            ev.add_events(Gdk.EventMask.BUTTON_PRESS_MASK)
            ev.connect("button-press-event",
                       lambda *_a, i=p["index"]: self._toggle_prompt(i))
        box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        box.set_margin_start(4)
        box.set_margin_end(6)
        ev.add(box)

        # caret (only when there's more to see)
        caret = Gtk.Label(label=("▾" if expanded else "▸") if expandable else " ")
        caret.get_style_context().add_class("idx")
        caret.set_valign(Gtk.Align.START)
        caret.set_margin_top(2)
        caret.set_width_chars(1)
        box.pack_start(caret, False, False, 0)

        idx = Gtk.Label(label="{:>2}".format(p["index"]))
        idx.get_style_context().add_class("idx")
        idx.set_valign(Gtk.Align.START)
        idx.set_margin_top(2)
        box.pack_start(idx, False, False, 0)

        live = p["running"]
        # Collapsed: 2-line preview. Expanded: the complete prompt text inline.
        title = Gtk.Label(xalign=0, label=(full if expanded else p["title"]))
        title.get_style_context().add_class("ptitle-live" if live else "ptitle")
        title.set_line_wrap(True)
        title.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
        if expanded:
            title.set_lines(-1)
            title.set_ellipsize(Pango.EllipsizeMode.NONE)
            title.set_selectable(True)             # let you select/copy the text
        else:
            title.set_lines(2)
            title.set_ellipsize(Pango.EllipsizeMode.END)
        title.set_width_chars(24)                  # min; grows with hexpand
        title.set_valign(Gtk.Align.START)
        title.set_hexpand(True)
        box.pack_start(title, True, True, 0)

        # Fixed right-hand metric column (never clips).
        right = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        right.set_halign(Gtk.Align.END)
        right.set_valign(Gtk.Align.START)
        right.set_size_request(96, -1)
        if live:
            r = Gtk.Label(label="running…")
            r.get_style_context().add_class("running")
            r.set_halign(Gtk.Align.END)
            right.pack_start(r, False, False, 0)
        else:
            cls = heat_class(p["out_tokens"], max_out)
            tok = Gtk.Label(label=sessions.fmt_tokens(p["out_tokens"]) + " tok")
            tok.get_style_context().add_class(cls)
            tok.set_xalign(1.0)
            right.pack_start(tok, False, False, 0)

            el = Gtk.Label(label=sessions.fmt_elapsed(p["elapsed"]))
            el.get_style_context().add_class("elapsed")
            el.set_xalign(1.0)
            right.pack_start(el, False, False, 0)
        box.pack_start(right, False, False, 0)
        return ev

    # ---- footer builders -------------------------------------------------
    def _set_footer_hint(self, text):
        self._clear(self.footer)
        lbl = Gtk.Label(xalign=0, label=text)
        lbl.get_style_context().add_class("foot-lbl")
        self.footer.pack_start(lbl, True, True, 0)
        self.footer.show_all()

    def _set_footer_summary(self, s):
        self._clear(self.footer)
        box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        box.get_style_context().add_class("foot-box")
        box.set_hexpand(True)
        cells = [
            (str(s["total_prompts"]), "prompts"),
            (sessions.fmt_tokens(s["total_tokens"]), "output tokens"),
            (sessions.fmt_elapsed(s["wall_seconds"]), "wall-clock"),
        ]
        for i, (num, lbl) in enumerate(cells):
            cell = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
            n = Gtk.Label(label=num, xalign=0)
            n.get_style_context().add_class("foot-num")
            l = Gtk.Label(label=lbl, xalign=0)
            l.get_style_context().add_class("foot-lbl")
            cell.pack_start(n, False, False, 0)
            cell.pack_start(l, False, False, 0)
            box.pack_start(cell, True, True, 0)
        self.footer.pack_start(box, True, True, 0)
        self.footer.show_all()

    # ---- shared bits -----------------------------------------------------
    def _add_close(self):
        btn = Gtk.Button(label="✕")
        btn.get_style_context().add_class("closebtn")
        btn.connect("clicked", lambda *_: self.hide_panel())
        self.header.pack_end(btn, False, False, 0)

    def _empty(self, msg):
        lbl = Gtk.Label(xalign=0, label=msg)
        lbl.get_style_context().add_class("empty")
        lbl.set_margin_top(12)
        self.body.pack_start(lbl, False, False, 0)


# ---------------------------------------------------------------------------
# Application glue
# ---------------------------------------------------------------------------
class App:
    def __init__(self):
        self.state = load_state()
        self.attention = set()      # session paths whose Claude just finished & waits
        self._att_working = {}      # path -> last-seen 'working' bool (transition detect)
        self._snapshot = []         # most recent find_sessions() result (shared)
        self._install_css()
        self.pet = Pet(self)
        self.panel = Panel(self)
        self.pet.connect("destroy", Gtk.main_quit)
        self.pet.show_all()
        GLib.timeout_add_seconds(ATTENTION_POLL_S, self._poll_attention)

    def _install_css(self):
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def any_live(self):
        return any(s.get("is_live") for s in self._snapshot)

    def _poll_attention(self):
        """Detect open sessions that just transitioned working -> waiting and
        flag them for attention (pet bounces + red ! badge)."""
        try:
            snap = sessions.find_sessions(limit=12)
        except Exception:
            return True
        self._snapshot = snap
        now = time.time()
        new_attention = False
        for s in snap:
            path = s["path"]
            if not s.get("open"):
                self._att_working.pop(path, None)
                self.attention.discard(path)
                continue
            was = self._att_working.get(path)
            now_working = s.get("working", False)
            recent = (s.get("last_prompt_ts") or 0) >= now - ATTENTION_WINDOW_S \
                or (now - s.get("mtime", 0)) <= ATTENTION_WINDOW_S
            # working -> not-working (and waiting) = Claude just finished for you
            if was and not now_working and s.get("waiting") and recent:
                if path not in self.attention:
                    self.attention.add(path)
                    new_attention = True
            if now_working:
                self.attention.discard(path)   # back to work; clear stale flag
            self._att_working[path] = now_working
        if new_attention:
            self.pet.bounce()
        self.pet.area.queue_draw()
        return True

    def clear_attention(self, path):
        self.attention.discard(path)
        self.pet.area.queue_draw()

    def toggle_panel(self, pet):
        if self.panel.get_visible():
            self.panel.hide_panel()
        else:
            self.panel.open_near(pet)


def main():
    if not (os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")):
        sys.stderr.write("claude-pet: no display found (need X11/Wayland).\n")
        return 1
    App()
    Gtk.main()
    return 0


if __name__ == "__main__":
    sys.exit(main())
