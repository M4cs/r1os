#!/usr/bin/env bash
# Builds r1os and deploys it to the R1:
#   r1d   → /data/r1os/bin/r1d
#   shell → /data/r1os/shell
#   speech-to-text (out/speech, tools/fetch-speech.sh) → /data/r1os/lib, /data/r1os/models/stt (the .ort copy)
#   System One's model, if fetched (out/system1, tools/fetch-models.sh) → /data/r1os/models/system1
#   the Translator's models, if fetched (out/translate, tools/fetch-models.sh) → /data/r1os/models/translate
# each relabeled for r1d's SELinux domain (restorecon, ADR-0035), then restarts r1d (r1os-base's service.sh loop, or init's `r1d` service on r1os's own image,
# starts the new binary within a second). The host app reconnects by itself when r1d comes back,
# and reloads its web pages (mini-apps, the fallback screens).
#
#   --host      also build and install the host app, and make it the home screen
#   --no-build  deploy what's already built
#
# Writes to the device (CLAUDE.md: agents ask the user before the first deploy of a session).
# Needs root over adb: Rooted debugging + `adb root`, or Magisk su.
#
# Usage: tools/deploy.sh [--host] [--no-build]
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need adb
need curl

HOST=0
BUILD=1
for a in "$@"; do
  case "$a" in
    --host) HOST=1 ;;
    --no-build) BUILD=0 ;;
    *) die "unknown option $a (see the header of $0)" ;;
  esac
done

adb_ready || die "No R1 over adb."
detect_root || die "Needs root over adb (enable Rooted debugging and run 'adb root', or allow Magisk su)."
cd "$ROOT"

if [ "$BUILD" = 1 ]; then
  tools/build-r1d.sh
  need bun
  [ -d node_modules ] || bun install --frozen-lockfile
  bun run --cwd shell build >/dev/null
  say "Built shell/dist"
  if [ "$HOST" = 1 ]; then tools/build-host.sh; fi
fi
BIN=target/aarch64-linux-android/release/r1d
[ -f "$BIN" ] || die "No $BIN (build first)."
[ -f shell/dist/index.html ] || die "No shell/dist (build first)."

# Something has to restart r1d: r1os-base's loop (LineageOS + Magisk) or init's `r1d` service (r1os's
# own image, device/aosp).
if ! root_sh 'test -f /data/adb/modules/r1os_base/service.sh && echo ok; [ -n "$(getprop init.svc.r1d)" ] && echo ok' | grep -q ok; then
  warn "Neither r1os-base v0.2+ nor r1os's image is here, so nothing restarts r1d. I'll start it by hand this time."
  warn "Install it for boot-time start: tools/install-module.sh device/magisk/r1os-base && adb reboot"
fi

# r1d won't start without the speech libs, so they go first. --sync skips files the device
# already has (the staging copies in /data/local/tmp stay as that cache; nothing in them is secret).
SPEECH=out/speech
[ -f "$SPEECH/lib/arm64-v8a/libsherpa-onnx-c-api.so" ] && [ -f "$SPEECH/stt-r1/tokens.txt" ] || die "No out/speech (run tools/fetch-speech.sh)."
say "Pushing speech-to-text (unchanged files skipped)"
adb push --sync "$SPEECH/lib/arm64-v8a/." /data/local/tmp/r1os-lib/ >/dev/null
adb shell 'mkdir -p /data/local/tmp/r1os-stt && cd /data/local/tmp/r1os-stt && rm -f *.onnx'
adb push --sync "$SPEECH/stt-r1/." /data/local/tmp/r1os-stt/ >/dev/null
# The running r1d has these libraries mapped: overwriting one in place (cp -f) crashed it mid-deploy
# (SIGSEGV, 2026-10-08). Unchanged ones are skipped; a changed one goes in under a new name and is
# renamed over the old, so the running r1d keeps its copy until the restart below. Files moved in from
# /data/local/tmp keep its label (and init may not relabel those), so each push ends with a forced restorecon
# (-F: the directory's stored hash would otherwise skip it; r1d's own labels, ADR-0035).
root_sh 'mkdir -p /data/r1os/lib && for f in /data/local/tmp/r1os-lib/*; do t=/data/r1os/lib/${f##*/}; cmp -s "$f" "$t" || { cp -f "$f" "$t.new" && mv -f "$t.new" "$t"; } || exit 1; done && rm -rf /data/r1os/models/stt && mkdir -p /data/r1os/models/stt && cp -f /data/local/tmp/r1os-stt/* /data/r1os/models/stt/ && chown -R 0:0 /data/r1os/lib /data/r1os/models && chmod -R go-rwx /data/r1os/lib /data/r1os/models && restorecon -RF /data/r1os/lib /data/r1os/models'

