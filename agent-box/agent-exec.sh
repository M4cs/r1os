#!/system/bin/sh
# agent-exec: runs a command inside the r1os agent box (ADR-0003).
#
# The box is an Alpine arm64 rootfs at /data/r1os/agent/rootfs. Each run gets:
#   - a private mount namespace (unshare -m, then rprivate), so none of these mounts leak into
#     Android's namespace;
#   - fresh /proc, a 512 MB /tmp (tmpfs), and a minimal /dev: null, zero, random, urandom, tty,
#     and its own devpts instance. No block or input devices.
#   - /home/agent ← /data/r1os/agent/home (CLI credentials live here, owned by the CLIs) and
#     /work ← /data/r1os/agent/work (workspaces);
#   - /etc/resolv.conf from the DNS servers of Android's active network (fallback 1.1.1.1).
# Then, unless --root, it drops to the unprivileged `agent` user (uid 2950) with Android's `inet`
# group (3003: the kernel's paranoid networking refuses sockets to processes without it), no new
# privileges, and an empty capability bounding set.
#
# The box is the security boundary: Rabbit's kernel has no user namespaces, so the CLIs' own
# bubblewrap sandboxes can't run (docs/open-questions.md Q2). Still to come in r1d's supervisor:
# a seccomp filter and a memory cgroup.
#
# Usage (as root on the R1): agent-exec [--root] [--] command [args...]
set -e
BB=/data/adb/magisk/busybox
# r1os's own image has no Magisk: tools/agent-box.sh puts the same static BusyBox here instead.
[ -x "$BB" ] || BB=/data/r1os/bin/busybox
BOX=/data/r1os/agent
R=$BOX/rootfs
AS_ROOT=0
[ "$1" = --root ] && { AS_ROOT=1; shift; }
[ "$1" = -- ] && shift
[ $# -gt 0 ] || { echo "usage: agent-exec [--root] [--] command [args...]" >&2; exit 2; }
[ -x "$R/bin/busybox" ] || { echo "agent-exec: no agent box at $R (run tools/agent-box.sh install)" >&2; exit 1; }
[ "$(id -u)" = 0 ] || { echo "agent-exec: needs root" >&2; exit 1; }

DNS=$(dumpsys connectivity 2>/dev/null | grep -o 'DnsAddresses: \[[^]]*\]' | head -1 | grep -oE '([0-9]{1,3}\.){3}[0-9]{1,3}' | head -3 | tr '\n' ' ')
[ -n "$DNS" ] || DNS=1.1.1.1
export BB BOX R AS_ROOT DNS

exec $BB unshare -m -- $BB sh -c '
  set -e
  $BB mount --make-rprivate /
  $BB mount -t proc -o nosuid,nodev,noexec proc "$R/proc"
  $BB mount -t tmpfs -o nosuid,nodev,mode=1777,size=512m tmpfs "$R/tmp"
  $BB mount -t tmpfs -o nosuid,noexec,mode=755,size=1m tmpfs "$R/dev"
  for n in null zero random urandom tty; do
    : >"$R/dev/$n"
    $BB mount --bind "/dev/$n" "$R/dev/$n"
  done
  mkdir -p "$R/dev/pts" "$R/dev/shm"
  $BB mount -t devpts -o newinstance,ptmxmode=0666,mode=620,nosuid,noexec devpts "$R/dev/pts"
  ln -sf pts/ptmx "$R/dev/ptmx"
  $BB mount --bind "$BOX/home" "$R/home/agent"
  $BB mount --bind "$BOX/work" "$R/work"
  : >"$R/etc/resolv.conf"
  for d in $DNS; do echo "nameserver $d" >>"$R/etc/resolv.conf"; done

  PATHS=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  if [ "$AS_ROOT" = 1 ]; then
    exec $BB chroot "$R" /usr/bin/env -i HOME=/root PATH=$PATHS TERM="${TERM:-xterm}" LANG=C.UTF-8 "$@"
  fi
  exec $BB chroot "$R" /bin/setpriv --reuid=agent --regid=agent --init-groups --no-new-privs \
    --inh-caps=-all --bounding-set=-all -- \
    /usr/bin/env -i HOME=/home/agent USER=agent SHELL=/bin/bash PATH=/home/agent/.local/bin:$PATHS \
    TERM="${TERM:-xterm}" LANG=C.UTF-8 USE_BUILTIN_RIPGREP=0 "$@"
' agent-exec "$@"
