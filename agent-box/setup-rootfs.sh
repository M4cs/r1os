#!/system/bin/sh
# Builds (or updates) the r1os agent box on the R1. Run as root by tools/agent-box.sh install.
#
#   1. Extracts the Alpine minirootfs into /data/r1os/agent/rootfs (first run only).
#   2. Creates the agent user (uid/gid 2950, plus group inet 3003 for Android's paranoid networking).
#   3. apk installs what the CLIs need. Claude Code on Alpine needs libgcc, libstdc++ and ripgrep
#      (with USE_BUILTIN_RIPGREP=0, set by agent-exec). setpriv (util-linux; Alpine installs it as /bin/setpriv) drops privileges.
#   4. Installs Codex from the tarball pushed next to this script (codex.tar.gz), if present.
#   5. Installs Claude Code with its official installer, as the agent user, into ~/.local/bin.
#
# Usage: setup-rootfs.sh <alpine-minirootfs.tar.gz> [codex.tar.gz [codex-code-mode-host.tar.gz]]
set -e
BB=/data/adb/magisk/busybox
# r1os's own image has no Magisk: tools/agent-box.sh puts the same static BusyBox here instead.
[ -x "$BB" ] || BB=/data/r1os/bin/busybox
BOX=/data/r1os/agent
R=$BOX/rootfs
EXEC=/data/r1os/bin/agent-exec
ROOTFS_TGZ=$1
CODEX_TGZ=$2
CODEX_HOST_TGZ=${3:-}

umask 022
mkdir -p "$R" "$BOX/home" "$BOX/work"
chown 0:0 "$BOX" "$R"
chmod 755 "$BOX" "$R"

if [ ! -x "$R/bin/busybox" ]; then
  echo "== extracting $(basename "$ROOTFS_TGZ")"
  $BB tar -xzf "$ROOTFS_TGZ" -C "$R"
fi
mkdir -p "$R/home/agent" "$R/work"

echo "== agent user"
"$EXEC" --root -- /bin/sh -c '
  grep -q "^inet:" /etc/group || addgroup -g 3003 inet
  grep -q "^agent:" /etc/group || addgroup -g 2950 agent
  grep -q "^agent:" /etc/passwd || adduser -D -H -u 2950 -G agent -h /home/agent -s /bin/sh agent
  addgroup agent inet 2>/dev/null || true
'
chown 2950:2950 "$BOX/home" "$BOX/work"
chmod 700 "$BOX/home" "$BOX/work"

echo "== packages"
"$EXEC" --root -- /bin/sh -c '
  apk update -q
  # quickjs: r1d runs a trial of an app's logic with it before installing the app (ADR-0041).
  apk add -q --no-cache bash curl ca-certificates libgcc libstdc++ ripgrep git setpriv procps-ng quickjs nodejs npm
  sed -i "s#^agent:\(.*\):/bin/sh\$#agent:\1:/bin/bash#" /etc/passwd
'

if [ -n "$CODEX_TGZ" ] && [ -f "$CODEX_TGZ" ]; then
  echo "== codex"
  mkdir -p "$R/tmp/codex-install"
  $BB tar -xzf "$CODEX_TGZ" -C "$R/tmp/codex-install"
  install -m 0755 "$R/tmp/codex-install/codex-aarch64-unknown-linux-musl" "$R/usr/local/bin/codex"
  # Codex's code-mode host (its tools fail closed without it).
  if [ -n "$CODEX_HOST_TGZ" ] && [ -f "$CODEX_HOST_TGZ" ]; then
    $BB tar -xzf "$CODEX_HOST_TGZ" -C "$R/tmp/codex-install"
    install -m 0755 "$R/tmp/codex-install/codex-code-mode-host-aarch64-unknown-linux-musl" "$R/usr/local/bin/codex-code-mode-host"
  fi
  rm -rf "$R/tmp/codex-install"
fi

echo "== claude code (official installer, as agent)"
"$EXEC" -- /bin/bash -c 'if [ -x ~/.local/bin/claude ]; then claude update || true; else curl -fsSL https://claude.ai/install.sh | bash; fi'

echo "== versions"
"$EXEC" -- /bin/bash -c 'echo "claude: $(claude --version 2>&1 | head -1)"; echo "codex:  $(codex --version 2>&1 | head -1)"'
