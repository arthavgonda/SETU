#!/bin/sh
set -eu

MANIFEST=/usr/lib/setu/bundle-files

[ "$(id -u)" -eq 0 ] || exec pkexec "$(readlink -f "$0")" "$@"
[ -f "$MANIFEST" ] || { echo "Setu: nothing to remove" >&2; exit 1; }

if [ "${1:-}" != "--yes" ] && [ -t 0 ]; then
  printf 'Remove Setu from this computer? [y/N] '
  read -r reply || exit 1
  case "$reply" in y|Y|yes|Yes|YES) ;; *) exit 1 ;; esac
fi

if command -v systemctl >/dev/null 2>&1; then
  systemctl --global disable setu-daemon.service setu-shell.service >/dev/null 2>&1 || :
fi
/usr/libexec/setu/setu-system --uninstall >/dev/null 2>&1 || :

while IFS= read -r f; do
  case "$f" in /usr/*) rm -f "$f" ;; esac
done < "$MANIFEST"

for d in /usr/lib/setu /usr/libexec/setu /usr/share/setu /usr/share/licenses/setu /usr/share/doc/setu; do
  [ -d "$d" ] && find "$d" -depth -type d -empty -delete
done
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules >/dev/null 2>&1 || :
fi
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :

echo "Setu has been removed. Your settings and voice data in your home folder were kept."
