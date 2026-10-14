#!/usr/bin/env bash
# Builds r1os's own system image (ADR-0013, roadmap R4): AOSP with TrebleDroid's GSI patches and
# device tree, plus r1os's layer (device/aosp/vendor_r1os). The result is a system-only Treble
# image for Rabbit's kernel and vendor, flashed to system_<slot>.
#
#   AOSP_VERSION=15 (default)  android-15.0.0_r36 (BP1A, security patch 2025-05-05), tree /aosp15
#   AOSP_VERSION=14            android-14.0.0_r55 (AP2A, security patch 2024-08-05), tree /aosp
# Google's newer android-security-* tags (October 2026) sit on different quarterly releases than
# TrebleDroid's patches, so they'd mean re-porting those patches (research, 2026-10-09).
#
# Runs inside a Linux build environment: on Windows, a WSL distro (`r1os-build` here) whose disk lives
# on <your build disk> (the system drive is usually too small). Run it from Git Bash like this, $ROOT
# being this repo as WSL sees it (under /mnt/c):
#   wsl -d r1os-build -u root -- bash "$ROOT/tools/build-image.sh" [stage...]
# (For Android 14: wsl -d r1os-build -u root -- env AOSP_VERSION=14 bash ".../build-image.sh" ...)
# Stages, in order (default: all; each one can be rerun on its own):
#   init    repo init + r1os's local manifests
#   sync    repo sync (~60-120 GB, shallow)
#   patch   TrebleDroid's patches (once; a marker file remembers)
#   layer   r1os's AOSP patches (device/aosp/patches/<version>), vendor/r1os from this repo, the
#           host APK, and the generated treble products
#   policy  just the SELinux policy, with AOSP's neverallow checks and tests (minutes; not a default
#           stage: run it after changing device/aosp/vendor_r1os/sepolicy, before `build`)
#   build   lunch treble_arm64_bvN-<release>-userdebug && make systemimage
#   export  out/image/r1os-system-a<version>-<date>.img (+ .sha256, .txt) in this repo
# Recipe from ponces/treble_aosp (android-14.0 and android-15.0, archived 2026-04), without his
# own patches, projects or signing keys; r1os's own patches are few and small (ADR-0035). Don't edit
# this file while it runs: bash reads it as it goes. Writes nothing to the R1.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AOSP_VERSION=${AOSP_VERSION:-15}
case "$AOSP_VERSION" in
  14) TAG=android-14.0.0_r55 RELEASE=ap2a DEFAULT_TREE=/aosp   DEFAULT_PATCHES=/aosp-patches ;;
  15) TAG=android-15.0.0_r36 RELEASE=bp1a DEFAULT_TREE=/aosp15 DEFAULT_PATCHES=/aosp-patches-15 ;;
  *) echo "AOSP_VERSION must be 14 or 15" >&2; exit 1 ;;
esac
TREE=${AOSP_TREE:-$DEFAULT_TREE}
PATCHES=${TREBLE_PATCHES:-$DEFAULT_PATCHES}
# r1os's own signing keys (T-133, ADR-0062), made once by tools/make-keys.sh in ~/.r1os/keys ($KEYS). WSL's root
# has another ~, so they're looked for beside this repo (a clone in your home); R1OS_KEYS overrides that. Copied
# into the tree (vendor/r1os-keys), never into this repo.
KEYS=${R1OS_KEYS:-$(dirname "$REPO")/.r1os/keys}
TARGET=treble_arm64_bvN
# Jobs by memory: about 1.5-2 GB per job at the build's peak (metalava, R8, javac). A .wslconfig of
# 16 GB ran 8 jobs; 28 GB runs all 16 threads, with a 48 GB swap file for the peaks.
MEM_GB=$(awk '/MemTotal/ {printf "%d", $2/1048576}' /proc/meminfo)
if [ "$MEM_GB" -ge 24 ]; then DEFAULT_JOBS=$(nproc); else DEFAULT_JOBS=8; fi
JOBS=${JOBS:-$DEFAULT_JOBS}

say() { printf '\n==> %s\n' "$*"; }

