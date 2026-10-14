#!/usr/bin/env bash
# Downloads r1os's speech-to-text pieces (ADR-0009) into out/speech (git-ignored), checked against
# pinned SHA-256s:
#   out/speech/lib/arm64-v8a/  sherpa-onnx's C API + ONNX Runtime for the R1 (k2-fsa release)
#   out/speech/stt/            the streaming model: encoder/decoder/joiner .onnx, tokens.txt,
#                              MODEL (its name) and test.wav (for /harness/hear). The simulator's.
#   out/speech/stt-r1/         the same model pre-optimized for the R1's ONNX Runtime (.ort, via
#                              tools/optimize-stt.py and uv): loads in ~14 s instead of ~35 s.
# tools/build-r1d.sh links against the libs, tools/deploy.sh pushes the libs and stt-r1, and
# tools/sim.sh gives the simulator stt. (Off Android the sherpa-onnx crate builds statically.)
#
# Usage: tools/fetch-speech.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need curl
need tar
need uv

SHERPA=1.13.8 # keep equal to the sherpa-onnx version in crates/r1d/Cargo.toml
LIBS=sherpa-onnx-v$SHERPA-android.tar.bz2
LIBS_SHA=2ff63469a71cb6009aa2e3ed5f4a670f8abdcbe4bb9ffd23776afc792a6b4f44
MODEL=sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06
MODEL_SHA=c8676e5ff9ac2a85296e53ee0fd4d5fb1db6770e7a7647166eeafe349ade6834
BASE=https://github.com/k2-fsa/sherpa-onnx/releases/download
D="$ROOT/out/speech"
mkdir -p "$D"

sha_ok() { [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$2" ]; }
fetch() { # url file sha256
  if [ ! -f "$D/$2" ] || ! sha_ok "$D/$2" "$3"; then
    say "Downloading $2"
    curl -fSL --retry 3 -o "$D/$2.part" "$1"
    mv "$D/$2.part" "$D/$2"
  fi
  sha_ok "$D/$2" "$3" || die "$2: SHA-256 mismatch"
}

fetch "$BASE/v$SHERPA/$LIBS" "$LIBS" "$LIBS_SHA"
fetch "$BASE/asr-models/$MODEL.tar.bz2" "$MODEL.tar.bz2" "$MODEL_SHA"

# Relative paths: GNU tar reads the "C:" of a Windows path as a remote host.
cd "$D"
rm -rf lib stt tmp
mkdir -p lib/arm64-v8a stt tmp
tar -xjf "$LIBS" -C tmp ./jniLibs/arm64-v8a/libsherpa-onnx-c-api.so ./jniLibs/arm64-v8a/libonnxruntime.so
mv tmp/jniLibs/arm64-v8a/*.so lib/arm64-v8a/
tar -xjf "$MODEL.tar.bz2" -C tmp
for f in encoder.onnx decoder.onnx joiner.onnx tokens.txt; do mv "tmp/$MODEL/$f" stt/; done
cp "tmp/$MODEL/test_wavs/0.wav" stt/test.wav
echo "$MODEL" >stt/MODEL
rm -rf tmp

# The R1's copy: ONNX Runtime's .ort format, optimized ahead of time (the onnxruntime minor
# version must match the R1's libonnxruntime.so: 1.28).
say "Optimizing the model for the R1 (.ort)"
rm -rf stt-r1 variants
uv run -q --python 3.11 --with onnxruntime==1.28.0 --with onnx python "$ROOT/tools/optimize-stt.py" stt variants ort >/dev/null 2>&1 ||
  die "tools/optimize-stt.py failed (rerun it by hand to see why)"
mv variants/ort stt-r1 && rm -rf variants && rm -f stt-r1/*.required_operators.config
say "Speech-to-text in out/speech: libs $(du -sh "$D/lib" | cut -f1), model $(du -sh "$D/stt" | cut -f1), R1 copy $(du -sh "$D/stt-r1" | cut -f1) ($MODEL)"
