#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/macos"
VERSION="$(sed -n 's/^project(Setu VERSION \([0-9.]*\).*/\1/p' "$ROOT/CMakeLists.txt")"
OUT="${OUT:-$ROOT/dist}"
WORK="$ROOT/build/package-macos"
if [ -z "${QT_DIR:-}" ]; then
  QT_DIR="$(ls -d "$HOME"/Qt/6.*/macos 2>/dev/null | sort -V | tail -1 || true)"
  if [ -z "$QT_DIR" ] && command -v brew >/dev/null; then QT_DIR="$(brew --prefix qt 2>/dev/null || true)"; fi
fi
if [ ! -x "${QT_DIR:-}/bin/macdeployqt" ]; then
  echo "Qt 6 was not found. Install it with 'brew install qt' or set QT_DIR, for example ~/Qt/6.8.3/macos" >&2
  exit 1
fi
if [ -z "${SETU_ARCHS:-}" ]; then
  QT_ARCHS="$(lipo -archs "$QT_DIR/lib/QtCore.framework/QtCore" 2>/dev/null || true)"
  case "$QT_ARCHS" in
    *arm64*x86_64*|*x86_64*arm64*) SETU_ARCHS="arm64 x86_64" ;;
    *) SETU_ARCHS="$(uname -m)" ;;
  esac
fi
ARCHS="$SETU_ARCHS"
export PATH="$HOME/.cargo/bin:$QT_DIR/bin:$PATH"
export SETU_PACKAGING=1
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/build/package-macos/cargo}"
export SETU_BUILD_ID="${SETU_BUILD_ID:-$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || date -u +%Y%m%d)}"
export MACOSX_DEPLOYMENT_TARGET=12.0

step() { printf '\n== %s\n' "$*"; }
CMAKE_ARCHS="$(echo "$ARCHS" | tr ' ' ';')"

step "Dependencies for $ARCHS with Qt from $QT_DIR"
"$ROOT/scripts/fetch-deps.sh"
LLAMA_CPP_BUILD="$WORK/llama" cmake -S "${LLAMA_CPP_DIR:-$ROOT/llama.cpp}" -B "$WORK/llama" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON -DGGML_METAL=ON -DGGML_NATIVE=OFF -DGGML_BACKEND_DL=OFF \
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_TOOLS=OFF -DLLAMA_BUILD_SERVER=OFF -DLLAMA_BUILD_COMMON=OFF -DLLAMA_BUILD_APP=OFF -DLLAMA_BUILD_MTMD=ON -DMTMD_VIDEO=OFF \
  -DCMAKE_OSX_ARCHITECTURES="$CMAKE_ARCHS" -DCMAKE_INSTALL_RPATH=@loader_path -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON
cmake --build "$WORK/llama" --target llama --target mtmd

step "Daemon and engines"
BINS=()
for a in $ARCHS; do
  t="$([ "$a" = arm64 ] && echo aarch64-apple-darwin || echo x86_64-apple-darwin)"
  rustup target add "$t" >/dev/null
  (cd "$ROOT/daemon" && LLAMA_CPP_LIB="$WORK/llama/bin" cargo build --release --locked --target "$t" --features setu-audio/portable -p setu-daemon -p setu-engine -p setu-system)
  BINS+=("$CARGO_TARGET_DIR/$t/release")
done

step "Interface"
cmake -S "$ROOT" -B "$WORK/qt" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH="$QT_DIR" -DCMAKE_OSX_ARCHITECTURES="$CMAKE_ARCHS" -DSETU_BUILD_TESTS=OFF -DSETU_BUILD_ID="$SETU_BUILD_ID"
cmake --build "$WORK/qt"

step "Bundle"
APP="$WORK/Setu.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources" "$APP/Contents/Library/LaunchAgents"
sed -e "s/@VERSION@/$VERSION/" -e "s/@BUILD@/$SETU_BUILD_ID/" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
cp "$WORK/qt/setu-shell.app/Contents/MacOS/setu-shell" "$APP/Contents/MacOS/"
cp "$WORK/qt/setu-ui.app/Contents/MacOS/setu-ui" "$APP/Contents/MacOS/"
for b in setu-daemon setu-engine setu-system; do
  lipo -create $(for d in "${BINS[@]}"; do echo "$d/$b"; done) -output "$APP/Contents/MacOS/$b"
