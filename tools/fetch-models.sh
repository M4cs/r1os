#!/usr/bin/env bash
# Downloads the on-device models beyond speech-to-text into out/ (git-ignored), checked against pinned
# SHA-256s. tools/deploy.sh pushes what's there.
#
#   out/system1/   System One's sentence embedder (ADR-0032): all-MiniLM-L6-v2, int8 for ARM64 (Apache-2.0,
#                  sentence-transformers), and its WordPiece vocab. ~23 MB.
#   out/translate/ The Translator's (ADR-0034; tools/translator-models.txt): Whisper base (multilingual
#                  speech-to-text, int8), OPUS-MT models for English to and from es/fr/it/pt (ROMANCE), de,
#                  zh, ru, ar, hi, and ja/ko into English, and Piper voices for 10 languages. ~2.2 GB.
#                  The voices' archives stay in out/translate-voices.
#
# Usage: tools/fetch-models.sh            (everything)
#        TRANSLATOR=0 tools/fetch-models.sh  (System One only)
set -euo pipefail
source "$(dirname "$0")/lib.sh"
need curl

sha_ok() { [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$2" ]; }
fetch() { # url file sha256
  if [ -f "$2" ] && sha_ok "$2" "$3"; then return; fi
  say "Downloading ${2##*/}"
  curl -fL --retry 3 -o "$2.part" "$1" || die "download failed: $1"
  sha_ok "$2.part" "$3" || die "checksum mismatch: $1"
  mv "$2.part" "$2"
}

# System One (ADR-0032), pinned to the model repo's revision.
MINILM=https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/1110a243fdf4706b3f48f1d95db1a4f5529b4d41
D="$ROOT/out/system1"
mkdir -p "$D"
fetch "$MINILM/onnx/model_qint8_arm64.onnx" "$D/model.int8.onnx" 4278337fd0ff3c68bfb6291042cad8ab363e1d9fbc43dcb499fe91c871902474
fetch "$MINILM/vocab.txt" "$D/vocab.txt" 07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3
echo "all-MiniLM-L6-v2 (int8, arm64)" > "$D/MODEL"
say "System One's model is in out/system1"

[ "${TRANSLATOR:-1}" = 1 ] || exit 0
need tar
while read -r url sha dest; do
  case "$url" in "" | "#"*) continue ;; esac
  mkdir -p "$(dirname "$ROOT/out/$dest")"
  fetch "$url" "$ROOT/out/$dest" "$sha"
done <"$ROOT/tools/translator-models.txt"
echo "whisper-base (int8, multilingual)" >"$ROOT/out/translate/asr/MODEL"
for d in "$ROOT"/out/translate/mt/*/; do basename "$d" >"$d/MODEL"; done

# Voices: each archive has the model, its tokens and a copy of espeak-ng's data, which they share on the
# R1 (tts/espeak-ng-data). A few (Chinese) have a lexicon and text-normalising rules (*.fst) instead, kept
# beside their model. Relative paths: GNU tar reads the "C:" of a Windows path as a remote host.
cd "$ROOT/out"
rm -rf translate/tts voices-tmp
mkdir -p translate/tts voices-tmp
for a in translate-voices/*.tar.bz2; do
  lang=$(basename "$a" .tar.bz2)
  tar -xjf "$a" -C voices-tmp
  v=$(ls voices-tmp)
  mkdir -p "translate/tts/$lang"
  mv voices-tmp/"$v"/*.onnx "translate/tts/$lang/model.onnx"
  mv voices-tmp/"$v"/tokens.txt "translate/tts/$lang/"
  for f in voices-tmp/"$v"/lexicon.txt voices-tmp/"$v"/*.fst voices-tmp/"$v"/MODEL_CARD; do [ -f "$f" ] && mv "$f" "translate/tts/$lang/"; done
  [ -d translate/tts/espeak-ng-data ] || { [ -d voices-tmp/"$v"/espeak-ng-data ] && mv voices-tmp/"$v"/espeak-ng-data translate/tts/; }
  echo "$v" >"translate/tts/$lang/MODEL"
  rm -rf voices-tmp/"$v"
done
rm -rf voices-tmp
say "The Translator's models are in out/translate ($(du -sh translate | cut -f1))"
