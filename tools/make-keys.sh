#!/usr/bin/env bash
# Makes r1os's own signing keys, once (T-133, ADR-0062): the keys the image's APKs are signed with
# instead of AOSP's public test-keys, and the platform key the host app is signed with.
#
# Writes $KEYS (~/.r1os/keys; R1OS_KEYS overrides it), never the repo:
#   releasekey, platform, shared, media, networkstack, nfc, bluetooth, sdk_sandbox
#   each as NAME.pk8 (the private key, PKCS#8 DER, unencrypted as AOSP's make_key leaves them
#   for an unattended build) and NAME.x509.pem (its certificate).
#
# Back the folder up somewhere safe and offline. Losing it means the next image re-signs every system
# app with new keys (Android keeps their data, as it did for this switch), and the host can no
# longer be updated in place until an image with the new key is flashed. Anyone holding it can sign
# an app with r1os's system powers. Refuses to overwrite keys that already exist.
#
# Usage: tools/make-keys.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need openssl

NAMES=(releasekey platform shared media networkstack nfc bluetooth sdk_sandbox)
for n in "${NAMES[@]}"; do
  [ ! -e "$KEYS/$n.pk8" ] || die "$KEYS/$n.pk8 already exists: r1os's keys are made once (see the header)."
done
mkdir -p "$KEYS"
umask 077
for n in "${NAMES[@]}"; do
  key=$(mktemp "$KEYS/.$n.XXXXXX")
  openssl genrsa -out "$key" 4096 2>/dev/null
  openssl req -new -x509 -sha256 -days 10000 -key "$key" -out "$KEYS/$n.x509.pem" \
    -subj "/C=US/O=r1os/OU=r1os/CN=r1os $n"
  openssl pkcs8 -in "$key" -topk8 -outform DER -nocrypt -out "$KEYS/$n.pk8"
  rm -f "$key"
  say "$n: $(openssl x509 -in "$KEYS/$n.x509.pem" -noout -fingerprint -sha256 | cut -d= -f2)"
done
say "Made r1os's keys in $KEYS. Back the folder up; never commit or share it."
