#!/usr/bin/env bash
# The r1os agent box (ADR-0003): an Alpine chroot on the R1 with the claude and codex CLIs.
#
#   tools/agent-box.sh install       download Alpine + Codex (checksums verified), build the box on
#                                    the R1, install Claude Code with its own installer. Re-run to update.
#   tools/agent-box.sh codex-host    add Codex's code-mode host (its tools need it) to an existing box
#   tools/agent-box.sh run -- CMD    run CMD in the box as the agent user (non-interactive)
#   tools/agent-box.sh root -- CMD   run CMD in the box as root (packages)
#   tools/agent-box.sh shell         interactive bash in the box (needs a real terminal)
#   tools/agent-box.sh status        versions, disk use, memory
#
# Writes to the device (install): ask the user first (CLAUDE.md). Needs root over adb.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
need curl
need sha256sum

ALPINE_VER=3.24.2
ALPINE_TGZ=alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz
ALPINE_URL=https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VER%.*}/releases/aarch64/$ALPINE_TGZ
ALPINE_SHA256=9bf70a7f18ea44094cbb5f70c58f9af129c8214745743db0e68e5502cc2ce773
CODEX_ASSET=codex-aarch64-unknown-linux-musl.tar.gz
# Codex 0.16x runs its tools through a companion binary; without it, MCP tools "fail closed".
CODE_MODE_ASSET=codex-code-mode-host-aarch64-unknown-linux-musl.tar.gz
CACHE="$FIRMWARE/agent-box"
DEV=/data/r1os/agent

adb_ready || die "No R1 over adb."
detect_root || die "Needs root over adb (Rooted debugging + 'adb root')."

fetch() { # <url> <file> <sha256>
  local url=$1 file=$2 want=$3
  if [ ! -f "$file" ] || [ "$(sha256sum "$file" | cut -d' ' -f1)" != "$want" ]; then
    say "Downloading $(basename "$file")"
    curl -fL --retry 3 -o "$file.part" "$url"
    mv "$file.part" "$file"
  fi
  [ "$(sha256sum "$file" | cut -d' ' -f1)" = "$want" ] || die "$(basename "$file"): checksum mismatch"
}

cmd=${1:-status}
shift || true
[ "${1:-}" = -- ] && shift

