#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/windows"
W="${SETU_WIN_WORK:-$ROOT/build/windows}"
OUT="${OUT:-$ROOT/dist}"
VERSION="$(sed -n 's/^project(Setu VERSION \([0-9.]*\).*/\1/p' "$ROOT/CMakeLists.txt")"
QT_VERSION="6.10.2"
LLVM_MINGW="20261020"
LLVM_MINGW_SHA="55064b60b991a1e1e8090b8c2f84e82b22cfe5903d12c159d65064cf5e8ae335"
SHERPA_ARCHIVE="sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-lib"
SHERPA_SHA="afa4de3ca4ab0afd53bb4aab40d28ee15608364b549ffc0fe0caf6489bfc39a5"
ESPEAK_SHA="bb4338102ff3b49a81423da8a1a158b420124b055b60fa76cfb4b18677130a23"
TARGET="x86_64-pc-windows-gnullvm"
export PATH="$HOME/.cargo/bin:$PATH"

step() { printf '\n== %s\n' "$*"; }
fetch() {
  local url="$1" sha="$2" out="$3"
  mkdir -p "$(dirname "$out")"
  if [ -f "$out" ] && echo "$sha  $out" | sha256sum -c --status; then return; fi
  curl -sSfL --retry 5 -o "$out.part" "$url"
  echo "$sha  $out.part" | sha256sum -c --status || { echo "checksum mismatch for $url" >&2; rm -f "$out.part"; exit 1; }
  mv "$out.part" "$out"
}

mkdir -p "$W" "$OUT"
LLVM="$W/llvm-mingw"
QT="$W/qt/$QT_VERSION/llvm-mingw_64"
QT_HOST="$W/qt/$QT_VERSION/gcc_64"

step "Cross toolchain (llvm-mingw $LLVM_MINGW)"
if [ ! -x "$LLVM/bin/x86_64-w64-mingw32-clang" ]; then
  fetch "https://github.com/mstorsjo/llvm-mingw/releases/download/$LLVM_MINGW/llvm-mingw-$LLVM_MINGW-ucrt-ubuntu-22.04-x86_64.tar.xz" "$LLVM_MINGW_SHA" "$W/dl/llvm/llvm-mingw.tar.xz"
  tar -xJf "$W/dl/llvm/llvm-mingw.tar.xz" -C "$W"
  mv "$W/llvm-mingw-$LLVM_MINGW-ucrt-ubuntu-22.04-x86_64" "$LLVM"
fi
cat > "$W/toolchain.cmake" <<EOF
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_C_COMPILER $LLVM/bin/x86_64-w64-mingw32-clang)
set(CMAKE_CXX_COMPILER $LLVM/bin/x86_64-w64-mingw32-clang++)
set(CMAKE_RC_COMPILER $LLVM/bin/x86_64-w64-mingw32-windres)
set(CMAKE_AR $LLVM/bin/llvm-ar)
set(CMAKE_RANLIB $LLVM/bin/llvm-ranlib)
set(CMAKE_FIND_ROOT_PATH $LLVM/x86_64-w64-mingw32 $QT)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
EOF

step "Qt $QT_VERSION for Windows and matching host tools"
if [ ! -d "$QT" ] || [ ! -d "$QT_HOST" ]; then
  python3 -m venv "$W/venv"
  "$W/venv/bin/pip" install -q aqtinstall
  "$W/venv/bin/aqt" install-qt windows desktop "$QT_VERSION" win64_llvm_mingw -O "$W/qt"
  "$W/venv/bin/aqt" install-qt linux desktop "$QT_VERSION" linux_gcc_64 -O "$W/qt"
fi

