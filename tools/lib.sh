# Shared helpers for tools/*.sh. Source, don't run.
# Parts adapted from r1ptt (MIT, Copyright (c) 2026 Julian Dice); see NOTICE.md.

# Git Bash rewrites arguments that look like POSIX paths (`/proc/version` becomes
# `C:/Program Files/Git/proc/version`) before adb or fastboot ever see them. Turn that off for
# everything these scripts run. Harmless on Linux and macOS.
export MSYS_NO_PATHCONV=1
export MSYS2_ARG_CONV_EXCL='*'

# With conversion off, local paths handed to the native adb.exe / fastboot.exe must already be
# Windows paths. `pwd -W` (Git Bash only) gives the mixed form C:/Users/..., which both Git Bash
# tools and Windows programs accept; elsewhere it fails and plain `pwd` is used. Keep every local
# path under $ROOT (or relative), and don't use /tmp: adb.exe can't see it.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && { pwd -W 2>/dev/null || pwd; })"
OUT="$ROOT/out"
FIRMWARE="$ROOT/firmware"
BACKUPS="$ROOT/backups"
# r1os's own signing keys (T-133): the image's platform, shared, media… keys and the host's. Never in
# the repo: ~/.r1os/keys ($KEYS), made once by tools/make-keys.sh; R1OS_KEYS overrides it. Not
# %APPDATA%: shells the Claude desktop app starts write AppData into its own package folder (MSIX),
# which WSL's build can't see and an app reset deletes.
KEYS=${R1OS_KEYS:-$( { command -v cygpath >/dev/null && cygpath -m "$HOME"; } || echo "$HOME")/.r1os/keys}

# r1d listens on 127.0.0.1:7171 on the R1. The desktop simulator uses 7171 on the PC, so the
# device is forwarded to a different local port and both can run at once.
R1D_PORT=7171
R1D_LOCAL_PORT=${R1D_LOCAL_PORT:-7172}
# Forwards the PC's $R1D_LOCAL_PORT to r1d on the R1 and prints the base URL.
r1d_forward() {
  adb forward "tcp:$R1D_LOCAL_PORT" "tcp:$R1D_PORT" >/dev/null || die "adb forward tcp:$R1D_LOCAL_PORT failed (port in use? set R1D_LOCAL_PORT)"
  echo "http://127.0.0.1:$R1D_LOCAL_PORT"
}

# r1d's per-boot token (ADR-0026): root-only on the R1, so read as root (adbd root, else su). r1d makes
# a new one each start. Empty if r1d hasn't written it (or for the simulator, which is open).
r1d_token() {
  local t
  t=$(adb shell cat /data/r1os/run/token 2>/dev/null | tr -d '\r\n')
  [ -n "$t" ] || t=$(adb shell su 0 cat /data/r1os/run/token 2>/dev/null | tr -d '\r\n')
  printf '%s' "$t"
}

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mxx\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "$1 not found (install Android platform-tools / $1)"; }

# Ask before anything destructive. YES=1 skips the prompt; agents may set it only after the user approved
# that exact step in chat (CLAUDE.md).
confirm() {
  [ "${YES:-0}" = 1 ] && return 0
  local a
  read -r -p "$1 [y/N] " a || die "Cancelled (no answer)."
  [ "$a" = y ] || [ "$a" = Y ] || die "Cancelled."
}

# adb shell output arrives with CRLF on some builds.
adb_sh() { adb shell "$@" | tr -d '\r'; }
prop() { adb shell getprop "$1" | tr -d '\r'; }

# The R1 over USB, or over wireless debugging (Android 11+, Settings › Developer options):
# - With nothing attached, it connects to a paired device advertising `_adb-tls-connect` on the
#   LAN. Pair a PC once with `adb pair <ip>:<pairing port> <code>` (the device's "Pair device with
#   pairing code" screen); the connect port changes whenever wireless debugging restarts, which
#   mDNS discovery follows.
# - With several attached, $ANDROID_SERIAL picks one; otherwise USB wins over Wi-Fi.
adb_ready() {
  adb get-state 2>/dev/null | grep -q '^device$' && return 0
  if [ -z "${ANDROID_SERIAL:-}" ]; then
    local serials pick
    serials=$(adb devices 2>/dev/null | tr -d '\r' | awk 'NR > 1 && $2 == "device" { print $1 }')
    if [ "$(printf '%s\n' "$serials" | grep -c .)" -gt 1 ]; then
      pick=$(printf '%s\n' "$serials" | grep -v ':' | head -1)
      ANDROID_SERIAL=${pick:-$(printf '%s\n' "$serials" | head -1)}
      export ANDROID_SERIAL
      adb get-state 2>/dev/null | grep -q '^device$' && return 0
    fi
  fi
  adb_wireless
}

