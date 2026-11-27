#!/usr/bin/env bash
# Limitlane installer for macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/reginaldosilva27/limitlane/main/install.sh | bash
#
# Options (when piping, pass them after `bash -s --`):
#   --uninstall   remove the app, its login item and the Claude Code hook
#   --dir DIR     install into DIR (default: /Applications, or ~/Applications
#                 when /Applications is not writable)
#   --dmg FILE    install from a local disk image instead of downloading
#   --no-launch   do not open the app afterwards
#
# The app is downloaded from the latest GitHub release. Files fetched with curl
# carry no quarantine flag, so macOS opens the app without a Gatekeeper prompt.

set -euo pipefail

REPO="reginaldosilva27/limitlane"
DMG_URL="https://github.com/$REPO/releases/latest/download/Limitlane-macos-universal.dmg"
APP="Limitlane.app"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }

dest_dir=""
dmg=""
launch=1
uninstall=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) dest_dir="${2:?--dir needs a folder}"; shift 2 ;;
    --dmg) dmg="${2:?--dmg needs a file}"; shift 2 ;;
    --no-launch) launch=0; shift ;;
    --uninstall) uninstall=1; shift ;;
    -h|--help) sed -n '2,15p' "$0" 2>/dev/null || true; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || die "this installer is for macOS. On Windows, run in PowerShell:
  irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex"

major="$(sw_vers -productVersion | cut -d. -f1)"
[ "$major" -ge 11 ] || die "Limitlane needs macOS 11 or later (this Mac runs $(sw_vers -productVersion))."

quit_running_app() {
  if pgrep -xq limitlane; then
    say "Quitting the running Limitlane"
    pkill -x limitlane || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -xq limitlane || break; sleep 0.3; done
  fi
}

# ---------- uninstall ----------
if [ "$uninstall" = 1 ]; then
  found=0
  # With --dir, touch only that folder.
  if [ -n "$dest_dir" ]; then dirs=("$dest_dir"); else dirs=(/Applications "$HOME/Applications"); fi
  login_item="$HOME/Library/LaunchAgents/Limitlane.plist"
  for dir in "${dirs[@]}"; do
    app="$dir/$APP"
    [ -d "$app" ] || continue
    found=1
    say "Removing the Claude Code statusline hook (your previous statusline is restored)"
    "$app/Contents/MacOS/limitlane" hook uninstall || true
    quit_running_app
    # The login item, only when it opens this copy of the app.
    if [ -f "$login_item" ] && grep -qF "$app/" "$login_item"; then rm -f "$login_item"; fi
    say "Deleting $app"
    rm -rf "$app"
  done
  [ "$found" = 1 ] || say "Limitlane.app was not found; nothing to remove."
  say "Done. Settings and limit history stay in ~/.limitlane; delete that folder to remove them too."
  exit 0
fi

# ---------- install ----------
if [ -z "$dest_dir" ]; then
  if [ -w /Applications ]; then dest_dir=/Applications; else dest_dir="$HOME/Applications"; fi
fi
mkdir -p "$dest_dir"

tmp="$(mktemp -d)"
mnt="$tmp/mnt"
cleanup() {
  [ -d "$mnt" ] && hdiutil detach -quiet "$mnt" 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

if [ -z "$dmg" ]; then
  say "Downloading the latest Limitlane release"
  dmg="$tmp/Limitlane.dmg"
  curl -fL --progress-bar -o "$dmg" "$DMG_URL" || die "download failed ($DMG_URL).
If no release has been published yet, build from source instead:
  https://github.com/$REPO#build-from-source"
fi
[ -f "$dmg" ] || die "disk image not found: $dmg"

mkdir -p "$mnt"
hdiutil attach -nobrowse -readonly -noautoopen -quiet -mountpoint "$mnt" "$dmg" || die "could not open the disk image"
[ -d "$mnt/$APP" ] || die "the disk image does not contain $APP"

quit_running_app
say "Installing to $dest_dir/$APP"
rm -rf "${dest_dir:?}/$APP"
ditto "$mnt/$APP" "$dest_dir/$APP"
/usr/bin/xattr -dr com.apple.quarantine "$dest_dir/$APP" 2>/dev/null || true

exe="$dest_dir/$APP/Contents/MacOS/limitlane"
"$exe" --version >/dev/null || die "the installed app does not run on this Mac"

if [ "$launch" = 1 ]; then
  say "Opening Limitlane"
  open "$dest_dir/$APP"
fi

cat <<EOF

Limitlane $("$exe" --version | awk '{print $2}') is installed.

Next:
  1. Click the Limitlane item in the menu bar and choose "Enable live limits".
     (Or from a terminal: "$exe" hook install)
  2. Send any message in Claude Code. The percentages appear within seconds.
  3. Optional: Settings > Open at login.

Terminal commands:  alias limitlane="$exe"   then   limitlane status
Uninstall:          curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash -s -- --uninstall
EOF
