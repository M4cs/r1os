#!/usr/bin/env bash
# Copies the R1's irreplaceable partitions off the device, into backups/<time>-slot<x>/ with
# SHA256SUMS and the size of every partition checked against the device:
#   - device-unique: nvram, nvdata, nvcfg, protect1/2, proinfo, persist, seccfg, frp, otp, sec1,
#     md_udc, para (IMEI, radio calibration, the unlock flag);
#   - the current boot chain on both slots: preloader, lk, boot, init_boot, vendor_boot, dtbo,
#     vbmeta*, md1img, tee, gz, scp, sspm, spmfw, dpm;
#   - the eMMC boot areas (mmcblk0boot0/1, where the preloader lives).
# super and userdata are skipped (the OS and your data; downloadable or disposable).
# Only reads from the device. Needs root: adb root on a userdebug build, or Magisk su.
#
# Usage: tools/backup.sh      (R1 booted with USB debugging)
# The backup holds the IMEI: it stays in backups/ (git-ignored). Never commit or share it.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
need sha256sum

adb_ready || die "No R1 over adb. Boot it with USB debugging on and plug it in."
if ! detect_root; then
  die "Needs root. On a userdebug build run 'adb root' first; with Magisk, allow the Shell su prompt on the R1."
fi
say "Root via $ROOT_MODE"

SLOT=$(prop ro.boot.slot_suffix | tr -d _)
DEST="$BACKUPS/$(date +%Y%m%d-%H%M%S)-slot${SLOT:-unknown}"
mkdir -p "$DEST"

# Matched against /dev/block/by-name. The R1 (k65v1_64_bsp) also has preloader_a/b, init_boot_a/b
# and vendor_boot_a/b (seen 2026-10-08), plus small MediaTek partitions (otp, sec1, para, md_udc).
# Skipped on purpose: super, userdata, metadata (tied to userdata), expdb and flashinfo (logs).
want() {
  case "$1" in
    nvram|nvdata|nvcfg|protect1|protect2|proinfo|persist|seccfg|frp|otp|sec1|md_udc) return 0 ;;
    para|boot_para|logo) return 0 ;;
    boot|dtbo|vbmeta|vbmeta_system|vbmeta_vendor|lk|md1img|tee|gz|scp|sspm|spmfw|dpm|preloader) return 0 ;;
    boot_[ab]|init_boot_[ab]|vendor_boot_[ab]|dtbo_[ab]|preloader_[ab]|lk_[ab]|md1img_[ab]) return 0 ;;
    vbmeta_[ab]|vbmeta_system_[ab]|vbmeta_vendor_[ab]) return 0 ;;
    tee_[ab]|gz_[ab]|scp_[ab]|sspm_[ab]|spmfw_[ab]|dpm_[ab]) return 0 ;;
    *) return 1 ;;
  esac
}

dump() {  # <device path> <output file>
  local dev=$1 out=$2 want_size got_size
  want_size=$(root_sh "blockdev --getsize64 $dev" | tr -d '\r')
  root_exec_out "dd if=$dev bs=1M 2>/dev/null" > "$out"
  got_size=$(stat -c %s "$out")
  if [ "$got_size" != "$want_size" ]; then
    warn "$(basename "$out"): got $got_size bytes, device says $want_size"
    return 1
  fi
}

{
  echo "date:        $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "slot:        ${SLOT:-unknown}"
  echo "system:      $(prop ro.system.build.fingerprint)"
  echo "vendor:      $(prop ro.vendor.build.fingerprint)"
  echo "kernel:      $(adb_sh cat /proc/version)"
} > "$DEST/info.txt"
root_sh 'ls -l /dev/block/by-name' | tr -d '\r' > "$DEST/by-name.txt"

FAILED=0
for n in $(root_sh 'ls /dev/block/by-name' | tr -d '\r'); do
  want "$n" || continue
  printf '  %-18s' "$n"
  if dump "/dev/block/by-name/$n" "$DEST/$n.img"; then
    echo "$(( $(stat -c %s "$DEST/$n.img") / 1024 )) KiB"
  else
    FAILED=1
  fi
done
for hw in mmcblk0boot0 mmcblk0boot1; do
  if root_sh "test -b /dev/block/$hw && echo yes" | grep -q yes; then
    printf '  %-18s' "$hw"
    if dump "/dev/block/$hw" "$DEST/$hw.bin"; then
      echo "$(( $(stat -c %s "$DEST/$hw.bin") / 1024 )) KiB"
    else
      FAILED=1
    fi
  fi
done

for n in nvram nvdata proinfo seccfg; do
  [ -s "$DEST/$n.img" ] || { warn "missing $n.img: this backup is incomplete"; FAILED=1; }
done

# Only a complete backup gets SHA256SUMS: tools/lib.sh's require_backup trusts that file.
SUMS=SHA256SUMS
[ "$FAILED" = 0 ] || SUMS=SHA256SUMS.partial
(cd "$DEST" && shopt -s nullglob && sha256sum -- *.img *.bin > "$SUMS")
[ "$FAILED" = 0 ] || die "Some partitions failed; see warnings. Partial backup left in $DEST ($SUMS)."
say "Backup complete: $DEST ($(du -sh "$DEST" | cut -f1))"
say "It holds the IMEI. Keep a second copy off this PC (USB stick), never in git or the cloud."
