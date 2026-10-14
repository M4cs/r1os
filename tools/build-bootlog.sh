#!/usr/bin/env bash
# Cross-compiles verbose boot's program (crates/r1-bootlog, T-101) for the R1 into out/bootlog/r1os-bootlog,
# which tools/build-image.sh puts in the image (/system/bin/r1os-bootlog). The NDK setup is build-r1d.sh's,
# without the speech libraries: it links only bionic.
#
# Usage: tools/build-bootlog.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need cargo

API=30
SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$LOCALAPPDATA/Android/Sdk}}
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  ANDROID_NDK_HOME=$(find "$SDK/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -1)
fi
[ -d "${ANDROID_NDK_HOME:-}" ] || die "No Android NDK found (set ANDROID_NDK_HOME)."
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) HOST=windows-x86_64; EXE=.exe ;;
  Darwin) HOST=darwin-x86_64; EXE= ;;
  *) HOST=linux-x86_64; EXE= ;;
esac
BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST/bin"
[ -x "$BIN/clang$EXE" ] || die "No clang in $BIN"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/clang$EXE"
# As build-r1d.sh: flags here, and a global RUSTFLAGS (+crt-static, for Windows builds) out of the way.
export CARGO_ENCODED_RUSTFLAGS="-Clink-arg=--target=aarch64-linux-android$API"
unset RUSTFLAGS

cd "$ROOT"
cargo build -p r1-bootlog --target aarch64-linux-android --release
mkdir -p "$OUT/bootlog"
cp target/aarch64-linux-android/release/r1os-bootlog "$OUT/bootlog/r1os-bootlog"
say "Built $OUT/bootlog/r1os-bootlog ($(( $(stat -c %s "$OUT/bootlog/r1os-bootlog") / 1024 )) KiB)"
