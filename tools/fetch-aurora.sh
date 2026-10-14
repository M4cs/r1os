#!/usr/bin/env bash
# Downloads Aurora Store for r1os's image into out/aurora/AuroraStore.apk (git-ignored), checked against a
# pinned SHA-256 and Aurora OSS's signing certificate. tools/build-image.sh's `layer` stage puts it in the
# image as a privileged app (ADR-0013, Q23; roadmap R4's Android apps).
#
# The `preload` build is Aurora's own flavour for system images: the same app, plus INSTALL_PACKAGES and
# DELETE_PACKAGES in its manifest, which a privileged app holds, so its installs need no confirmation.
# GPL-3.0-or-later (NOTICE.md). Aurora updates itself in place afterwards, as an update to a system app.
#
# Usage: tools/fetch-aurora.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need curl

VERSION=4.8.4
URL=https://gitlab.com/-/project/6922885/uploads/3376fef895820893ddeb7154b056a07e/AuroraStore-preload-$VERSION.apk
SHA256=d2c0cd2ecaf3211ac31328e20649c654666e391afc9335478d8360a633bb159b
# Aurora OSS's release certificate, as its README gives it ("Certificate Fingerprints").
CERT=4c626157ad02bda3401a7263555f68a79663fc3e13a4d4369a12570941aa280f

D="$ROOT/out/aurora"
APK="$D/AuroraStore.apk"
mkdir -p "$D"
sha_ok() { [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$SHA256" ]; }

if ! { [ -f "$APK" ] && sha_ok "$APK"; }; then
  say "Downloading Aurora Store $VERSION (preload)"
  curl -fL --retry 3 -o "$APK.part" "$URL" || die "download failed: $URL"
  sha_ok "$APK.part" || die "checksum mismatch: $URL"
  mv "$APK.part" "$APK"
fi

# The certificate, when Java and the SDK's apksigner are around (the hash above already pins the file). Its
# jar, not apksigner.bat: the .bat breaks on a space in the SDK's path ("C:/Users/Sam Example").
SDK=${ANDROID_HOME:-${LOCALAPPDATA:-$HOME}/Android/Sdk}
BT=$(ls -d "$SDK"/build-tools/*/ 2>/dev/null | sort -V | tail -1)
if command -v java >/dev/null && [ -f "${BT}lib/apksigner.jar" ]; then
  certs=$(java -jar "${BT}lib/apksigner.jar" verify --print-certs "$APK" 2>/dev/null) || die "Aurora's APK doesn't verify"
  grep -q "SHA-256 digest: $CERT" <<<"$certs" || die "Aurora's signature doesn't match its published certificate"
  say "Aurora's signature matches its published certificate"
else
  warn "java or apksigner not found: certificate not checked (the pinned SHA-256 matched)"
fi
echo "$VERSION-preload" >"$D/VERSION"
say "Aurora Store $VERSION is in out/aurora"