step "sherpa-onnx for Windows"
SHERPA="$W/sherpa-root"
if [ ! -f "$SHERPA/lib/sherpa-onnx-c-api.dll" ]; then
  fetch "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/$SHERPA_ARCHIVE.tar.bz2" "$SHERPA_SHA" "$W/dl/sherpa/sherpa.tar.bz2"
  mkdir -p "$SHERPA/include/sherpa-onnx/c-api" "$SHERPA/lib" "$W/sherpa"
  tar -xjf "$W/dl/sherpa/sherpa.tar.bz2" -C "$W/sherpa"
  cp "$W/sherpa/$SHERPA_ARCHIVE/lib/"* "$SHERPA/lib/"
  cp "$ROOT/third_party/sherpa-onnx/include/sherpa-onnx/c-api/c-api.h" "$SHERPA/include/sherpa-onnx/c-api/"
  cp "$ROOT/third_party/sherpa-onnx/LICENSE" "$SHERPA/"
fi

step "espeak-ng 1.52.0 (compiled directly, without its build system)"
if [ ! -f "$W/espeak-out/espeak-ng.exe" ]; then
  fetch "https://github.com/espeak-ng/espeak-ng/archive/refs/tags/1.52.0.tar.gz" "$ESPEAK_SHA" "$W/dl/espeak-src/espeak-ng-1.52.0.tar.gz"
  rm -rf "$W/espeak-src" && mkdir -p "$W/espeak-src" "$W/espeak-out/cfg"
  tar -xzf "$W/dl/espeak-src/espeak-ng-1.52.0.tar.gz" -C "$W/espeak-src" --strip-components=1
  printf '#define HAVE_MKSTEMP 0\n#define USE_ASYNC 0\n#define USE_KLATT 0\n#define USE_LIBPCAUDIO 0\n#define USE_LIBSONIC 0\n#define USE_MBROLA 0\n#define USE_SPEECHPLAYER 0\n#define PACKAGE_VERSION "1.52.0"\n' > "$W/espeak-out/cfg/config.h"
  E="$W/espeak-src/src"
  SRCS=()
  for f in common mnemonics error ieee80 compiledata compiledict dictionary encoding intonation langopts numbers phoneme phonemelist readclause setlengths soundicon spect ssml synthdata synthesize tr_languages translate translateword voices wavegen speech espeak_api; do SRCS+=("$E/libespeak-ng/$f.c"); done
  SRCS+=("$E"/ucd-tools/src/*.c "$E/espeak-ng.c")
  "$LLVM/bin/x86_64-w64-mingw32-clang" -O2 -std=gnu99 -fwrapv -w -static -DLIBESPEAK_NG_EXPORT=1 -DPATH_ESPEAK_DATA='"espeak-ng-data"' \
    -I"$W/espeak-out/cfg" -I"$E/include" -I"$E/include/compat" -I"$E/ucd-tools/src/include" -I"$E/libespeak-ng" "${SRCS[@]}" -o "$W/espeak-out/espeak-ng.exe"
fi
VOICE_URL="$(python3 -I -c "import json,sys; m=json.load(open(sys.argv[1])); print(next(e['url'] for e in m['models'] if e['id']=='tts-amy-low'))" "$ROOT/daemon/core/assets/manifest.json")"
VOICE_SHA="$(python3 -I -c "import json,sys; m=json.load(open(sys.argv[1])); print(next(e['sha256'] for e in m['models'] if e['id']=='tts-amy-low'))" "$ROOT/daemon/core/assets/manifest.json")"
if [ ! -d "$W/espeak-data/espeak-ng-data" ]; then
  fetch "$VOICE_URL" "$VOICE_SHA" "$W/dl/voice/voice.tar.bz2"
  rm -rf "$W/espeak-data" && mkdir -p "$W/espeak-data"
  tar -xjf "$W/dl/voice/voice.tar.bz2" -C "$W/espeak-data" --strip-components=1 --wildcards '*/espeak-ng-data/*'
fi

step "Vulkan for Windows"
VK_SDK="${VULKAN_SDK:-$(ls -d "$ROOT"/.devcache/vulkan/*/x86_64 2>/dev/null | tail -1 || true)}"
VKFLAGS=(-DGGML_VULKAN=OFF)
VKTARGET=()
if [ "${SETU_VULKAN:-1}" = "1" ] && [ -x "$VK_SDK/bin/glslc" ]; then
  mkdir -p "$W/vulkan"
  if [ ! -f "$W/vulkan/libvulkan-1.a" ]; then
    LOADER="$(find "$VK_SDK/lib" -name 'libvulkan.so.1' | head -1)"
    { echo "LIBRARY vulkan-1.dll"; echo "EXPORTS"; nm -D --defined-only "$LOADER" | awk '$2=="T" && $3 ~ /^vk/ {print "    "$3}'; } > "$W/vulkan/vulkan-1.def"
    "$LLVM/bin/llvm-dlltool" -m i386:x86-64 -d "$W/vulkan/vulkan-1.def" -l "$W/vulkan/libvulkan-1.a"
  fi
  VKFLAGS=(-DGGML_VULKAN=ON -DVulkan_INCLUDE_DIR="$VK_SDK/include" -DVulkan_LIBRARY="$W/vulkan/libvulkan-1.a" -DVulkan_GLSLC_EXECUTABLE="$VK_SDK/bin/glslc" -DSPIRV-Headers_DIR="$VK_SDK/share/cmake/SPIRV-Headers")
  VKTARGET=(--target ggml-vulkan)
  echo "Vulkan backend: on"
