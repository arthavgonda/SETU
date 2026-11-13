#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${LLAMA_CPP_DIR:-$ROOT/llama.cpp}"
OUT="${LLAMA_CPP_BUILD:-$ROOT/build/llama-cpp}"
GEN=()
command -v ninja >/dev/null && GEN=(-G Ninja)
EXTRA=()
if [ "$(uname -s)" = "Darwin" ]; then
  EXTRA+=(-DGGML_METAL=ON -DCMAKE_INSTALL_RPATH=@loader_path -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON)
else
  EXTRA+=(-DCMAKE_INSTALL_RPATH='$ORIGIN' -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON)
fi
if [ -z "${VULKAN_SDK:-}" ] && ! command -v glslc >/dev/null; then
  SDK="$(ls -d "$ROOT"/.devcache/vulkan/*/x86_64 2>/dev/null | tail -1 || true)"
  [ -n "$SDK" ] && export VULKAN_SDK="$SDK"
fi
GLSLC="${VULKAN_SDK:+$VULKAN_SDK/bin/glslc}"
[ -x "$GLSLC" ] || GLSLC="$(command -v glslc || true)"
if [ "$(uname -s)" != "Darwin" ] && [ "${SETU_VULKAN:-1}" = "1" ] && [ -x "$GLSLC" ]; then
  EXTRA+=(-DGGML_VULKAN=ON -DVulkan_GLSLC_EXECUTABLE="$GLSLC")
  [ -n "${VULKAN_SDK:-}" ] && EXTRA+=(-DCMAKE_PREFIX_PATH="$VULKAN_SDK")
  echo "Vulkan backend: on ($GLSLC)"
else
  EXTRA+=(-DGGML_VULKAN=OFF)
  echo "Vulkan backend: off (install glslc or set VULKAN_SDK to build it)"
fi
cmake -S "$SRC" -B "$OUT" "${GEN[@]}" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DGGML_BACKEND_DL=ON \
  -DGGML_CPU_ALL_VARIANTS=ON \
  -DGGML_NATIVE=OFF \
  -DGGML_OPENMP=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  -DLLAMA_BUILD_TOOLS=OFF \
  -DLLAMA_BUILD_SERVER=OFF \
  -DLLAMA_BUILD_COMMON=OFF \
  -DLLAMA_BUILD_APP=OFF \
  -DLLAMA_BUILD_MTMD=ON \
  -DMTMD_VIDEO=OFF \
  "${EXTRA[@]}"
TARGETS=(--target llama --target mtmd)
if grep -q "GGML_VULKAN:BOOL=ON" "$OUT/CMakeCache.txt" 2>/dev/null; then TARGETS+=(--target ggml-vulkan); fi
cmake --build "$OUT" --config Release "${TARGETS[@]}" -j "$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
echo "llama.cpp libraries are in $OUT/bin"
