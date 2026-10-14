#!/usr/bin/env bash
# Cross-compiles r1d for the R1 (aarch64-linux-android, API 30) with the Android NDK.
#
# Uses the NDK's clang.exe directly with an explicit --target rather than the
# aarch64-linux-android30-clang.cmd wrapper: .cmd wrappers misbehave when their path has spaces
# (C:\Users\Max Bridgland\...).
#
# Usage: tools/build-r1d.sh [--debug] [extra cargo args, e.g. clippy-only: CARGO_CMD=clippy]
# Output: target/aarch64-linux-android/release/r1d
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
# Set the flags here (CARGO_ENCODED_RUSTFLAGS, and RUSTFLAGS unset), not CARGO_TARGET_*_RUSTFLAGS: a
# global RUSTFLAGS takes precedence over the target-specific variable, and Max's machine sets
# RUSTFLAGS="-C target-feature=+crt-static" for Windows builds, which here would both drop the --target
# and statically link bionic.
#
# Speech-to-text (crate::voice) links k2-fsa's prebuilt sherpa-onnx + ONNX Runtime shared libs
# (tools/fetch-speech.sh), which tools/deploy.sh puts in /data/r1os/lib: hence the rpath. r1d also
# links libonnxruntime directly (--no-as-needed), so bionic has it loaded by soname when
# libsherpa-onnx-c-api.so (no rpath of its own) asks for it.
SPEECH_LIB="$ROOT/out/speech/lib/arm64-v8a"
[ -f "$SPEECH_LIB/libsherpa-onnx-c-api.so" ] || die "No speech libs in out/speech (run tools/fetch-speech.sh)."
export SHERPA_ONNX_LIB_DIR="$SPEECH_LIB"
# -L too: every linked artifact gets -lonnxruntime, and iroh-relay (crates/r1-link) builds a cdylib besides its lib.
# Encoded (separated by 0x1f, not spaces), since the speech libs' path may have a space in it.
SEP=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="-Clink-arg=--target=aarch64-linux-android$API${SEP}-Clink-arg=-Wl,-rpath,/data/r1os/lib${SEP}-Clink-arg=-Wl,--no-as-needed${SEP}-Clink-arg=-L$SPEECH_LIB${SEP}-Clink-arg=-lonnxruntime"
unset RUSTFLAGS
# For any crate that compiles C (none today, but keep cc-rs pointed at the NDK too).
export CC_aarch64_linux_android="$BIN/clang$EXE"
export CFLAGS_aarch64_linux_android="--target=aarch64-linux-android$API"
export AR_aarch64_linux_android="$BIN/llvm-ar$EXE"

PROFILE=--release
if [ "${1:-}" = --debug ]; then PROFILE=; shift; fi
cd "$ROOT"
cargo "${CARGO_CMD:-build}" -p r1d --target aarch64-linux-android $PROFILE "$@"
if [ "${CARGO_CMD:-build}" = build ]; then
  OUTBIN="target/aarch64-linux-android/$([ -n "$PROFILE" ] && echo release || echo debug)/r1d"
  say "Built $OUTBIN ($(( $(stat -c %s "$OUTBIN") / 1024 )) KiB)"
fi