case "$cmd" in
  install)
    confirm "Build/update the agent box on the R1 (downloads Alpine, Codex and Claude Code)?"
    mkdir -p "$CACHE"
    fetch "$ALPINE_URL" "$CACHE/$ALPINE_TGZ" "$ALPINE_SHA256"
    # Codex: the latest release's static musl build, checked against GitHub's published digest.
    need gh
    CODEX_TAG=$(gh api repos/openai/codex/releases/latest --jq .tag_name)
    CODEX_SHA=$(gh api repos/openai/codex/releases/latest --jq ".assets[] | select(.name==\"$CODEX_ASSET\") | .digest" | sed 's/^sha256://')
    [ -n "$CODEX_SHA" ] || die "No $CODEX_ASSET in Codex $CODEX_TAG"
    fetch "https://github.com/openai/codex/releases/download/$CODEX_TAG/$CODEX_ASSET" "$CACHE/codex-$CODEX_TAG.tar.gz" "$CODEX_SHA"
    HOST_SHA=$(gh api "repos/openai/codex/releases/tags/$CODEX_TAG" --jq ".assets[] | select(.name==\"$CODE_MODE_ASSET\") | .digest" | sed 's/^sha256://')
    [ -n "$HOST_SHA" ] && fetch "https://github.com/openai/codex/releases/download/$CODEX_TAG/$CODE_MODE_ASSET" "$CACHE/codex-code-mode-host-$CODEX_TAG.tar.gz" "$HOST_SHA"

    say "Pushing to the R1"
    adb push "$CACHE/$ALPINE_TGZ" /data/local/tmp/alpine.tar.gz >/dev/null
    adb push "$CACHE/codex-$CODEX_TAG.tar.gz" /data/local/tmp/codex.tar.gz >/dev/null
    [ -f "$CACHE/codex-code-mode-host-$CODEX_TAG.tar.gz" ] && adb push "$CACHE/codex-code-mode-host-$CODEX_TAG.tar.gz" /data/local/tmp/codex-host.tar.gz >/dev/null
    adb push "$ROOT/agent-box/agent-exec.sh" /data/local/tmp/agent-exec >/dev/null
    adb push "$ROOT/agent-box/setup-rootfs.sh" /data/local/tmp/setup-rootfs.sh >/dev/null
    root_sh 'mkdir -p /data/r1os/bin && mv -f /data/local/tmp/agent-exec /data/r1os/bin/agent-exec && chmod 700 /data/r1os/bin/agent-exec'
    # The box scripts need a static BusyBox (unshare, mount, chroot, tar -z). The LineageOS dev base has
    # Magisk's; r1os's own image (device/aosp) has no Magisk, so ship the same static build from the
    # Magisk APK in firmware/ to /data/r1os/bin/busybox.
    if ! root_sh '[ -x /data/adb/magisk/busybox ] && echo yes' | grep -q yes; then
      APK=$(ls -t "$ROOT"/firmware/Magisk-*.apk 2>/dev/null | head -1)
      [ -n "$APK" ] || die "No Magisk BusyBox on the R1 and no firmware/Magisk-*.apk to take one from."
      unzip -p "$APK" lib/arm64-v8a/libbusybox.so > "$CACHE/busybox-arm64"
      adb push "$CACHE/busybox-arm64" /data/local/tmp/busybox >/dev/null
      root_sh 'mv -f /data/local/tmp/busybox /data/r1os/bin/busybox && chmod 700 /data/r1os/bin/busybox && /data/r1os/bin/busybox | head -1'
    fi
    say "Building the box (Alpine $ALPINE_VER, Codex $CODEX_TAG, Claude Code latest)"
    # Then its SELinux labels (ADR-0035): the rootfs is r1os_agent_root_file, the rest r1os_agent_file.
    root_sh 'sh /data/local/tmp/setup-rootfs.sh /data/local/tmp/alpine.tar.gz /data/local/tmp/codex.tar.gz /data/local/tmp/codex-host.tar.gz; rc=$?; rm -f /data/local/tmp/alpine.tar.gz /data/local/tmp/codex.tar.gz /data/local/tmp/codex-host.tar.gz /data/local/tmp/setup-rootfs.sh; restorecon -RF /data/r1os/bin /data/r1os/agent; exit $rc' | tr -d '\r' \
      || die "The box's build stopped partway (above). The box as it was is still there."
    ;;
  codex-host)
    # Adds Codex's code-mode host to an existing box, matching the box's Codex version.
    need gh
    VER=$(root_sh "/data/r1os/bin/agent-exec -- codex --version" | tr -d '\r' | awk '{print $2}' | tail -1)
    [ -n "$VER" ] || die "Can't read the box's Codex version"
    TAG="rust-v$VER"
    SHA=$(gh api "repos/openai/codex/releases/tags/$TAG" --jq ".assets[] | select(.name==\"$CODE_MODE_ASSET\") | .digest" | sed 's/^sha256://')
    [ -n "$SHA" ] || die "No $CODE_MODE_ASSET in Codex $TAG"
    mkdir -p "$CACHE"
    fetch "https://github.com/openai/codex/releases/download/$TAG/$CODE_MODE_ASSET" "$CACHE/codex-code-mode-host-$TAG.tar.gz" "$SHA"
    adb push "$CACHE/codex-code-mode-host-$TAG.tar.gz" /data/local/tmp/codex-host.tar.gz >/dev/null
    root_sh "R=$DEV/rootfs; BB=/data/adb/magisk/busybox; [ -x \$BB ] || BB=/data/r1os/bin/busybox; mkdir -p \$R/tmp/host && \$BB tar -xzf /data/local/tmp/codex-host.tar.gz -C \$R/tmp/host && install -m 0755 \$R/tmp/host/codex-code-mode-host-aarch64-unknown-linux-musl \$R/usr/local/bin/codex-code-mode-host && rm -rf \$R/tmp/host /data/local/tmp/codex-host.tar.gz && restorecon -RF \$R/usr/local/bin && ls -la \$R/usr/local/bin" | tr -d '\r'
    say "Installed codex-code-mode-host ($TAG)"
    ;;
  run) root_sh "/data/r1os/bin/agent-exec -- $*" | tr -d '\r' ;;
  root) root_sh "/data/r1os/bin/agent-exec --root -- $*" | tr -d '\r' ;;
  shell)
    [ -t 0 ] || die "shell needs an interactive terminal (run it yourself in a terminal tab)."
    adb shell -t "/data/r1os/bin/agent-exec -- /bin/bash -l"
    ;;
  status)
    root_sh "/data/r1os/bin/agent-exec -- /bin/bash -c 'echo \"claude: \$(claude --version 2>&1 | head -1)\"; echo \"codex:  \$(codex --version 2>&1 | head -1)\"; echo \"alpine: \$(cat /etc/alpine-release)\"; echo \"id:     \$(id)\"'" | tr -d '\r'
    root_sh "du -sh $DEV/rootfs $DEV/home 2>/dev/null; grep -E 'MemAvailable' /proc/meminfo" | tr -d '\r'
    ;;
  *) die "unknown command '$cmd' (install | run | root | shell | status)" ;;
esac
