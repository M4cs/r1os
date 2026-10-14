#!/usr/bin/env bash
# Roots the R1 by patching its *current* boot image (Rabbit's kernel) with Magisk, from this PC.
# There's no tapping through the Magisk app on the 2.88" screen.
#   1. Installs the Magisk app.
#   2. Copies boot_<slot> off over `adb root`.
#   3. Patches it with Magisk's own boot_patch.sh, on the R1.
#   4. Keeps the unpatched image in out/ for unrooting, and flashes the patched one to the active slot.
# Needs a userdebug system (the LineageOS GSI is one) so `adb root` can read the boot partition.
# Adapted from r1ptt (MIT); see NOTICE.md.
#
# Usage: tools/root.sh [Magisk.apk]     (default: the newest firmware/Magisk-*.apk)
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
need fastboot
need unzip

APK=${1:-$(find "$FIRMWARE" -maxdepth 1 -name 'Magisk-*.apk' 2>/dev/null | sort | tail -1)}
[ -n "$APK" ] && [ -f "$APK" ] || die "No Magisk APK: download Magisk-v*.apk into firmware/ (github.com/topjohnwu/Magisk/releases)"

adb wait-for-device
adb root >/dev/null 2>&1 || true
adb wait-for-device
[ "$(adb_sh id -u)" = 0 ] || die "adb root isn't available on this build (enable Rooted debugging in Developer options)"
SLOT=$(prop ro.boot.slot_suffix)
[ -n "$SLOT" ] || die "Couldn't read the active slot"

confirm "Patch boot$SLOT with $(basename "$APK") and flash it (roots the R1)?"

say "Installing the Magisk app"
adb install -r "$APK" >/dev/null

say "Patching boot$SLOT with Magisk's own script, on the R1"
mkdir -p "$OUT"
T=$(mktemp -d "$OUT/magisk.XXXXXX")   # under out/, not /tmp: adb.exe must be able to see it
unzip -q -j "$APK" 'assets/*.sh' 'assets/stub.apk' 'lib/arm64-v8a/*' -d "$T"
for f in "$T"/lib*.so; do n=$(basename "$f" .so); mv "$f" "$T/${n#lib}"; done
adb shell 'rm -rf /data/local/tmp/magisk && mkdir -p /data/local/tmp/magisk'
adb push "$T/." /data/local/tmp/magisk/ >/dev/null
rm -rf "$T"
if ! LOG=$(adb shell "dd if=/dev/block/by-name/boot$SLOT of=/data/local/tmp/magisk/boot.img bs=1M 2>&1 &&
    cd /data/local/tmp/magisk && chmod 755 * && sh ./boot_patch.sh boot.img 2>&1 && test -s new-boot.img"); then
  printf '%s\n' "$LOG" | tail -15 >&2
  die "Magisk couldn't patch the boot image"
fi

mkdir -p "$OUT"
adb pull /data/local/tmp/magisk/boot.img "$OUT/boot$SLOT-unpatched.img" >/dev/null
adb pull /data/local/tmp/magisk/new-boot.img "$OUT/magisk_patched-boot.img" >/dev/null
adb shell 'rm -rf /data/local/tmp/magisk'
say "Saved $OUT/boot$SLOT-unpatched.img (flash it back with tools/flash-boot.sh to unroot)"

YES=1 "$ROOT/tools/flash-boot.sh" "$OUT/magisk_patched-boot.img"