stage_init() {
  mkdir -p "$TREE" && cd "$TREE"
  [ -d .repo ] || repo init -u https://android.googlesource.com/platform/manifest -b "$TAG" \
    --depth=1 --git-lfs --no-clone-bundle
  mkdir -p .repo/local_manifests
  rm -f .repo/local_manifests/*.xml
  cp "$REPO/device/aosp/manifest/r1os-$AOSP_VERSION.xml" .repo/local_manifests/r1os.xml
  cp "$REPO/device/aosp/manifest/remove-$AOSP_VERSION.xml" .repo/local_manifests/remove.xml
}

stage_sync() {
  cd "$TREE"
  repo sync -c -j"${SYNC_JOBS:-12}" --force-sync --no-clone-bundle --no-tags --optimized-fetch --fail-fast ||
    repo sync -c -j4 --force-sync --no-clone-bundle --no-tags --optimized-fetch
}

stage_patch() {
  cd "$TREE"
  [ -f .r1os-patched ] && { echo "already patched ($(cat .r1os-patched))"; return; }
  [ -d "$PATCHES" ] || git clone --depth=1 -b "android-$AOSP_VERSION.0" https://github.com/ponces/treble_aosp "$PATCHES"
  # TrebleDroid's set only. ponces' patch.sh: patches/<tree>/<project_with_underscores>/*.patch.
  bash "$PATCHES/patch.sh" "$PATCHES" trebledroid
  date -u +%FT%TZ > .r1os-patched
}

stage_layer() {
  cd "$TREE"
  # r1os's own patches to AOSP projects: device/aosp/patches/<AOSP version>/<project path>/*.patch
  # (ADR-0035). Each is applied once: one that already reverses cleanly is in.
  local p proj patches="$REPO/device/aosp/patches/$AOSP_VERSION"
  [ -d "$patches" ] || patches=/dev/null
  while IFS= read -r p; do
    proj=${p#"$patches/"}
    proj=${proj%/*}
    if git -C "$proj" apply --reverse --check "$p" 2>/dev/null; then
      echo "in $proj already: ${p##*/}"
    else
      git -C "$proj" apply "$p"
      echo "applied to $proj: ${p##*/}"
    fi
  done < <([ "$patches" = /dev/null ] || find "$patches" -name '*.patch' | sort)
  mkdir -p vendor/r1os/prebuilt
  rsync -a --delete --exclude prebuilt/ "$REPO/device/aosp/vendor_r1os/" vendor/r1os/
  # r1os.mk's PRODUCT_DEFAULT_DEV_CERTIFICATE points here: every key the build signs with.
  local k
  mkdir -p vendor/r1os-keys
  for k in releasekey platform shared media networkstack nfc bluetooth sdk_sandbox; do
    [ -f "$KEYS/$k.pk8" ] && [ -f "$KEYS/$k.x509.pem" ] ||
      { echo "$KEYS/$k.pk8 is missing: run tools/make-keys.sh once (Git Bash), or set R1OS_KEYS" >&2; exit 1; }
    install -m 600 "$KEYS/$k.pk8" vendor/r1os-keys/
    install -m 644 "$KEYS/$k.x509.pem" vendor/r1os-keys/
  done
  # SELinux's keys.conf (and mac_permissions.go) look for the default key by the name "testkey" in
  # the default certificate's folder: the same releasekey, under that name.
  install -m 600 "$KEYS/releasekey.pk8" vendor/r1os-keys/testkey.pk8
  install -m 644 "$KEYS/releasekey.x509.pem" vendor/r1os-keys/testkey.x509.pem
  [ -f "$REPO/out/r1os-host.apk" ] || { echo "out/r1os-host.apk is missing: run tools/build-host.sh first" >&2; exit 1; }
  cp "$REPO/out/r1os-host.apk" vendor/r1os/prebuilt/r1os-host.apk
  [ -f "$REPO/out/aurora/AuroraStore.apk" ] || { echo "out/aurora/AuroraStore.apk is missing: run tools/fetch-aurora.sh first" >&2; exit 1; }
  cp "$REPO/out/aurora/AuroraStore.apk" vendor/r1os/prebuilt/AuroraStore.apk
  # Verbose boot's program (T-101) and r1os's own boot animation (T-029), built on Windows like the host.
  [ -f "$REPO/out/bootlog/r1os-bootlog" ] || { echo "out/bootlog/r1os-bootlog is missing: run tools/build-bootlog.sh first" >&2; exit 1; }
  cp "$REPO/out/bootlog/r1os-bootlog" vendor/r1os/prebuilt/r1os-bootlog
  chmod 755 vendor/r1os/prebuilt/r1os-bootlog
  [ -f "$REPO/out/bootanim/bootanimation.zip" ] || { echo "out/bootanim/bootanimation.zip is missing: run tools/r1h bootanim default first" >&2; exit 1; }
  cp "$REPO/out/bootanim/bootanimation.zip" vendor/r1os/prebuilt/bootanimation.zip
  chmod 755 vendor/r1os/bin/r1os-r1d
  (cd device/phh/treble && bash generate.sh vendor/r1os/r1os.mk)
}

