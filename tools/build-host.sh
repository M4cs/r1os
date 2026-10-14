#!/usr/bin/env bash
# Builds the host app (host/, Kotlin + Gradle) into out/r1os-host.apk.
# Toolchain: Gradle 9.4.1, AGP 9.2.1 (built-in Kotlin), JDK 17+.
#
# Usage: tools/build-host.sh [debug]      (default: release)
#
# Release by default (T-100): Android won't compile a debuggable app ahead of time, so a debug host runs
# interpreted and JIT-compiled. Same key either way, so one updates the other in place.
#
# Signed with r1os's platform key (T-133, ADR-0062): $KEYS/platform.{pk8,x509.pem}, made once by
# tools/make-keys.sh. The image is signed with the same keys, so the host gets its signature
# permissions and tools/deploy.sh --host still updates the image's copy in place.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
export ANDROID_HOME=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$LOCALAPPDATA/Android/Sdk}}
[ -d "$ANDROID_HOME" ] || die "No Android SDK at $ANDROID_HOME (set ANDROID_HOME)."
[ -f "$KEYS/platform.pk8" ] || die "No platform key in $KEYS: run tools/make-keys.sh once (or set R1OS_KEYS)."
APKSIGNER=$(ls -d "$ANDROID_HOME"/build-tools/*/lib/apksigner.jar 2>/dev/null | sort -V | tail -1)
[ -n "$APKSIGNER" ] || die "No apksigner in $ANDROID_HOME/build-tools."
VARIANT=${1:-release}
TASK=assemble$(tr '[:lower:]' '[:upper:]' <<<"${VARIANT:0:1}")${VARIANT:1}
cd "$ROOT/host"
./gradlew --quiet "$TASK"
mkdir -p "$OUT"
# Gradle signs with its debug key; apksigner replaces that with the platform key (v2 + v3 schemes).
java -jar "$APKSIGNER" sign --key "$KEYS/platform.pk8" --cert "$KEYS/platform.x509.pem" \
  --out "$OUT/r1os-host.apk" "app/build/outputs/apk/$VARIANT/app-$VARIANT.apk"
rm -f "$OUT/r1os-host.apk.idsig"
java -jar "$APKSIGNER" verify --print-certs "$OUT/r1os-host.apk" | grep -q "CN=r1os platform" ||
  die "out/r1os-host.apk isn't signed with r1os's platform key."
say "Built $OUT/r1os-host.apk ($(( $(stat -c %s "$OUT/r1os-host.apk") / 1024 )) KiB, signed with r1os's platform key)"
