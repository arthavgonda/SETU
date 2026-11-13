#!/bin/sh
set -eu

VERSION="@VERSION@"
SKIP=@SKIP@
LIB=/usr/lib/setu
MANIFEST="$LIB/bundle-files"
ASSUME_YES=0
ACTION=install

say() { printf '%s\n' "$*"; }
die() { printf 'Setu: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
  say "Setu $VERSION installer"
  say ""
  say "  sh $(basename "$0")              install or upgrade Setu"
  say "  sh $(basename "$0") --uninstall  remove Setu"
  say "  sh $(basename "$0") --yes        do not ask before installing dependencies"
  say "  sh $(basename "$0") --extract D  unpack the files into folder D without installing"
}

while [ $# -gt 0 ]; do
  case "$1" in
    -y|--yes) ASSUME_YES=1 ;;
    --uninstall) ACTION=uninstall ;;
    --extract) ACTION=extract; shift; [ $# -gt 0 ] || die "--extract needs a folder"; EXTRACT_TO="$1" ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

payload() { tail -n +"$SKIP" "$SELF" | tar -xJf - "$@"; }

SELF="$(readlink -f "$0")"

if [ "$ACTION" = extract ]; then
  mkdir -p "$EXTRACT_TO"
  payload -C "$EXTRACT_TO" --no-same-owner
  say "Unpacked Setu $VERSION into $EXTRACT_TO"
  exit 0
fi

[ "$ACTION" = uninstall ] || {
  if have rpm && rpm -qf /usr/bin/setu-daemon >/dev/null 2>&1; then
    die "Setu is already installed as the $(rpm -qf --qf '%{NAME}' /usr/bin/setu-daemon) package. Remove it first (sudo dnf remove setu), then run this again."
  fi
  if have dpkg && dpkg -S /usr/bin/setu-daemon >/dev/null 2>&1; then
    die "Setu is already installed as a .deb package. Remove it first (sudo apt remove setu), then run this again."
  fi
}

if [ "$(id -u)" -ne 0 ]; then
  ARGS=""
  [ "$ACTION" = uninstall ] && ARGS="--uninstall"
  [ "$ASSUME_YES" = 1 ] && ARGS="$ARGS --yes"
  say "Setu needs administrator rights to install into /usr."
  if have sudo; then
    exec sudo sh "$SELF" $ARGS
  elif have pkexec; then
    exec pkexec sh "$SELF" $ARGS
  fi
  die "run this again as root"
fi

if [ "$ACTION" = uninstall ]; then
  [ -x "$LIB/uninstall" ] || die "Setu was not installed with this installer"
  if [ "$ASSUME_YES" = 1 ]; then exec "$LIB/uninstall" --yes; fi
  exec "$LIB/uninstall"
fi

[ "$(uname -m)" = x86_64 ] || die "this installer is for x86_64 computers"
[ "$(uname -s)" = Linux ] || die "this installer is for Linux"

ask() {
  [ "$ASSUME_YES" = 1 ] && return 0
  [ -t 0 ] || return 0
  printf '%s [Y/n] ' "$1"
  read -r reply || return 0
  case "$reply" in n|N|no|No|NO) return 1 ;; esac
  return 0
}

deb_installed() { dpkg -s "$1" 2>/dev/null | grep -q '^Status: install ok installed'; }

missing_packages() {
  case "$1" in
    dnf)
      for p in qt6-qtbase qt6-qtbase-gui qt6-qtdeclarative qt6-qtsvg pulseaudio-libs espeak-ng libsecret xcb-util layer-shell-qt xz; do
        rpm -q "$p" >/dev/null 2>&1 || printf '%s ' "$p"
      done ;;
    apt)
      apt-get update -qq >/dev/null 2>&1 || :
      for p in libqt6core6 libqt6gui6 libqt6network6 libqt6dbus6 libqt6qml6 libqt6quick6 libqt6svg6 qt6-qpa-plugins qml6-module-qtquick qml6-module-qtquick-layouts qml6-module-qtquick-templates qml6-module-qtquick-controls qml6-module-qtquick-dialogs qml6-module-qtquick-window qml6-module-qt-labs-platform qml6-module-qtqml-workerscript qml6-module-qtqml-models libpulse0 espeak-ng libsecret-1-0 libxcb-shape0 liblayershellqtinterface6 xz-utils; do
        deb_installed "$p" || deb_installed "${p}t64" || {
          if apt-cache policy "${p}t64" 2>/dev/null | grep -q "Candidate: [^(]"; then printf '%s ' "${p}t64"; else printf '%s ' "$p"; fi
        }
      done ;;
    pacman)
      for p in qt6-base qt6-declarative qt6-svg libpulse espeak-ng libsecret xcb-util layer-shell-qt xz; do
        pacman -Q "$p" >/dev/null 2>&1 || printf '%s ' "$p"
      done ;;
  esac
}

