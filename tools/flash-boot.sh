#!/usr/bin/env bash
# Writes a boot image to the active slot only. tools/root.sh uses it for the Magisk-patched image;
# flash out/boot_<slot>-unpatched.img the same way to unroot. Only ever flash an image made from
# this R1's own boot partition (Rabbit's kernel: ADR-0001). Adapted from r1ptt (MIT); see NOTICE.md.
#
# Two routes:
#   - R1 booted with root over adb (adb root or Magisk su): writes boot_<slot> from Android with dd
#     (what Magisk's "Direct install" does), reads it back to verify the checksum, then reboots.
#     There's no fastboot, so no Windows driver is needed.
#   - Otherwise: adb reboot bootloader (or already in fastboot), then fastboot flash boot_<slot>.
#     On Windows the R1's bootloader shares USB ID 0E8D:201C with Android's adb, so every Android
#     boot rebinds it to the adb driver. Re-pick "Android Bootloader Interface" in Device Manager
#     when fastboot doesn't see the device (docs/runbooks/fresh-start.md §0).
#
# Usage: tools/flash-boot.sh <boot image>     (R1 booted with USB debugging, or in fastboot)
set -euo pipefail
source "$(dirname "$0")/lib.sh"

[ $# -ge 1 ] && [ -f "$1" ] || die "usage: $0 <boot image>"
need fastboot
need adb
need sha256sum
IMG=$1
SIZE=$(stat -c %s "$IMG")

confirm "Write $(basename "$IMG") to the active slot's boot partition?"

if adb_ready && detect_root; then
  SLOT=$(prop ro.boot.slot_suffix)
  case "$SLOT" in _a|_b) ;; *) die "Couldn't read the active slot (got '$SLOT')." ;; esac
  DEV=/dev/block/by-name/boot$SLOT
  PART=$(root_sh "blockdev --getsize64 $DEV" | tr -d '\r')
  [ "$SIZE" -le "$PART" ] || die "$(basename "$IMG") is $SIZE bytes; boot$SLOT is only $PART."
  WANT=$(sha256sum "$IMG" | cut -d' ' -f1)

  say "Writing boot$SLOT from Android ($ROOT_MODE root, $SIZE bytes)"
  adb push "$IMG" /data/local/tmp/r1os-boot.img >/dev/null
  # Rabbit's September 2026 build marks the boot partitions read-only (BLKROSET) at boot; the
  # 2024 one didn't. It's the kernel's software flag, reset every boot, so clear it for the write.
  root_sh "blockdev --setrw \$(readlink -f $DEV)"
  root_sh "dd if=/data/local/tmp/r1os-boot.img of=$DEV bs=1M && sync" || die "Writing boot$SLOT failed (see dd's error above). Nothing was rebooted."
  root_sh "rm -f /data/local/tmp/r1os-boot.img"
  GOT=$(root_sh "head -c $SIZE $DEV | sha256sum" | tr -d '\r' | cut -d' ' -f1)
  [ "$GOT" = "$WANT" ] || die "boot$SLOT readback doesn't match ($GOT vs $WANT). Don't reboot; rerun this script."
  say "boot$SLOT verified (sha256 ${WANT:0:12}…). Rebooting."
  adb reboot
  exit 0
fi

if adb_ready; then
  say "No root over adb; rebooting to the bootloader to use fastboot"
  adb reboot bootloader
fi
wait_fastboot
if in_fastbootd; then
  fastboot reboot bootloader
  sleep 5
  wait_fastboot
fi

# Only the active slot: the other one may hold a different kernel generation.
SLOT=$(active_slot)
fastboot flash "boot_$SLOT" "$IMG"
say "Rebooting. Next: tools/install-module.sh device/magisk/r1os-base"
fastboot reboot
