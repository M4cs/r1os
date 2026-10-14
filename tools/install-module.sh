#!/usr/bin/env bash
# Zips a Magisk module directory from device/magisk/ and installs it on the R1 with Magisk.
# It takes effect after a reboot. To remove every module from a boot-looping R1 (there are no
# volume keys for Magisk's safe mode):
#   adb wait-for-device shell magisk --remove-modules
#
# Usage: tools/install-module.sh <module dir>     (rooted R1 booted with USB debugging)
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
PY=$(command -v python3 || command -v python || true)
[ -n "$PY" ] || die "python not found"

DIR=${1:?usage: $0 <module dir>}
[ -f "$DIR/module.prop" ] || die "$DIR/module.prop not found"
ID=$(sed -n 's/^id=//p' "$DIR/module.prop" | tr -d '\r')
[ -n "$ID" ] || die "no id= in $DIR/module.prop"

# Magisk's installer runs these with Android's sh: they must have LF line endings.
if grep -rlI $'\r' "$DIR" >/dev/null 2>&1; then
  die "CRLF line endings in $DIR ($(grep -rlI $'\r' "$DIR" | head -3 | tr '\n' ' ')). Fix them first."
fi

mkdir -p "$OUT"
ZIP="$OUT/$ID.zip"
"$PY" - "$DIR" "$ZIP" <<'PY'
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(src):
        for f in files:
            p = os.path.join(root, f)
            z.write(p, os.path.relpath(p, src).replace(os.sep, "/"))
PY
say "Built $ZIP"

adb_ready || die "No R1 over adb."
detect_root || die "Needs root (Magisk)."
confirm "Install Magisk module '$ID' on the R1?"

# A boot image patched from the command line (tools/root.sh) leaves /data/adb/magisk empty until the
# Magisk app's "additional setup" runs, and `magisk --install-module` refuses with "Incomplete Magisk
# install". Do the same file copy from the Magisk APK here (as r1ptt's provision.sh does).
if ! root_sh 'test -x /data/adb/magisk/busybox && test -f /data/adb/magisk/util_functions.sh && echo ok' | grep -q ok; then
  APK=$(find "$FIRMWARE" -maxdepth 1 -name 'Magisk-*.apk' 2>/dev/null | sort | tail -1)
  [ -n "$APK" ] || die "Magisk's setup is incomplete: open the Magisk app on the R1 once, or put Magisk-v*.apk in firmware/."
  say "Finishing Magisk's setup from $(basename "$APK")"
  need unzip
  T=$(mktemp -d "$OUT/magiskbin.XXXXXX")   # under out/, not /tmp: adb.exe must be able to see it
  unzip -q -j "$APK" 'assets/*.sh' 'assets/stub.apk' 'lib/arm64-v8a/*' -d "$T"
  for f in "$T"/lib*.so; do n=$(basename "$f" .so); mv "$f" "$T/${n#lib}"; done
  adb shell 'rm -rf /data/local/tmp/magiskbin && mkdir -p /data/local/tmp/magiskbin'
  adb push "$T/." /data/local/tmp/magiskbin/ >/dev/null
  rm -rf "$T"
  root_sh 'mkdir -p /data/adb/modules /data/adb/magisk && chmod 700 /data/adb && cp -af /data/local/tmp/magiskbin/. /data/adb/magisk/ && chown -R 0:0 /data/adb/magisk && chmod -R 755 /data/adb/magisk && rm -rf /data/local/tmp/magiskbin'
fi
adb push "$ZIP" "/data/local/tmp/$ID.zip" >/dev/null
root_sh "magisk --install-module /data/local/tmp/$ID.zip"
root_sh "rm -f /data/local/tmp/$ID.zip"
say "Installed $ID. Reboot to apply: adb reboot"
