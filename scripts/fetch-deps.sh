#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TP="$ROOT/third_party"
SHERPA_VERSION="2.0.1"
G="https://github.com/k2-fsa/sherpa-onnx/releases/download"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

verify() {
  if command -v sha256sum >/dev/null; then sha256sum -c --status; else shasum -a 256 -c --status; fi
}

fetch() {
  local url="$1" sha="$2" out="$3"
  if [ -f "$out" ] && echo "$sha  $out" | verify 2>/dev/null; then return; fi
  curl -sSfL --retry 5 -o "$out.part" "$url"
  echo "$sha  $out.part" | verify || { echo "checksum mismatch for $url" >&2; rm -f "$out.part"; exit 1; }
  mv "$out.part" "$out"
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) LIB="sherpa-onnx-v$SHERPA_VERSION-linux-x64-shared-lib.tar.bz2"; LIBSHA="532c45bbd41036d83cd70606797529aa8086c2053395a58967736c727625b120" ;;
  Darwin-*) LIB="sherpa-onnx-v$SHERPA_VERSION-osx-universal2-shared-lib.tar.bz2"; LIBSHA="a3aa3e8a6c78bc06d98f8a5792a924e3d2a7c308c76db68aef358b9278366ac7" ;;
  *) LIB="${SHERPA_LIB_ARCHIVE:?set SHERPA_LIB_ARCHIVE and SHERPA_LIB_SHA256 for this platform}"; LIBSHA="${SHERPA_LIB_SHA256:?}" ;;
esac

place() {
  local src="$1" dst="$2"
  if [ -f "$dst" ] && cmp -s "$src" "$dst"; then return; fi
  cp -P "$src" "$dst.new.$$" && mv -f "$dst.new.$$" "$dst"
}

mkdir -p "$TP/sherpa-onnx/include/sherpa-onnx/c-api" "$TP/sherpa-onnx/lib" "$TP/bundled/models"
if [ "$(cat "$TP/sherpa-onnx/.stamp" 2>/dev/null)" != "$LIBSHA" ]; then
  fetch "$G/v$SHERPA_VERSION/$LIB" "$LIBSHA" "$TMP/$LIB"
  tar -xjf "$TMP/$LIB" -C "$TMP"
  for f in "$TMP/${LIB%.tar.bz2}"/lib/*; do place "$f" "$TP/sherpa-onnx/lib/$(basename "$f")"; done
  echo "$LIBSHA" > "$TP/sherpa-onnx/.stamp"
fi
RAW="https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/v$SHERPA_VERSION"
fetch "$RAW/sherpa-onnx/c-api/c-api.h" "5698f38ba09ad2bdf085c56a6cbbd7741acf15e25f4ce5c7097b9702cadf437f" "$TP/sherpa-onnx/include/sherpa-onnx/c-api/c-api.h"
fetch "$RAW/LICENSE" "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30" "$TP/sherpa-onnx/LICENSE"
mkdir -p "$TP/onnxruntime/include"
ORT_RAW="https://raw.githubusercontent.com/microsoft/onnxruntime/v1.28.2/include/onnxruntime/core/session"
fetch "$ORT_RAW/onnxruntime_c_api.h" "b69a133143c0da61782b2b02cc8a620d21ac139231fd211719776d10385135c1" "$TP/onnxruntime/include/onnxruntime_c_api.h"
fetch "$ORT_RAW/onnxruntime_error_code.h" "5ce3b054e798eced8d14f5b86e98692fd33470463f96194ce0700a2d53dd8721" "$TP/onnxruntime/include/onnxruntime_error_code.h"
fetch "$ORT_RAW/onnxruntime_ep_c_api.h" "db86df0f846b8d3bdbf429a5c76c5805293196575ca3c475878612591cd18e51" "$TP/onnxruntime/include/onnxruntime_ep_c_api.h"

KWS="sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01"
KWSSHA="f170013b4716e41b62b9bfd809687c207cef798ef9bc6534d524e17af9b6561a"
D="$TP/bundled/models/wake-gigaspeech-3m"
mkdir -p "$D"
if [ "$(cat "$D/.stamp" 2>/dev/null)" != "$KWSSHA" ]; then
  fetch "$G/kws-models/$KWS.tar.bz2" "$KWSSHA" "$TMP/kws.tar.bz2"
  tar -xjf "$TMP/kws.tar.bz2" -C "$TMP"
  for f in encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx decoder-epoch-12-avg-2-chunk-16-left-64.onnx joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx tokens.txt bpe.model; do
    place "$TMP/$KWS/$f" "$D/$f"
  done
  place "$TMP/$KWS/test_wavs/0.wav" "$TP/bundled/benchmark.wav"
  echo "$KWSSHA" > "$D/.stamp"
fi

mkdir -p "$TP/bundled/models/vad-silero" "$TP/bundled/models/denoise-gtcrn"
fetch "$G/asr-models/silero_vad.onnx" "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6" "$TP/bundled/models/vad-silero/silero_vad.onnx"
fetch "$G/speech-enhancement-models/gtcrn_simple.onnx" "e77603ac0c23dac3227dd2d7135b3a585cbee2679048aecfa886657d3ae1b534" "$TP/bundled/models/denoise-gtcrn/gtcrn_simple.onnx"
echo "Dependencies are in $TP"
