#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG="$ROOT/build-macos.log"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "Run this on a Mac."
  exit 1
fi

if ! xcode-select -p >/dev/null 2>&1; then
  xcode-select --install >/dev/null 2>&1 || true
  echo "Finish installing the Xcode command line tools in the window that opened, then run this again."
  exit 1
fi

if ! command -v brew >/dev/null; then
  for b in /opt/homebrew/bin/brew /usr/local/bin/brew; do
    if [ -x "$b" ]; then
      eval "$("$b" shellenv)"
      break
    fi
  done
fi
if ! command -v brew >/dev/null; then
  echo "Homebrew is needed. Install it from https://brew.sh, then run this again."
  exit 1
fi

NEED=""
for tool in cmake ninja espeak-ng; do
  command -v "$tool" >/dev/null || NEED="$NEED $tool"
done
if [ -z "${QT_DIR:-}" ] && ! ls -d "$HOME"/Qt/6.*/macos >/dev/null 2>&1 && [ ! -x "$(brew --prefix qt)/bin/macdeployqt" ]; then
  NEED="$NEED qt"
fi
if ! python3 -c "import PIL" >/dev/null 2>&1 && ! command -v magick >/dev/null; then
  NEED="$NEED imagemagick"
fi
if [ -n "$NEED" ]; then
  echo "Installing$NEED with Homebrew"
  brew install $NEED
fi

export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v rustup >/dev/null; then
  echo "Installing Rust"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi
rustup toolchain install stable --profile minimal
export RUSTUP_TOOLCHAIN=stable

echo "Building Setu. The first build takes a while; the full log is in $LOG"
if "$ROOT/packaging/macos/build.sh" 2>&1 | tee "$LOG"; then
  DMG="$(ls -t "$ROOT"/dist/Setu-*.dmg | head -1)"
  echo
  echo "Done: $DMG"
  open -R "$DMG" || true
else
  echo
  echo "The build stopped. The full log is in $LOG"
  exit 1
fi
