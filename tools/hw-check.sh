#!/usr/bin/env bash
# Hardware check for the parts r1os builds on.
#   default:  shows raw wheel and side-button events for $SECS seconds (read-only). The side button
#             is the power key until r1os remaps it, so a press also turns the screen off and on.
#   --motor:  rotates the camera through 0°, 90° and 180°, then back to where it was (asks first;
#             needs root). The motor is an open-loop stepper (drivers/misc/step_motor_ms35774.c):
#             don't hold the lens while it moves.
#
# Usage: tools/hw-check.sh [--motor]     (R1 booted with USB debugging)
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
adb_ready || die "No R1 over adb."
detect_root || true

MOTOR=/sys/devices/platform/step_motor_ms35774/orientation

if [ "${1:-}" = --motor ]; then
  [ -n "${ROOT_MODE:-}" ] || die "--motor needs root (adb root, or Magisk su)."
  cur=$(root_sh "cat $MOTOR" | tr -d '\r')
  case "$cur" in ''|*[!0-9]*) die "Couldn't read $MOTOR (got '$cur')." ;; esac
  confirm "Rotate the camera through 0, 90, 180 and back to $cur?"
  for a in 0 90 180 "$cur"; do
    say "orientation -> $a"
    root_sh "echo $a > $MOTOR"
    sleep 2
    say "reads $(root_sh "cat $MOTOR" | tr -d '\r')"
  done
  exit 0
fi

# Event nodes for the wheel and the button, from the kernel's input device list.
node_for() {
  adb_sh cat /proc/bus/input/devices | awk -v want="\"$1\"" '
    /^N: Name=/ { name = substr($0, index($0, "=") + 1) }
    /^H: Handlers=/ {
      # The first handler is glued to the key: "H: Handlers=event0" or "H: Handlers=sysrq kbd event0".
      sub(/^H: Handlers=/, "")
      if (name == want) for (i = 1; i <= NF; i++) if ($i ~ /^event[0-9]+$/) { print "/dev/input/" $i; exit }
    }'
}
# Both mtk-kpd and mtk-pmic-keys advertise KEY_POWER; which one the side button drives depends on
# the Rabbit kernel build (r1ptt saw mtk-kpd), so listen on both.
WHEEL=$(node_for och1970_holl_key)
KPD=$(node_for mtk-kpd)
PMIC=$(node_for mtk-pmic-keys)
say "wheel (och1970_holl_key): ${WHEEL:-not found}"
say "keys  (mtk-kpd):          ${KPD:-not found}"
say "keys  (mtk-pmic-keys):    ${PMIC:-not found}"
[ -n "$WHEEL$KPD$PMIC" ] || die "No input devices found. Is this Rabbit's kernel? (tools/device-report.sh)"

PY=$(command -v python3 || command -v python || true)
[ -n "$PY" ] || die "python not found"

# Raw struct input_event records straight from the nodes, captured on the R1 for $SECS seconds.
# getevent is no good here: it takes a single device, and it block-buffers when piped (r1ptt).
SECS=${SECS:-15}
T=/data/local/tmp
CMD="rm -f $T/r1os-ev-*.bin; timeout $SECS cat ${WHEEL:-/dev/null} > $T/r1os-ev-wheel.bin & timeout $SECS cat ${KPD:-/dev/null} > $T/r1os-ev-kpd.bin & timeout $SECS cat ${PMIC:-/dev/null} > $T/r1os-ev-pmic.bin & wait"
say "Spin the wheel both ways and press the side button now (${SECS} s)…"
if [ -n "${ROOT_MODE:-}" ]; then root_sh "$CMD" || true; else adb_sh "$CMD" || true; fi

mkdir -p "$OUT"
for k in wheel kpd pmic; do
  if [ -n "${ROOT_MODE:-}" ]; then root_exec_out "cat $T/r1os-ev-$k.bin"; else adb exec-out "cat $T/r1os-ev-$k.bin"; fi > "$OUT/ev-$k.bin"
done
if [ -n "${ROOT_MODE:-}" ]; then root_sh "rm -f $T/r1os-ev-*.bin"; else adb_sh "rm -f $T/r1os-ev-*.bin"; fi

# arm64 struct input_event: timeval (2 × int64), type u16, code u16, value s32 = 24 bytes.
"$PY" - "$OUT/ev-wheel.bin" "$OUT/ev-kpd.bin" "$OUT/ev-pmic.bin" <<'PY'
import struct, sys
names = {103: "KEY_UP", 108: "KEY_DOWN", 116: "KEY_POWER"}
states = {0: "up", 1: "down", 2: "repeat"}
for label, path in zip(("wheel (och1970_holl_key)", "keys (mtk-kpd)", "keys (mtk-pmic-keys)"), sys.argv[1:]):
    data = open(path, "rb").read()
    keys = []
    for i in range(0, len(data) - 23, 24):
        sec, usec, typ, code, value = struct.unpack("<qqHHi", data[i:i + 24])
        if typ == 1:  # EV_KEY
            keys.append((sec + usec / 1e6, names.get(code, f"code {code}"), states.get(value, value)))
    print(f"{label}: {len(keys)} key events")
    t0 = keys[0][0] if keys else 0
    for t, name, state in keys[:20]:
        print(f"  +{t - t0:6.2f}s  {name:<9} {state}")
    if len(keys) > 20:
        print(f"  … {len(keys) - 20} more")
PY
rm -f "$OUT/ev-wheel.bin" "$OUT/ev-kpd.bin" "$OUT/ev-pmic.bin"
say "Expected: KEY_UP / KEY_DOWN from the wheel, KEY_POWER from the side button (on mtk-kpd or mtk-pmic-keys)."