else
  echo "Vulkan backend: off (set VULKAN_SDK to a Vulkan SDK with glslc)"
fi

step "llama.cpp for Windows"
if [ ! -f "$W/llama/bin/libllama.dll" ] || [ ! -f "$W/llama/bin/libmtmd.dll" ] || { [ ${#VKTARGET[@]} -gt 0 ] && [ ! -f "$W/llama/bin/ggml-vulkan.dll" ]; }; then
  cmake -S "${LLAMA_CPP_DIR:-$ROOT/llama.cpp}" -B "$W/llama" -G Ninja -DCMAKE_TOOLCHAIN_FILE="$W/toolchain.cmake" -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=ON -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF \
    -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_TOOLS=OFF -DLLAMA_BUILD_SERVER=OFF -DLLAMA_BUILD_COMMON=OFF -DLLAMA_CURL=OFF -DLLAMA_BUILD_MTMD=ON -DMTMD_VIDEO=OFF "${VKFLAGS[@]}"
  cmake --build "$W/llama" --target llama --target mtmd "${VKTARGET[@]}"
fi
mkdir -p "$W/llama-lib"
cp "$W/llama/src/libllama.dll.a" "$W/llama/ggml/src/libggml.dll.a" "$W/llama/ggml/src/libggml-base.dll.a" "$W/llama/bin/"*.dll "$W/llama-lib/"

step "Daemon and engines"
"$ROOT/scripts/fetch-deps.sh"
rustup target add "$TARGET" >/dev/null
(
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$LLVM/bin/x86_64-w64-mingw32-clang"
  export CC_x86_64_pc_windows_gnullvm="$LLVM/bin/x86_64-w64-mingw32-clang"
  export CXX_x86_64_pc_windows_gnullvm="$LLVM/bin/x86_64-w64-mingw32-clang++"
  export AR_x86_64_pc_windows_gnullvm="$LLVM/bin/llvm-ar"
  export BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_gnullvm="--target=x86_64-w64-mingw32 --sysroot=$LLVM/x86_64-w64-mingw32"
  export ORT_INCLUDE_DIR="$ROOT/third_party/onnxruntime/include" SHERPA_ONNX_DIR="$SHERPA" LLAMA_CPP_DIR="${LLAMA_CPP_DIR:-$ROOT/llama.cpp}" LLAMA_CPP_LIB="$W/llama-lib" SETU_PACKAGING=1 CARGO_TARGET_DIR="$W/cargo"
  export SETU_BUILD_ID="${SETU_BUILD_ID:-$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || date -u +%Y%m%d)}"
  cd "$ROOT/daemon"
  cargo build --release --locked --target "$TARGET" -p setu-daemon -p setu-engine -p setu-system -p setu-facecheck -p setu-credprov --features setu-audio/portable
)

step "Interface"
cmake -S "$ROOT" -B "$W/qt-build" -G Ninja -DCMAKE_TOOLCHAIN_FILE="$QT/lib/cmake/Qt6/qt.toolchain.cmake" -DQT_CHAINLOAD_TOOLCHAIN_FILE="$W/toolchain.cmake" \
  -DQT_HOST_PATH="$QT_HOST" -DCMAKE_BUILD_TYPE=Release -DSETU_BUILD_TESTS=OFF -DSETU_RUST_TARGET="$TARGET" -DSETU_BUILD_ID="${SETU_BUILD_ID:-windows}" > "$W/qt-config.log"
cmake --build "$W/qt-build"

step "Folder"
R="$W/cargo/$TARGET/release"
EXTRA=()
for f in "$W/llama/bin/"ggml-cpu-*.dll "$W/llama/bin/"ggml-vulkan.dll "$W/llama/bin/"libmtmd.dll "$R/setu_credprov.dll" "$SHERPA/lib/onnxruntime_providers_shared.dll"; do [ -f "$f" ] && EXTRA+=(--extra-dll "$f"); done
python3 -I "$HERE/deploy.py" --out "$W/stage/Setu" --qt "$QT" --objdump "$LLVM/bin/x86_64-w64-mingw32-objdump" \
  --exe "$W/qt-build/setu-shell.exe" --exe "$W/qt-build/setu-ui.exe" --exe "$R/setu-daemon.exe" --exe "$R/setu-engine.exe" --exe "$R/setu-system.exe" --exe "$R/setu-facecheck.exe" \
  --dll-dir "$LLVM/x86_64-w64-mingw32/bin" --dll-dir "$SHERPA/lib" --dll-dir "$W/llama/bin" "${EXTRA[@]}" \
  --espeak "$W/espeak-out/espeak-ng.exe" --espeak-data "$W/espeak-data/espeak-ng-data" \
  --models "$ROOT/third_party/bundled/models" --benchmark "$ROOT/third_party/bundled/benchmark.wav" \
  --license setu.txt="$ROOT/assets/licenses/setu.txt" --license third-party.json="$W/qt-build/generated/licenses/third_party.json" \
  --license espeak-ng-COPYING.txt="$W/espeak-src/COPYING" --license llvm-mingw-LICENSE.txt="$LLVM/LICENSE.TXT" --license sherpa-onnx-LICENSE.txt="$SHERPA/LICENSE"


step "Installer and portable zip"
MAKENSIS="$(command -v makensis || true)"
NSISDIR_ARG=""
if [ -z "$MAKENSIS" ] && [ -x "$W/nsis/usr/bin/makensis" ]; then
  MAKENSIS="$W/nsis/usr/bin/makensis"
  export NSISDIR="$W/nsis/usr/share/nsis"
fi
if [ -n "$MAKENSIS" ]; then
  (cd "$HERE" && "$MAKENSIS" -NOCONFIG -V2 -DVERSION="$VERSION" -DSTAGE="$W/stage/Setu" -DOUTFILE="$OUT/Setu-$VERSION-x64-setup.exe" setu.nsi)
else
  echo "makensis was not found; only the portable zip is produced" >&2
fi
rm -f "$OUT/Setu-$VERSION-x64-portable.zip"
(cd "$W/stage" && python3 -I -c "import shutil,sys; shutil.make_archive(sys.argv[1], 'zip', '.', 'Setu')" "$OUT/Setu-$VERSION-x64-portable")
(cd "$OUT" && sha256sum "Setu-$VERSION-x64-"* > "SHA256SUMS-windows")
ls -la "$OUT"/Setu-"$VERSION"-x64-*
