#!/usr/bin/env bash
# Read-only snapshot of the R1. It shows what's flashed, which kernel, whether there's root, and
# whether the hardware r1os needs (wheel, side button, camera motor) and the kernel features the
# agent box needs are visible.
# Writes nothing to the device. Never prints serial numbers or IMEIs.
#
# Usage: tools/device-report.sh     (R1 booted with USB debugging, or in fastboot/fastbootd)
# Output: stdout, plus out/device-report-<time>.txt
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
need fastboot
mkdir -p "$OUT"
REPORT="$OUT/device-report-$(date +%Y%m%d-%H%M%S).txt"

section() { printf '\n## %s\n' "$*"; }
row() { printf '%-34s %s\n' "$1" "$2"; }

report_adb() {
  echo "# R1 device report ($(date -u +%Y-%m-%dT%H:%M:%SZ), adb)"

  section "Build"
  for p in ro.product.model ro.product.device ro.product.vendor.device ro.build.version.release \
           ro.build.type ro.build.display.id ro.system.build.fingerprint ro.vendor.build.fingerprint \
           ro.build.version.security_patch ro.vendor.build.security_patch ro.lineage.version \
           ro.treble.enabled ro.product.cpu.abi; do
    row "$p" "$(prop "$p")"
  done

  section "Boot and slots"
  for p in ro.boot.slot_suffix ro.boot.verifiedbootstate ro.boot.flash.locked \
           ro.boot.vbmeta.device_state ro.boot.veritymode ro.debuggable ro.secure; do
    row "$p" "$(prop "$p")"
  done

  section "Kernel (Rabbit's builds say so in the version string)"
  adb_sh cat /proc/version

  section "Root"
  if detect_root; then row root "$ROOT_MODE"; else row root "none (no adb root, no su)"; fi

  section "USB (CUA mode should be off on non-stock systems)"
  for p in ro.vendor.cua.enable sys.usb.config sys.usb.state persist.sys.usb.config; do
    row "$p" "$(prop "$p")"
  done

  section "Input devices (wheel = och1970_holl_key, side button = mtk-kpd)"
  adb_sh cat /proc/bus/input/devices | grep -E '^(N|H):' || true

  section "Key layouts in use"
  adb_sh dumpsys input 2>/dev/null | grep -E 'Name: "(mtk-kpd|och1970_holl_key)"|KeyLayoutFile' | head -20 || true

  section "Camera motor"
  adb_sh 'ls -l /sys/devices/platform/step_motor_ms35774/ 2>&1 | head -20'
  row orientation "$(adb_sh 'cat /sys/devices/platform/step_motor_ms35774/orientation 2>&1')"

  section "Cameras"
  adb_sh dumpsys media.camera 2>/dev/null | grep -E '^== Camera device|Facing|Orientation' | head -10 || true

  section "Kernel features for the agent box"
  row "namespaces (/proc/self/ns)" "$(adb_sh ls /proc/self/ns | tr '\n' ' ')"
  row "user namespaces" "$(adb_sh 'test -e /proc/self/ns/user && echo yes || echo no')"
  row "max_user_namespaces" "$(adb_sh 'cat /proc/sys/user/max_user_namespaces 2>/dev/null || echo n/a')"
  row "seccomp / no_new_privs" "$(adb_sh 'grep -E "^(Seccomp|NoNewPrivs)" /proc/self/status' | tr '\n' ' ')"
  row "filesystems" "$(adb_sh cat /proc/filesystems | grep -E 'overlay|tmpfs|ext4|f2fs|cgroup' | awk '{print $NF}' | tr '\n' ' ')"
  row "cgroup controllers" "$(adb_sh 'cat /proc/cgroups 2>/dev/null' | awk 'NR>1 && $4==1 {print $1}' | tr '\n' ' ')"

  section "Memory and storage"
  # One quoted string: adb joins arguments and the R1's shell would read a bare | as a pipe.
  adb_sh "grep -E 'MemTotal|MemAvailable|SwapTotal|SwapFree' /proc/meminfo"
  adb_sh 'df -h /data 2>/dev/null | tail -1'

  section "Display"
  adb_sh wm size
  adb_sh wm density

  section "Partitions"
  if [ -n "${ROOT_MODE:-}" ]; then
    root_sh 'ls /dev/block/by-name' | tr -d '\r' | tr '\n' ' '
    echo
  else
    echo "(needs root)"
  fi

  section "Home app"
  adb_sh cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME 2>/dev/null | tail -1 || true
  row "third-party packages" "$(adb_sh pm list packages -3 2>/dev/null | wc -l)"
}

report_fastboot() {
  echo "# R1 device report ($(date -u +%Y-%m-%dT%H:%M:%SZ), fastboot)"
  section "Bootloader"
  for v in product unlocked secure current-slot slot-count version-bootloader version-baseband \
           is-userspace super-partition-name; do
    row "$v" "$(fb_var "$v")"
  done
  row "unlock ability" "$(fastboot flashing get_unlock_ability 2>&1 | grep -io 'ability.*' | head -1)"
  if in_fastbootd; then
    section "Logical partitions (fastbootd)"
    fastboot getvar all 2>&1 | grep -E 'is-logical:.*yes' | sed 's/(bootloader) //' || true
  fi
}

{
  if adb_ready; then
    report_adb
  elif fastboot devices 2>/dev/null | grep -q .; then
    report_fastboot
  else
    echo "No R1 found. Boot it with USB debugging on (or put it in fastboot) and plug it in." >&2
    echo "  adb devices      -> $(adb devices | sed 1d | grep -c . || true) device(s)" >&2
    exit 1
  fi
} 2>&1 | tee "$REPORT"

echo
say "Saved $REPORT"
