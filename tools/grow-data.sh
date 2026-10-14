#!/usr/bin/env bash
# Grows /data's ext4 filesystem to fill the userdata partition, online and without a wipe.
#
# Why: tools/flash-gsi.sh writes Rabbit's small empty userdata.img (a 2.8 GB filesystem). Rabbit's
# vendor fstab asks for it to be grown on mount (the `resize` flag), but the Android 14 GSI's
# libfstab ignores that flag ("unknown flag: resize"), so /data stays at 2.8 GB of the 115 GB
# partition. The metadata-encryption device (dm-default-key, named "userdata") already spans the
# whole partition; only the filesystem needs growing. Seen on Max's R1 on 2026-10-08.
#
# Usage: tools/grow-data.sh     (R1 booted, with root over adb: Rooted debugging + adb root, or Magisk su)
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
adb_ready || die "No R1 over adb."
detect_root || die "Needs root (enable Rooted debugging and run 'adb root', or allow Magisk su for Shell)."

DEV=$(adb_sh df /data | tail -1 | awk '{print $1}')
case "$DEV" in /dev/block/*) ;; *) die "Couldn't find /data's block device (got '$DEV')." ;; esac
DEV_BYTES=$(root_sh "blockdev --getsize64 $DEV" | tr -d '\r')
read -r BLOCKS BSIZE _ <<<"$(root_sh 'stat -f -c "%b %S %f" /data' | tr -d '\r')"
FS_BYTES=$(( BLOCKS * BSIZE ))
say "/data: filesystem $(( FS_BYTES / 1000000000 )) GB on $DEV ($(( DEV_BYTES / 1000000000 )) GB)"

# statfs reports a little less than the device because of ext4 metadata; 90 % means "already grown".
if [ $(( FS_BYTES * 10 )) -ge $(( DEV_BYTES * 9 )) ]; then
  say "Already full size; nothing to do."
  exit 0
fi

confirm "Grow /data online to fill $DEV with resize2fs?"
root_sh "resize2fs $DEV 2>&1; sync"
say "$(adb_sh df -h /data | tail -1 | awk '{print "/data now " $2 " (" $4 " free)"}')"
