#!/usr/bin/env bash
# Replaces the R1's system with a GSI on the active slot, keeping Rabbit's kernel and vendor (the
# wheel and side button only work with Rabbit's kernel: ADR-0001). Turns verified boot off, wipes
# data, and deletes the stock product partition. Adapted from r1ptt (MIT); see NOTICE.md.
#
# GSI for the dev base: Andy Yan's LineageOS 21, arm64 vanilla:
#   lineage-21.0-YYYYMMDD-UNOFFICIAL-arm64_bvN.img.gz from
#   https://sourceforge.net/projects/andyyan-gsi/files/ (lineage-21-pre-qpr2-td or lineage-21-td)
# The stock firmware folder only supplies vbmeta*.img and the empty userdata.img.
#
# Usage: tools/flash-gsi.sh <gsi .img|.img.gz|.img.xz> <extracted stock firmware folder> [--keep-product]
#        (R1 in fastboot mode, bootloader unlocked, backup taken)
set -euo pipefail
source "$(dirname "$0")/lib.sh"

[ $# -ge 2 ] || die "usage: $0 <gsi image> <extracted stock firmware folder> [--keep-product]"
need fastboot
[ "${NO_BACKUP_OK:-0}" = 1 ] || require_backup
IMG=$(stock_dir "$2")
for f in vbmeta.img vbmeta_system.img vbmeta_vendor.img userdata.img; do
  [ -f "$IMG/$f" ] || die "missing $IMG/$f"
done
KEEP_PRODUCT=0
[ "${3:-}" = "--keep-product" ] && KEEP_PRODUCT=1
say "Unpacking $(basename "$1") (if compressed)"
GSI=$(unpacked "$1")
[ -f "$GSI" ] || die "GSI not found: $GSI"

wait_fastboot
if in_fastbootd; then
  say "In fastbootd; going to the bootloader first for vbmeta"
  fastboot reboot bootloader
  sleep 5
  wait_fastboot
fi
[ "$(fb_var unlocked)" = yes ] || die "Bootloader is locked."

# Flash the slot the R1 boots from: that's where its current kernel and vendor live.
SLOT=$(active_slot)
cat <<EOF

About to:
  1. flash vbmeta, vbmeta_system, vbmeta_vendor (verification off) to slots a and b
  2. ERASE userdata (everything on the R1) and write the stock empty userdata
  3. in fastbootd: $( [ "$KEEP_PRODUCT" = 1 ] && echo "keep" || echo "delete" ) product_$SLOT
  4. flash $(basename "$GSI") to system_$SLOT
Untouched: boot (kernel), vendor, dtbo, lk, preloader, nvram/nvdata and everything device-unique.
Active slot: $SLOT

EOF
confirm "Go ahead and ERASE all data on the R1?"

say "vbmeta with verification off"
flash_vbmeta_disabled "$IMG"

# Wipe data the way Rabbit's own restore does: erase, then the stock (empty) userdata image.
# fastbootd can't do it here (`fastboot -w` there reports "partition not found" and skips the wipe).
say "Wiping data"
fastboot erase userdata
fastboot flash userdata "$IMG/userdata.img"

say "Rebooting to fastbootd"
fastboot reboot fastboot
sleep 5
wait_fastboot
in_fastbootd || die "Didn't reach fastbootd. (On Windows, fastbootd may need the Android bootloader interface driver assigned again in Device Manager.)"
fastboot snapshot-update cancel 2>/dev/null || true

if [ "$KEEP_PRODUCT" = 0 ]; then
  # Stock product holds Rabbit's launcher and services: dead weight (and battery drain) on a GSI.
  say "Removing stock product_$SLOT"
  fastboot delete-logical-partition "product_$SLOT" 2>/dev/null || warn "product_$SLOT not deleted (maybe already gone)"
fi

say "Flashing the GSI to system_$SLOT"
fastboot flash "system_$SLOT" "$GSI"

say "Booting. First boot takes a few minutes."
say "Next: finish setup on the R1, then tools/device-report.sh and tools/root.sh (docs/runbooks/fresh-start.md)."
fastboot reboot
