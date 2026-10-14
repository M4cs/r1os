#!/usr/bin/env bash
# Path B only (docs/runbooks/fresh-start.md). Puts a known stock base on the R1 from the command
# line: rabbitOS v0.8.293's kernel (boot), dtbo, vbmeta and super (system + vendor + product), on
# slot a, with verified boot off. Adapted from r1ptt's flash-baseline.sh (MIT); see NOTICE.md.
#
# Compared with Rabbit's WebUSB tool ("Flash Stock ROM"):
#   - this keeps the bootloader unlocked (Rabbit's tool relocks it, and re-unlocking depends on
#     `fastboot flashing get_unlock_ability`);
#   - but it never touches preloader/lk/firmware blobs, so an R1 that took OTAs after v0.8.293 ends
#     up with a newer bootloader chain next to the v0.8.293 kernel. That combination worked for r1ptt,
#     but Rabbit's tool is the cleaner reset.
#
# Never touched: preloader, lk, nvram/nvdata/nvcfg, protect*, proinfo, seccfg, frp, md1img.
#
# Usage: tools/flash-stock-base.sh <folder with the extracted rabbit_OS_v0.8.293.zip>
#        (R1 in fastboot mode, bootloader unlocked, backup taken)
set -euo pipefail
source "$(dirname "$0")/lib.sh"

[ $# -ge 1 ] || die "usage: $0 <extracted stock firmware folder>"
need fastboot
[ "${NO_BACKUP_OK:-0}" = 1 ] || require_backup
IMG=$(stock_dir "$1")
for f in boot.img dtbo.img vbmeta.img vbmeta_system.img vbmeta_vendor.img super.img userdata.img; do
  [ -f "$IMG/$f" ] || die "missing $IMG/$f"
done

wait_fastboot
in_fastbootd && die "This is fastbootd; reboot to the bootloader first: fastboot reboot bootloader"
[ "$(fb_var unlocked)" = yes ] || die "Bootloader is locked."

cat <<EOF

About to:
  1. switch to slot a
  2. flash stock boot + dtbo (v0.8.293 kernel) to slots a and b
  3. flash vbmeta* with verification off to slots a and b
  4. ERASE userdata and write the stock empty userdata
  5. in fastbootd: flash stock super (system + vendor + product)
Untouched: preloader, lk, nvram/nvdata, protect*, proinfo, seccfg, frp, md1img.

EOF
confirm "Flash stock v0.8.293 and ERASE all data on the R1?"

say "Slot a, stock kernel and dtbo"
# Stock super holds slot-a partitions, so slot a must be the one that boots.
fastboot --set-active=a || true
[ "$(active_slot)" = a ] || die "Couldn't switch to slot a. Use Rabbit's flash tool for the baseline instead."
for s in a b; do
  fastboot flash "boot_$s" "$IMG/boot.img"
  fastboot flash "dtbo_$s" "$IMG/dtbo.img"
done

say "vbmeta with verification off"
flash_vbmeta_disabled "$IMG"

say "Wiping data"
fastboot erase userdata
fastboot flash userdata "$IMG/userdata.img"

say "Rebooting to fastbootd for the super partition"
fastboot reboot fastboot
sleep 5
wait_fastboot
in_fastbootd || die "Didn't reach fastbootd."
fastboot snapshot-update cancel 2>/dev/null || true
say "Flashing super (about two minutes)"
fastboot flash super "$IMG/super.img"

say "Booting stock rabbitOS. Once it reaches its setup screen the base is good; next: tools/flash-gsi.sh"
fastboot reboot
