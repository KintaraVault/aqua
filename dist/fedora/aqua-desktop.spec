# Fedora / RHEL-family package for Aqua.
#
# Build from a checkout (network is needed for the crates and the Smithay git dependency):
#   dist/fedora/build-rpm.sh            # tarball + rpmbuild -ba, result in ~/rpmbuild/RPMS
# Offline / mock builds: put a `cargo vendor` tarball next to the sources as
# aqua-desktop-%%{version}-vendor.tar.xz (dist/fedora/build-rpm.sh --vendor creates it).
%bcond vendor 0
%if %{with vendor}
%global cargo_net --offline
%else
%global cargo_net %{nil}
%endif

Name:           aqua-desktop
Version:        0.1.0
Release:        1%{?dist}
Summary:        macOS-style (Liquid Glass) Wayland compositor and desktop shell

License:        MIT
URL:            https://github.com/aqua-desktop/aqua
Source0:        %{name}-%{version}.tar.gz
%if %{with vendor}
Source1:        %{name}-%{version}-vendor.tar.xz
%endif

ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo >= 1.88
BuildRequires:  rust >= 1.88
BuildRequires:  gcc
BuildRequires:  clang-devel
BuildRequires:  pkgconf-pkg-config
BuildRequires:  pkgconfig(libinput)
BuildRequires:  pkgconfig(libudev)
BuildRequires:  pkgconfig(libseat)
BuildRequires:  pkgconfig(gbm)
BuildRequires:  pkgconfig(egl)
BuildRequires:  pkgconfig(libdrm)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(wayland-server)
BuildRequires:  pkgconfig(libpipewire-0.3)
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  pkgconfig(freetype2)
BuildRequires:  pkgconfig(dbus-1)
BuildRequires:  pam-devel

Requires:       libinput
Requires:       libxkbcommon
Requires:       mesa-libEGL
Requires:       mesa-libgbm
Requires:       libdrm
Requires:       libseat
Requires:       pam
Requires:       dbus
Requires:       fontconfig
Requires:       polkit
Requires:       xkeyboard-config
Requires:       xorg-x11-server-Xwayland
Requires:       xdg-desktop-portal
Requires:       xdg-desktop-portal-gtk
Requires:       xdg-utils
Requires:       pipewire
Requires:       wireplumber
Recommends:     pipewire-pulseaudio
Recommends:     NetworkManager
Recommends:     bluez
Recommends:     upower
Recommends:     google-noto-emoji-color-fonts
Recommends:     sound-theme-freedesktop
Recommends:     (ffmpeg-free or ffmpeg)
Recommends:     flatpak
Recommends:     polkit
Suggests:       greetd
Suggests:       cage
Suggests:       brightnessctl
Suggests:       pciutils
Suggests:       fcitx5
Suggests:       gnome-calculator

%description
Aqua is a Wayland compositor and desktop shell written in Rust on top of Smithay.
It recreates the macOS "Liquid Glass" look: menu bar, Dock, Control Center,
Launchpad, Spotlight, Mission Control and Spaces, rounded windows with shader
shadows, plus its own Finder, System Settings, polkit agent and greetd greeter.

%prep
%autosetup -n %{name}-%{version}
%if %{with vendor}
tar -xf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<'CFG'
[source.crates-io]
replace-with = "vendored-sources"

[source."git+https://github.com/Smithay/smithay.git?rev=118e34ffc9b99854a2230c4805e1f37dc9029edb"]
git = "https://github.com/Smithay/smithay.git"
rev = "118e34ffc9b99854a2230c4805e1f37dc9029edb"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CFG
%endif

%build
export CARGO_TARGET_DIR=%{_builddir}/%{name}-%{version}/target
cargo build --release --locked %{cargo_net} -p aqua-compositor -p aqua-ui

%install
dist/stage-install.sh %{buildroot} --prefix %{_prefix} --target target/release --pam system-auth
# shipped by stage-install.sh, packaged via %%license / %%doc below
rm -rf %{buildroot}%{_datadir}/licenses/%{name} %{buildroot}%{_docdir}/%{name}

%check
export CARGO_TARGET_DIR=%{_builddir}/%{name}-%{version}/target
cargo test --release --locked %{cargo_net} -p aqua-i18n -p aqua-config -p aqua-wm -p aqua-apps --lib

%posttrans
for f in /usr/local/bin/aqua /usr/local/bin/aqua-session; do
    [ -e "$f" ] && echo "warning: $f (from dist/install.sh) shadows %{_bindir} — run: sudo ./dist/install.sh --uninstall" || :
done

%files
%license LICENSE
%doc README.md
%{_bindir}/aqua
%{_bindir}/aqua-session
%{_bindir}/aqua-screenshot
%{_bindir}/aqua-settings
%{_bindir}/aqua-finder
%{_bindir}/aqua-store
%{_bindir}/aqua-filechooser
%{_bindir}/aqua-polkit-agent
%{_bindir}/aqua-greeter
%{_datadir}/wayland-sessions/aqua.desktop
%{_datadir}/xdg-desktop-portal/portals/aqua.portal
%{_datadir}/xdg-desktop-portal/aqua-portals.conf
%{_datadir}/applications/aqua-screenshot.desktop
%{_datadir}/applications/org.aqua.finder.desktop
%{_datadir}/applications/org.aqua.store.desktop
%config(noreplace) %{_sysconfdir}/xdg/autostart/org.aqua.store-updates.desktop
%{_datadir}/aqua/
%config(noreplace) %{_sysconfdir}/pam.d/aqua

%changelog
* Sat Oct 03 2026 Aqua contributors - 0.1.0-1
- Initial package