# System One's model (ADR-0032; tools/fetch-models.sh). Optional: without it, r1d routes by rules alone.
if [ -f out/system1/model.int8.onnx ]; then
  say "Pushing System One's model (unchanged files skipped)"
  adb push --sync out/system1/. /data/local/tmp/r1os-system1/ >/dev/null
  root_sh 'mkdir -p /data/r1os/models/system1 && cp -f /data/local/tmp/r1os-system1/* /data/r1os/models/system1/ && chown -R 0:0 /data/r1os/models/system1 && chmod -R go-rwx /data/r1os/models/system1 && restorecon -RF /data/r1os/models/system1'
fi

# The Translator's models (ADR-0034; tools/fetch-models.sh, ~2.4 GB). Optional: without them the Translator
# says it isn't installed. Only files newer than the R1's copy are copied in (adb push keeps their times).
if [ -f out/translate/asr/encoder.int8.onnx ]; then
  say "Pushing the Translator's models (unchanged files skipped; the first time takes a few minutes)"
  adb push --sync out/translate/. /data/local/tmp/r1os-translate/ >/dev/null
  root_sh 'S=/data/local/tmp/r1os-translate; D=/data/r1os/models/translate; cd $S && find . -type f | while read -r f; do t=$D/${f#./}; if [ ! -f "$t" ] || [ "$f" -nt "$t" ]; then mkdir -p "${t%/*}" && cp -f "$f" "$t" || exit 1; fi; done && chown -R 0:0 $D && chmod -R go-rwx $D && restorecon -RF $D'
fi

say "Pushing r1d and the shell"
adb shell 'rm -rf /data/local/tmp/r1os-shell /data/local/tmp/r1d.new'
adb push "$BIN" /data/local/tmp/r1d.new >/dev/null
adb push shell/dist/. /data/local/tmp/r1os-shell/ >/dev/null
root_sh 'mkdir -p /data/r1os/bin /data/r1os/log && mv -f /data/local/tmp/r1d.new /data/r1os/bin/r1d && chmod 755 /data/r1os/bin/r1d && rm -rf /data/r1os/shell && mv /data/local/tmp/r1os-shell /data/r1os/shell && chown 0:0 /data/r1os && chmod 700 /data/r1os && chown -R 0:0 /data/r1os/bin /data/r1os/shell && chmod -R go-rwx /data/r1os/bin /data/r1os/shell && restorecon -RF /data/r1os/bin /data/r1os/shell'

say "Restarting r1d"
root_sh 'pkill -x r1d; true'
# r1os-base's service.sh restarts r1d within a second, but while the binary was missing (a fresh
# install) it only rechecks every 5 s. Starting our own copy then left two r1d fighting over the
# ports (2026-10-09), so only start one when there's no loop at all. `[s]` keeps grep from
# matching itself.
#
# On r1os's own image (device/aosp, ADR-0013) the restarter is init's `r1d` service instead.
if root_sh 'ps -A -o ARGS | grep -q "[r]1os_base/service.sh" && echo loop; [ -n "$(getprop init.svc.r1d)" ] && echo loop' | grep -q loop; then
  for _ in $(seq 1 16); do
    root_sh 'pidof r1d >/dev/null && echo up' | grep -q up && break
    sleep 0.5
  done
else
  # No service loop (module missing or not rebooted yet): start it detached. Every fd must be
  # redirected or `adb shell` waits on the child forever.
  root_sh 'nohup /data/r1os/bin/r1d --harness </dev/null >>/data/r1os/log/r1d.log 2>&1 &'
  sleep 1
fi

BASE=$(r1d_forward)
for _ in $(seq 1 20); do
  # r1d makes a new token each start (ADR-0026): read it again on each try.
  if STATE=$(curl -sf -m 2 -H "Authorization: Bearer $(r1d_token)" "$BASE/api/state"); then break; fi
  sleep 0.5
done
[ -n "${STATE:-}" ] || die "r1d didn't come up; see: tools/r1h logs"
say "r1d is up: $(printf '%s' "$STATE" | tr -d '\n' | sed -E 's/.*"version":"([^"]*)".*/v\1/')"

if [ "$HOST" = 1 ]; then
  say "Installing the host app as the home screen"
  adb install -r "$OUT/r1os-host.apk" >/dev/null
  # Compiled ahead of time (T-100): an install leaves it interpreted until Android's idle dexopt. The install has
  # already restarted it on the old, interpreted code, so it starts again on the compiled code (T-106: until then
  # nearly half its main thread was ART's interpreter).
  adb shell cmd package compile -m speed -f dev.r1os.host >/dev/null
  adb shell am force-stop dev.r1os.host
  adb shell cmd package set-home-activity dev.r1os.host/.MainActivity >/dev/null
  # r1os's keyboard for every text box (ADR-0055): the host can't make itself Android's default.
  adb shell ime enable dev.r1os.host/.ime.R1Keyboard >/dev/null
  adb shell ime set dev.r1os.host/.ime.R1Keyboard >/dev/null
  adb shell am start -a android.intent.action.MAIN -c android.intent.category.HOME >/dev/null
fi
say "Done. Drive it with tools/r1h (state, wheel, button, screenshot, logs)."