install_packages() {
  case "$1" in
    dnf) dnf install -y $2 ;;
    apt) DEBIAN_FRONTEND=noninteractive apt-get install -y $2 ;;
    pacman) pacman -S --needed --noconfirm $2 ;;
  esac
}

PM=""
if have dnf; then PM=dnf; elif have apt-get && have dpkg; then PM=apt; elif have pacman; then PM=pacman; fi

if [ -n "$PM" ]; then
  NEED="$(missing_packages "$PM")"
  if [ -n "$NEED" ]; then
    say "Setu needs these packages: $NEED"
    if ask "Install them now with $PM?"; then
      install_packages "$PM" "$NEED" || say "Some packages could not be installed; Setu may not start until they are."
    else
      say "Skipping the packages; Setu may not start until they are installed."
    fi
  fi
fi

have xz || die "xz is needed to unpack Setu. Install it (the xz or xz-utils package) and run this again."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/root"
payload -C "$WORK/root" --no-same-owner

if have ldd; then
  BROKEN="$(for b in "$WORK"/root/usr/bin/setu-*; do ldd "$b" 2>&1; done | grep -E 'not found' | sed "s#$WORK/root##g" | sort -u || :)"
  if [ -n "$BROKEN" ]; then
    say ""
    say "Setu $VERSION cannot run on this system yet. It needs @NEEDS@, and these are missing or too old:"
    say "$BROKEN"
    die "nothing was installed"
  fi
fi

say "Installing Setu $VERSION"
(cd "$WORK/root" && find ./usr ! -type d) | sed 's#^\./#/#' | sort > "$WORK/new"

if have systemctl; then
  systemctl --global disable setu-daemon.service setu-shell.service >/dev/null 2>&1 || :
fi

if [ -f "$MANIFEST" ]; then
  sort "$MANIFEST" | comm -23 - "$WORK/new" | while IFS= read -r f; do
    rm -f "$f"
  done
fi

tar -C "$WORK/root" -cf - ./usr | tar -C / -xf - --no-same-owner --no-overwrite-dir
printf '/usr/lib/setu/bundle-files\n' >> "$WORK/new"
sort -u "$WORK/new" > "$MANIFEST"
chmod 644 "$MANIFEST"
chmod 755 "$LIB/uninstall"

if have restorecon; then
  restorecon -R /usr/bin/setu-daemon /usr/bin/setu-engine /usr/bin/setu-shell /usr/bin/setu-ui /usr/libexec/setu /usr/lib/setu /usr/share/setu >/dev/null 2>&1 || :
fi
if have systemctl; then
  systemctl --global enable setu-daemon.service setu-shell.service >/dev/null 2>&1 || :
  systemctl try-restart setu-system.service >/dev/null 2>&1 || :
fi
modprobe uinput >/dev/null 2>&1 || :
if have udevadm; then
  udevadm control --reload-rules >/dev/null 2>&1 || :
  udevadm trigger --sysname-match=uinput >/dev/null 2>&1 || :
fi
have update-desktop-database && update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
have gtk-update-icon-cache && gtk-update-icon-cache -q /usr/share/icons/hicolor >/dev/null 2>&1 || :

say ""
say "Setu $VERSION is installed. Log out and back in, or open Setu from your app menu."
say "To remove it later: sudo /usr/lib/setu/uninstall"
exit 0
