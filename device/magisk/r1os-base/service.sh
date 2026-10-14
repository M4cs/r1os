#!/system/bin/sh
# r1os-base: starts r1d (the r1os core daemon) at boot and restarts it if it exits.
#
# r1d lives in /data/r1os/bin, not in this module, so development deploys (tools/deploy.sh) just
# push a new binary and `pkill -x r1d`; this loop starts the new one within a second.
# Log: /data/r1os/log/r1d.log, rotated at 1 MB (one old copy, r1d.log.1).
R1OS=/data/r1os
BIN=$R1OS/bin/r1d
LOG=$R1OS/log/r1d.log

# Root-only: r1d runs as root from here, so nothing else may write to it. (Magisk runs this with
# umask 0, which left /data/r1os world-writable on 2026-10-08.) The WebView never reads these
# files directly; r1d serves the shell over HTTP.
# Only the top level and r1d's own directories: never recurse into agent/ (the agent box and the agent
# user's home, uid 2950), apps/ or data/. A recursive chown here broke the box on 2026-10-08.
umask 077
mkdir -p $R1OS/bin $R1OS/shell $R1OS/log
chown 0:0 $R1OS $R1OS/bin $R1OS/shell $R1OS/log
chmod 700 $R1OS $R1OS/bin $R1OS/log

(
  while true; do
    if [ -x "$BIN" ]; then
      if [ -f "$LOG" ] && [ "$(stat -c %s "$LOG")" -gt 1048576 ]; then mv -f "$LOG" "$LOG.1"; fi
      echo "== r1d start $(date)" >>"$LOG"
      # --harness: dev builds expose /harness/* on 127.0.0.1 (input injection, screenshots).
      "$BIN" --harness >>"$LOG" 2>&1
      echo "== r1d exited $? $(date)" >>"$LOG"
      sleep 1
    else
      sleep 5
    fi
  done
) &
