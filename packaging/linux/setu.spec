Name:           setu
Version:        %{version}
Release:        1%{?dist}
Summary:        Hands-free voice control for this computer
License:        LicenseRef-Setu
URL:            https://setu.invalid
AutoReqProv:    no
Requires:       qt6-qtbase >= 6.8, qt6-qtbase-gui >= 6.8, qt6-qtdeclarative >= 6.8, qt6-qtsvg >= 6.8
Requires:       pulseaudio-libs, espeak-ng, libsecret, xcb-util
Recommends:     gnome-keyring, vulkan-loader
%global debug_package %{nil}
%global __strip /bin/true

%description
Say "Hey Setu", then what you want. Speech recognition, the language model
and the voice run on this computer. Nothing is processed before the wake
phrase and audio is never stored.

%install
mkdir -p %{buildroot}
cp -a %{stage}/. %{buildroot}/

%post
systemctl --global enable setu-daemon.service setu-shell.service >/dev/null 2>&1 || :
modprobe uinput >/dev/null 2>&1 || :
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --sysname-match=uinput >/dev/null 2>&1 || :

systemctl try-restart setu-system.service >/dev/null 2>&1 || :

%preun
if [ $1 -eq 0 ]; then
  systemctl --global disable setu-daemon.service setu-shell.service >/dev/null 2>&1 || :
  /usr/libexec/setu/setu-system --uninstall >/dev/null 2>&1 || :
fi

%files
/usr/bin/setu-daemon
/usr/bin/setu-engine
/usr/bin/setu-shell
/usr/bin/setu-ui
/usr/libexec/setu
/usr/share/polkit-1/actions/ai.setu.system.policy
/usr/lib/setu
/usr/share/setu
/usr/lib/systemd/user/setu-daemon.service
/usr/lib/systemd/user/setu-shell.service
/usr/share/applications/ai.setu.Setu.desktop
/usr/share/metainfo/ai.setu.Setu.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/ai.setu.Setu.svg
/usr/share/licenses/setu
/usr/share/doc/setu
/usr/lib/udev/rules.d/70-setu-uinput.rules
/usr/lib/modules-load.d/setu-uinput.conf
