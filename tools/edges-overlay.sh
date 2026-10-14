#!/usr/bin/env bash
# The framework overlay that turns off Android's swipe from an edge (T-127, ADR-0059), for an R1 whose image doesn't have
# it built in. Builds device/overlays/edges into an APK, signs it with the image's platform key (r1os's own, $KEYS:
# ADR-0062), installs it and enables it. WRITES TO THE DEVICE: asks first (YES=1 only after the user said so).
#   tools/edges-overlay.sh            build, install, enable
#   tools/edges-overlay.sh off        disable it again (Android's edge swipes come back)
set -euo pipefail
source "$(dirname "$0")/lib.sh"
SRC="$ROOT/device/overlays/edges"
OUT="$ROOT/out/edges"
BT=$(ls -d "$LOCALAPPDATA/Android/Sdk/build-tools/"* | tail -1)
PLATFORM=$(ls -d "$LOCALAPPDATA/Android/Sdk/platforms/android-"* | grep -E 'android-[0-9]+$' | sort -V | tail -1)
ID=dev.r1os.overlay.edges

if [ "${1:-}" = off ]; then
  adb_ready
  confirm "Disable $ID on the R1 (Android's edge swipes come back)?"
  adb shell cmd overlay disable "$ID"
  exit 0
fi

mkdir -p "$OUT"
"$BT/aapt2.exe" compile --dir "$SRC/res" -o "$OUT/res.zip"
"$BT/aapt2.exe" link -o "$OUT/unsigned.apk" -I "$PLATFORM/android.jar" --manifest "$SRC/AndroidManifest.xml" "$OUT/res.zip"
# The image's platform key: r1os's own (ADR-0062; the framework's certificate is checked against it below).
[ -f "$KEYS/platform.pk8" ] || die "No platform key in $KEYS: run tools/make-keys.sh once (or set R1OS_KEYS)."
java -jar "$BT/lib/apksigner.jar" sign --key "$KEYS/platform.pk8" --cert "$KEYS/platform.x509.pem" --out "$OUT/edges.apk" "$OUT/unsigned.apk"
say "Built $OUT/edges.apk"

adb_ready
want=$(java -jar "$BT/lib/apksigner.jar" verify --print-certs "$OUT/edges.apk" | grep -m1 'SHA-256' | sed 's/.*: //' | tr -d '\r')
adb pull /system/framework/framework-res.apk "$OUT/framework-res.apk" >/dev/null
have=$(java -jar "$BT/lib/apksigner.jar" verify --print-certs "$OUT/framework-res.apk" | grep -m1 'SHA-256' | sed 's/.*: //' | tr -d '\r')
[ "$want" = "$have" ] || die "the R1's framework isn't signed with the platform key in $KEYS: an overlay can't target it"
confirm "Install $ID on the R1 and enable it (Android's swipe from an edge goes; tools/edges-overlay.sh off undoes it)?"
adb install -r "$OUT/edges.apk"
adb shell cmd overlay enable "$ID"
adb shell cmd overlay lookup android android:dimen/system_gestures_start_threshold
