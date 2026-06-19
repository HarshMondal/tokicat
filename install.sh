#!/usr/bin/env bash
# Installer for the Claude Code desktop pet.
# - Verifies dependencies (GTK3 / PyGObject) and offers to apt-install if missing.
# - Installs an XDG autostart entry so the pet launches on login.
# All code stays in THIS repo; nothing is copied elsewhere except the autostart
# .desktop file (which just points back here).

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PET="$REPO_DIR/pet.py"
AUTOSTART_DIR="$HOME/.config/autostart"
DESKTOP_FILE="$AUTOSTART_DIR/claude-pet.desktop"

echo "Claude Code pet installer"
echo "  repo: $REPO_DIR"
echo

# ---- 1. dependency check ---------------------------------------------------
need_apt=0
if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 not found."; exit 1
fi
if ! python3 -c "import gi; gi.require_version('Gtk','3.0'); from gi.repository import Gtk" >/dev/null 2>&1; then
    echo "Missing GTK3 / PyGObject Python bindings."
    need_apt=1
fi

if [ "$need_apt" -eq 1 ]; then
    echo "The pet needs these system packages (no pip):"
    echo "    python3-gi  gir1.2-gtk-3.0  python3-gi-cairo"
    if command -v apt >/dev/null 2>&1; then
        read -r -p "Install them now with apt? [Y/n] " ans
        ans="${ans:-Y}"
        if [[ "$ans" =~ ^[Yy] ]]; then
            sudo apt update
            sudo apt install -y python3-gi gir1.2-gtk-3.0 python3-gi-cairo
        else
            echo "Skipped. Install them yourself, then re-run this script."
            exit 1
        fi
    else
        echo "Not a Debian/apt system. Install the equivalent of:"
        echo "  python3-gi gir1.2-gtk-3.0 python3-gi-cairo  for your distro, then re-run."
        exit 1
    fi
else
    echo "Dependencies OK (GTK3 / PyGObject present)."
fi

# ---- 2. quick smoke test of the data layer --------------------------------
echo
echo "Checking transcript access..."
if python3 "$REPO_DIR/sessions.py" >/dev/null 2>&1; then
    echo "  sessions.py runs OK (found ~/.claude/projects transcripts or handled absence)."
else
    echo "  WARNING: sessions.py errored. The pet still runs; check ~/.claude/projects."
fi

# ---- 3. autostart entry ----------------------------------------------------
echo
read -r -p "Add autostart entry so the pet launches on login? [Y/n] " ans
ans="${ans:-Y}"
if [[ "$ans" =~ ^[Yy] ]]; then
    mkdir -p "$AUTOSTART_DIR"
    cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Claude Code Pet
Comment=Ambient desktop pet showing your Claude Code session scoreboard
Exec=bash -c "sleep 5; exec python3 '$PET'"
Icon=utilities-terminal
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
    echo "  wrote $DESKTOP_FILE"
else
    echo "  Skipped autostart."
fi

# ---- 4. done ---------------------------------------------------------------
echo
echo "Done. Start the pet now with:"
echo "    python3 '$PET'      (or  $REPO_DIR/run.sh )"
echo
echo "Drop  assets/pet.gif  or  assets/pet.png  in the repo to use your own art;"
echo "until then a built-in placeholder mascot is shown."