# Connects to a paired R1 found by wireless-debugging discovery. Returns 1 if there's none.
adb_wireless() {
  local addr
  for addr in $(adb mdns services 2>/dev/null | tr -d '\r' | awk '$2 ~ /_adb-tls-connect/ { print $3 }'); do
    adb connect "$addr" 2>/dev/null | grep -q 'connected to' || continue
    for _ in 1 2 3 4 5; do
      if adb -s "$addr" get-state 2>/dev/null | grep -q '^device$'; then
        export ANDROID_SERIAL=$addr
        printf '\033[1;32m==>\033[0m %s\n' "Connected over wireless debugging ($addr)" >&2
        return 0
      fi
      sleep 1
    done
  done
  return 1
}

# Sets ROOT_MODE to "adb" (adbd runs as root: a userdebug build after `adb root`) or "su" (Magisk),
# and returns 1 if neither works. Never runs `adb root` itself: that restarts adbd, which callers
# may not want mid-session.
detect_root() {
  ROOT_MODE=
  if [ "$(adb_sh id -u 2>/dev/null)" = 0 ]; then
    ROOT_MODE=adb
  elif [ "$(adb shell "su -c 'id -u'" 2>/dev/null | tr -d '\r')" = 0 ]; then
    ROOT_MODE=su
  fi
  [ -n "$ROOT_MODE" ]
}

# Runs a command string as root on the R1 (call detect_root first). No single quotes in $1.
root_sh() {
  case "$ROOT_MODE" in
    adb) adb shell "$1" ;;
    su)  adb shell "su -c '$1'" ;;
    *)   die "root_sh without root (call detect_root first)" ;;
  esac
}

# Like root_sh, but binary-safe stdout (adb exec-out has no pty and no line-ending translation).
root_exec_out() {
  case "$ROOT_MODE" in
    adb) adb exec-out "$1" ;;
    su)  adb exec-out "su -c '$1'" ;;
    *)   die "root_exec_out without root (call detect_root first)" ;;
  esac
}

# Waits until `fastboot devices` lists something (bootloader or fastbootd).
wait_fastboot() {
  for _ in $(seq 1 90); do
    fastboot devices 2>/dev/null | grep -q . && return 0
    sleep 1
  done
  die "No fastboot device after 90 s. On Windows, check Device Manager for a missing driver."
}

# True when connected to userspace fastbootd rather than the bootloader's fastboot.
in_fastbootd() { fastboot getvar is-userspace 2>&1 | grep -q 'is-userspace: yes'; }

fb_var() { fastboot getvar "$1" 2>&1 | sed -n "s/^$1: //p" | head -1 | tr -d '\r'; }

active_slot() {
  local s
  s=$(fb_var current-slot)
  case "$s" in a|b) echo "$s" ;; *) die "Couldn't read the active slot (got '$s')." ;; esac
}

# Finds the folder with boot.img inside an extracted stock firmware zip.
stock_dir() {
  local d
  d=$(find "$1" -name boot.img -not -path '*__MACOSX*' -print -quit 2>/dev/null)
  [ -n "$d" ] || die "No boot.img under $1 (extract rabbit_OS_v0.8.293.zip there first)."
  dirname "$d"
}

# Writes a copy of a vbmeta image with verification off and prints its path. Sets the AVB header's
# flags (big-endian u32 at byte 120) to 3, HASHTREE_DISABLED | VERIFICATION_DISABLED: the same edit
# `fastboot --disable-verity --disable-verification` makes, done by hand because some fastboot
# builds fail it with "Failed to find AVB_MAGIC at offset: 0".
vbmeta_disabled() {
  local src=$1 dst
  dst="$OUT/$(basename "$1" .img)-noverify.img"
  mkdir -p "$OUT"
  [ "$(head -c 4 "$src")" = AVB0 ] || die "$src doesn't start with an AVB header"
  cp "$src" "$dst"
  printf '\000\000\000\003' | dd of="$dst" bs=1 seek=120 count=4 conv=notrunc 2>/dev/null
  echo "$dst"
}

