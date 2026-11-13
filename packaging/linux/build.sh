#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/linux"
VERSION="$(sed -n 's/^project(Setu VERSION \([0-9.]*\).*/\1/p' "$ROOT/CMakeLists.txt")"
ARCH="$(uname -m)"
OUT="${OUT:-$ROOT/dist}"
WORK="$ROOT/build/package"
STAGE="$WORK/stage"
FORMATS=("$@")
[ ${#FORMATS[@]} -eq 0 ] && FORMATS=(bundle)
export PATH="$HOME/.cargo/bin:$PATH"
export SETU_PACKAGING=1
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/build/package/cargo}"
export SETU_BUILD_ID="${SETU_BUILD_ID:-$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || date -u +%Y%m%d)}"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || date +%s)}"

step() { printf '\n== %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

step "Dependencies"
"$ROOT/scripts/fetch-deps.sh"
"$ROOT/scripts/build-llama.sh"

step "Daemon and engines"
(cd "$ROOT/daemon" && cargo build --release --locked -p setu-daemon -p setu-engine -p setu-system -p setu-facecheck -p setu-pam)

step "Interface"
cmake -S "$ROOT" -B "$WORK/qt" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr -DSETU_BUILD_TESTS=OFF -DSETU_BUILD_ID="$SETU_BUILD_ID"
cmake --build "$WORK/qt"

step "Staging"
rm -rf "$STAGE"
DESTDIR="$STAGE" cmake --install "$WORK/qt"
install -Dm755 "$CARGO_TARGET_DIR/release/setu-daemon" "$STAGE/usr/bin/setu-daemon"
install -Dm755 "$CARGO_TARGET_DIR/release/setu-engine" "$STAGE/usr/bin/setu-engine"
install -Dm755 "$CARGO_TARGET_DIR/release/setu-system" "$STAGE/usr/libexec/setu/setu-system"
install -Dm755 "$CARGO_TARGET_DIR/release/setu-facecheck" "$STAGE/usr/libexec/setu/setu-facecheck"
install -Dm755 "$CARGO_TARGET_DIR/release/libpam_setu.so" "$STAGE/usr/libexec/setu/pam_setu.so"
if have checkmodule && have semodule_package; then
  checkmodule -M -m -o "$WORK/setu_face.mod" "$HERE/setu_face.te" >/dev/null
  semodule_package -o "$WORK/setu_face.pp" -m "$WORK/setu_face.mod"
  install -Dm644 "$WORK/setu_face.pp" "$STAGE/usr/libexec/setu/setu_face.pp"
else
  echo "checkmodule not found; the SELinux rules for face sign-in are left out"
fi
install -Dm644 "$HERE/ai.setu.system.policy" "$STAGE/usr/share/polkit-1/actions/ai.setu.system.policy"
mkdir -p "$STAGE/usr/lib/setu"
cp -P "$ROOT"/third_party/sherpa-onnx/lib/*.so* "$STAGE/usr/lib/setu/"
cp -P "$ROOT"/build/llama-cpp/bin/libllama.so* "$ROOT"/build/llama-cpp/bin/libggml*.so* "$ROOT"/build/llama-cpp/bin/libmtmd.so* "$STAGE/usr/lib/setu/"
mkdir -p "$STAGE/usr/share/setu"
cp -r "$ROOT/third_party/bundled/models" "$STAGE/usr/share/setu/"
find "$STAGE/usr/share/setu" -name .stamp -delete
install -Dm644 "$ROOT/third_party/bundled/benchmark.wav" "$STAGE/usr/share/setu/benchmark.wav"
install -Dm644 "$HERE/setu-daemon.service" "$STAGE/usr/lib/systemd/user/setu-daemon.service"
install -Dm644 "$HERE/setu-shell.service" "$STAGE/usr/lib/systemd/user/setu-shell.service"
install -Dm644 "$HERE/ai.setu.Setu.desktop" "$STAGE/usr/share/applications/ai.setu.Setu.desktop"
install -Dm644 "$HERE/ai.setu.Setu.metainfo.xml" "$STAGE/usr/share/metainfo/ai.setu.Setu.metainfo.xml"
install -Dm644 "$HERE/ai.setu.Setu.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/ai.setu.Setu.svg"
install -Dm644 "$WORK/qt/generated/licenses/third_party.json" "$STAGE/usr/share/licenses/setu/third_party.json"
install -Dm644 "$ROOT/assets/licenses/setu.txt" "$STAGE/usr/share/licenses/setu/LICENSE"
cp "$ROOT"/assets/fonts/OFL-*.txt "$STAGE/usr/share/licenses/setu/"
install -Dm644 "$ROOT/README.md" "$STAGE/usr/share/doc/setu/README.md"
install -Dm644 "$HERE/70-setu-uinput.rules" "$STAGE/usr/lib/udev/rules.d/70-setu-uinput.rules"
install -Dm644 "$HERE/setu-uinput.conf" "$STAGE/usr/lib/modules-load.d/setu-uinput.conf"
strip --strip-unneeded "$STAGE"/usr/bin/setu-* "$STAGE"/usr/libexec/setu/setu-system "$STAGE"/usr/libexec/setu/setu-facecheck "$STAGE"/usr/libexec/setu/pam_setu.so "$STAGE"/usr/lib/setu/*.so* 2>/dev/null || true
if readelf -d "$STAGE/usr/libexec/setu/setu-facecheck" | grep -q "$ROOT"; then
  echo "setu-facecheck still carries a build-tree rpath; rebuild with SETU_PACKAGING=1" >&2
  exit 1
fi
for b in setu-daemon setu-engine; do
  if readelf -d "$STAGE/usr/bin/$b" | grep -q "$ROOT"; then
    echo "$b still carries a build-tree rpath; rebuild with SETU_PACKAGING=1" >&2
    exit 1
  fi
done
mkdir -p "$OUT"

package_rpm() {
  have rpmbuild || { echo "rpmbuild not found; skipping .rpm"; return; }
  rpmbuild -bb "$HERE/setu.spec" --define "_topdir $WORK/rpm" --define "stage $STAGE" --define "version $VERSION" --define "_rpmdir $OUT" --define "_build_name_fmt %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm"
}

package_bundle() {
  local payload="$WORK/bundle-payload.tar.xz" header="$WORK/bundle-header.sh" root="$WORK/bundle"
  rm -rf "$root" && cp -a "$STAGE" "$root"
  install -Dm755 "$HERE/bundle-uninstall.sh" "$root/usr/lib/setu/uninstall"
  tar -C "$root" --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner -cf - ./usr | xz -T0 -9 > "$payload"
  local lines; lines=$(wc -l < "$HERE/bundle.sh")
  local qt glibc; qt=$(qtpaths6 --qt-version 2>/dev/null | cut -d. -f1,2); glibc=$(ldd --version | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')
  sed -e "s/@VERSION@/$VERSION/" -e "s/@SKIP@/$((lines + 1))/" -e "s/@NEEDS@/Qt ${qt:-6.8}.x and glibc ${glibc:-2.35} or newer/" "$HERE/bundle.sh" > "$header"
  [ "$(wc -l < "$header")" -eq "$lines" ] || { echo "bundle header changed length" >&2; exit 1; }
  cat "$header" "$payload" > "$OUT/Setu-$VERSION-$ARCH.bundle"
  chmod 755 "$OUT/Setu-$VERSION-$ARCH.bundle"
}

for f in "${FORMATS[@]}"; do
  step "Package: $f"
  case "$f" in
    bundle) package_bundle ;;
    rpm) package_rpm ;;
    flatpak) flatpak-builder --force-clean --repo="$WORK/flatpak-repo" "$WORK/flatpak" "$HERE/flatpak/ai.setu.Setu.yml" && flatpak build-bundle "$WORK/flatpak-repo" "$OUT/Setu-$VERSION.flatpak" ai.setu.Setu ;;
    *) echo "unknown format $f" >&2; exit 2 ;;
  esac
done

step "Checksums and SBOM"
(cd "$OUT" && find . -maxdepth 1 -type f ! -name SHA256SUMS ! -name '*.asc' ! -name '*.json' -printf '%f\0' | sort -z | xargs -0 -r sha256sum > SHA256SUMS)
python3 -I "$ROOT/scripts/sbom.py" --licenses "$WORK/qt/generated/licenses/third_party.json" --version "$VERSION" --out "$OUT/setu-$VERSION.spdx.json"
if [ -n "${SETU_GPG_KEY:-}" ]; then
  gpg --batch --yes --local-user "$SETU_GPG_KEY" --armor --detach-sign "$OUT/SHA256SUMS"
fi
ls -la "$OUT"