lunch_target() {
  cd "$TREE"
  # The 48 GB swap file doesn't survive a WSL restart, and 16 GB alone isn't enough for the build.
  if [ -f /swapfile ] && ! swapon --show | grep -q /swapfile; then swapon /swapfile; fi
  set +u
  # shellcheck disable=SC1091
  source build/envsetup.sh
  lunch "$TARGET-$RELEASE-userdebug"
  set -u
  # Work that doesn't change the image: ABI checks, and Soong's own self-tests at bootstrap.
  # (WITHOUT_CHECK_API=true would skip metalava's API checks too, but Android 15 refuses it: its
  # stubs come from the API signature files, which that flag can't keep in sync.)
  export SKIP_ABI_CHECKS=true
  echo "jobs: $JOBS (RAM ${MEM_GB} GB)"
}

stage_policy() {
  lunch_target
  m -j"$JOBS" --skip-soong-tests selinux_policy
}

stage_build() {
  lunch_target
  m -j"$JOBS" --skip-soong-tests systemimage
}

stage_export() {
  cd "$TREE"
  local img out stamp
  img=$(ls -t out/target/product/*/system.img | head -1)
  # Aurora must go in exactly as Aurora OSS signed it: a build that rewrites it (uncompressing its dex) breaks
  # its signature, and Android refuses it (v0.2.11's first build).
  local aurora
  aurora=$(ls out/target/product/*/system/priv-app/AuroraStore/AuroraStore.apk 2>/dev/null | head -1)
  if [ -n "$aurora" ] && ! cmp -s "$aurora" "$REPO/out/aurora/AuroraStore.apk"; then
    echo "the image's Aurora Store isn't the APK tools/fetch-aurora.sh checked: the build changed it" >&2
    exit 1
  fi
  # Signed with r1os's keys (T-133): the framework and the host carry r1os's platform certificate.
  local want got apk
  want=$(openssl x509 -in vendor/r1os-keys/platform.x509.pem -outform DER | sha256sum | cut -d' ' -f1)
  for apk in out/target/product/*/system/framework/framework-res.apk out/target/product/*/system/priv-app/R1osHost/R1osHost.apk; do
    got=$(out/host/linux-x86/bin/apksigner verify --print-certs "$apk" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p')
    [ "$got" = "$want" ] || { echo "$apk isn't signed with r1os's platform key ($got)" >&2; exit 1; }
  done
  stamp="a$AOSP_VERSION-$(date +%Y%m%d-%H%M)"
  out="$REPO/out/image"
  mkdir -p "$out"
  cp "$img" "$out/r1os-system-$stamp.img"
  (cd "$out" && sha256sum "r1os-system-$stamp.img" > "r1os-system-$stamp.img.sha256")
  {
    echo "built: $(date -u +%FT%TZ)"
    echo "aosp: $TAG, target $TARGET-$RELEASE-userdebug"
    echo "keys: r1os's own (platform $(openssl x509 -in vendor/r1os-keys/platform.x509.pem -noout -fingerprint -sha256 | cut -d= -f2))"
    echo "r1os image version: $(grep -o "ro.r1os.image.version=[^ ]*" "$REPO/device/aosp/vendor_r1os/r1os.mk" | cut -d= -f2)"
    echo "r1os repo: $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null)$(git -C "$REPO" diff --quiet 2>/dev/null || echo ' (with uncommitted changes)')"
    echo "trebledroid device tree: $(git -C device/phh/treble rev-parse --short HEAD)"
    echo "size: $(stat -c %s "$img") bytes"
  } > "$out/r1os-system-$stamp.txt"
  say "Exported $out/r1os-system-$stamp.img ($(du -h "$img" | cut -f1))"
}

stages=("$@")
[ ${#stages[@]} -gt 0 ] || stages=(init sync patch layer build export)
for s in "${stages[@]}"; do
  say "Stage: $s"
  "stage_$s"
done