done
cp -P "$ROOT"/third_party/sherpa-onnx/lib/*.dylib "$APP/Contents/Frameworks/"
cp -P "$WORK"/llama/bin/*.dylib "$APP/Contents/Frameworks/"
cp -r "$ROOT/third_party/bundled/models" "$APP/Contents/Resources/"
find "$APP/Contents/Resources/models" -name .stamp -delete
cp "$ROOT/third_party/bundled/benchmark.wav" "$APP/Contents/Resources/"
cp "$HERE/ai.setu.daemon.plist" "$APP/Contents/Library/LaunchAgents/"
if command -v espeak-ng >/dev/null; then
  cp "$(command -v espeak-ng)" "$APP/Contents/Resources/"
  for d in /opt/homebrew/share/espeak-ng-data /usr/local/share/espeak-ng-data; do [ -d "$d" ] && cp -r "$d" "$APP/Contents/Resources/" && break; done
fi
ICONSET="$WORK/Setu.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s "$HERE/icon-1024.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) "$HERE/icon-1024.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Setu.icns"
for b in setu-daemon setu-engine; do
  install_name_tool -add_rpath @executable_path/../Frameworks "$APP/Contents/MacOS/$b" 2>/dev/null || true
done
macdeployqt "$APP" -qmldir="$ROOT/qml" -executable="$APP/Contents/MacOS/setu-ui" -always-overwrite
find "$APP/Contents/PlugIns" -type d \( -name "qmltooling" -o -name "virtualkeyboard" -o -name "sqldrivers" -o -name "multimedia" \) -prune -exec rm -rf {} +

step "Sign"
sign_tree() {
  find "$APP/Contents" \( -name "*.dylib" -o -name "*.so" -o -perm -111 -type f \) -print0 | while IFS= read -r -d '' f; do
    codesign --force $SIGN_FLAGS --sign "$SIGN_AS" "$f"
  done
  find "$APP/Contents/Frameworks" -maxdepth 1 -name "*.framework" -print0 | xargs -0 -I{} codesign --force $SIGN_FLAGS --sign "$SIGN_AS" {}
}
if [ -n "${SETU_SIGN_ID:-}" ]; then
  SIGN_AS="$SETU_SIGN_ID" SIGN_FLAGS="--timestamp --options runtime"
  sign_tree
  codesign --force --timestamp --options runtime --entitlements "$HERE/entitlements.plist" --sign "$SETU_SIGN_ID" "$APP"
  codesign --verify --deep --strict "$APP"
else
  echo "SETU_SIGN_ID is not set; the app gets an ad-hoc signature, so other Macs will ask before opening it"
  SIGN_AS="-" SIGN_FLAGS=""
  sign_tree
  codesign --force --sign - "$APP"
  codesign --verify --deep --strict "$APP" || echo "warning: the ad-hoc signature did not verify"
fi

step "Disk image"
mkdir -p "$OUT"
DMG="$OUT/Setu-$VERSION.dmg"
STAGE="$WORK/dmg"
rm -rf "$STAGE" "$DMG" "$WORK/rw.dmg"
mkdir -p "$STAGE/.background"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp "$HERE/dmg-background.png" "$STAGE/.background/background.png"
cp "$HERE/dmg-background@2x.png" "$STAGE/.background/background@2x.png"
hdiutil create -volname "Setu" -srcfolder "$STAGE" -fs HFS+ -format UDRW "$WORK/rw.dmg"
DEV="$(hdiutil attach -readwrite -noverify -noautoopen "$WORK/rw.dmg" | awk '/Apple_HFS/ {print $1}')"
osascript <<OSA || true
tell application "Finder"
  tell disk "Setu"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set bounds of container window to {200, 120, 840, 520}
    set viewOptions to the icon view options of container window
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to 96
    set background picture of viewOptions to file ".background:background.png"
    set position of item "Setu.app" of container window to {170, 190}
    set position of item "Applications" of container window to {470, 190}
    update without registering applications
    close
  end tell
end tell
OSA
hdiutil detach "$DEV"
hdiutil convert "$WORK/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
if [ -n "${SETU_SIGN_ID:-}" ]; then codesign --timestamp --sign "$SETU_SIGN_ID" "$DMG"; fi
if [ -n "${SETU_NOTARY_PROFILE:-}" ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$SETU_NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi
(cd "$OUT" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
echo "$DMG"
