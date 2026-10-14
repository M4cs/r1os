#!/usr/bin/env bash
# Desktop simulator: r1d with no hardware (--sim) and the harness on, serving the built shell at
# http://127.0.0.1:7171. Arrow keys / mouse wheel = the scroll wheel, Space = the side button;
# input goes through r1d's gesture engine like on the R1.
#
# Usage: tools/sim.sh            build the shell if needed, run r1d
#        tools/sim.sh --dev      same, plus Vite with hot reload on http://127.0.0.1:5171
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need cargo
need pnpm
cd "$ROOT"
[ -d shell/node_modules ] || pnpm --dir shell install
[ -f shell/dist/index.html ] || pnpm --dir shell build
if [ "${1:-}" = --dev ]; then
  pnpm --dir shell dev &
  trap 'kill %1 2>/dev/null || true' EXIT
  say "Shell with hot reload: http://127.0.0.1:5171"
fi
# Push-to-talk in the simulator uses the same model as the R1 (and your PC's mic, through the page).
if [ -f out/speech/stt/tokens.txt ] && [ ! -f .r1os-sim/models/stt/tokens.txt ]; then
  mkdir -p .r1os-sim/models && cp -r out/speech/stt .r1os-sim/models/
fi
say "Simulator: http://127.0.0.1:7171"
cargo run -q -p r1d -- --sim --harness --shell shell/dist