# Flashes vbmeta images with verification off on both slots: the R1 only boots a non-Rabbit
# system this way.
flash_vbmeta_disabled() {
  local img=$1 v s patched
  for v in vbmeta vbmeta_system vbmeta_vendor; do
    patched=$(vbmeta_disabled "$img/$v.img")
    for s in a b; do
      fastboot flash "${v}_$s" "$patched"
    done
  done
}

# Decompresses .gz / .xz images into out/ and prints the resulting path.
unpacked() {
  local f=$1 base
  base=$(basename "$f")
  mkdir -p "$OUT"
  case "$f" in
    *.gz) [ -f "$OUT/${base%.gz}" ] || gunzip -c "$f" > "$OUT/${base%.gz}"; echo "$OUT/${base%.gz}" ;;
    *.xz) [ -f "$OUT/${base%.xz}" ] || xz -dc "$f" > "$OUT/${base%.xz}"; echo "$OUT/${base%.xz}" ;;
    *)    echo "$f" ;;
  esac
}

# The serial of the R1 plugged in (fastboot first, else adb), for matching backups. Never printed.
device_serial() {
  local s
  s=$(fastboot devices 2>/dev/null | tr -d '\r' | awk 'NR == 1 { print $1 }' || true)
  [ -n "$s" ] || s=$(adb get-serialno 2>/dev/null | tr -d '\r' || true)
  [ "$s" = unknown ] && s=
  printf '%s' "$s"
}

# The folder chosen for backups in the companion (ADR-0069), as a path this shell can use, if one was: its path is in
# %APPDATA%\r1os\companion\backup-folder.txt.
chosen_backups() {
  local settings f p
  settings=${APPDATA:-$HOME/AppData/Roaming}
  command -v cygpath >/dev/null 2>&1 && settings=$(cygpath -u "$settings")
  f=$settings/r1os/companion/backup-folder.txt
  [ -f "$f" ] || return 0
  p=$(tr -d '\r\n' <"$f")
  [ -n "$p" ] || return 0
  if command -v cygpath >/dev/null 2>&1; then cygpath -u "$p"; else printf '%s' "$p"; fi
}

# Refuses to continue unless a backup of the R1 plugged in verifies: its proinfo starts with this R1's serial (where
# the R1 keeps it), and every file matches its SHA256SUMS. Looks in backups/ (tools/backup.sh), the folder chosen in
# the companion, and Documents/r1os backups (T-169, T-194; R1OS_BACKUPS names another in its place).
require_backup() {
  local serial d p chosen
  local roots=("$BACKUPS" "${R1OS_BACKUPS:-$HOME/Documents/r1os backups}")
  chosen=$(chosen_backups)
  [ -n "$chosen" ] && roots+=("$chosen")
  serial=$(device_serial)
  [ -n "$serial" ] || die "No R1 found to match a backup to: plug it in (fastboot or booted) first."
  while IFS= read -r d; do
    p=$d/proinfo.img
    [ -f "$p" ] || p=$d/proinfo.bin
    [ -f "$p" ] || continue
    # The serial is proinfo's leading run of letters and digits.
    [ "$(head -c 64 "$p" | LC_ALL=C tr -c '[:alnum:]' '\n' | head -1)" = "$serial" ] || continue
    say "Checking $d against its SHA256SUMS"
    if (cd "$d" && sha256sum --quiet -c SHA256SUMS >/dev/null 2>&1); then
      say "Backup of this R1 found: $d"
      return 0
    fi
    warn "$d doesn't match its SHA256SUMS: not counted"
  done < <(find "${roots[@]}" -maxdepth 3 -name SHA256SUMS -printf '%h\n' 2>/dev/null | sort -r)
  die "No verified backup of this R1. Run tools/backup.sh (rooted) or the companion's backup first (or set NO_BACKUP_OK=1 if you really mean it)."
}
