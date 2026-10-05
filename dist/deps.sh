# shellcheck shell=sh
# Dependency installation for dist/install.sh (sourced; uses step/ok/warn/die/root/as_user).
#
# Package lists per distribution family. "a|b" = alternatives, the first one the
# repositories have is used (package names change between releases). Every name is checked
# with the package manager before installing, so a package that a release does not have
# (Debian has no "seatd" dev split before 12, Fedora renamed the emoji font …) is skipped
# with a note instead of aborting the transaction.

# ------------------------------------------------------------------ package lists
ARCH_BUILD="rust|rustup gcc clang pkgconf git libinput seatd mesa libglvnd libdrm libxkbcommon wayland pipewire fontconfig freetype2 dbus pam systemd-libs"
ARCH_RUN="xorg-xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-utils wireplumber pipewire-pulse polkit xkeyboard-config"
ARCH_REC="noto-fonts noto-fonts-emoji networkmanager bluez upower sound-theme-freedesktop ffmpeg flatpak"
ARCH_NVIDIA="egl-wayland libva-nvidia-driver"

DEB_BUILD="ca-certificates curl git gcc clang libclang-dev pkgconf|pkg-config libinput-dev libudev-dev libseat-dev libgbm-dev libegl-dev|libegl1-mesa-dev libdrm-dev libxkbcommon-dev libwayland-dev libpipewire-0.3-dev libfontconfig-dev|libfontconfig1-dev libfreetype-dev|libfreetype6-dev libdbus-1-dev libpam0g-dev"
DEB_RUN="dbus fontconfig libpam-runtime polkitd|policykit-1 xkb-data xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-utils pipewire wireplumber pipewire-pulse libgl1 libegl1 libgles2"
DEB_REC="fonts-noto-color-emoji fonts-noto-core network-manager bluez upower sound-theme-freedesktop ffmpeg flatpak"
DEB_NVIDIA="libnvidia-egl-wayland1 nvidia-vaapi-driver"

FED_BUILD="ca-certificates curl git gcc clang-devel pkgconf-pkg-config libinput-devel systemd-devel libseat-devel mesa-libgbm-devel mesa-libEGL-devel libdrm-devel libxkbcommon-devel wayland-devel pipewire-devel fontconfig-devel freetype-devel dbus-devel pam-devel"
FED_RUN="dbus fontconfig pam polkit xkeyboard-config xorg-x11-server-Xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-utils pipewire wireplumber pipewire-pulseaudio mesa-libEGL mesa-libgbm libglvnd-gles"
FED_REC="google-noto-color-emoji-fonts|google-noto-emoji-color-fonts google-noto-sans-symbols-2-fonts|google-noto-sans-symbols2-fonts NetworkManager bluez upower sound-theme-freedesktop ffmpeg-free|ffmpeg flatpak"
FED_NVIDIA="egl-wayland libva-nvidia-driver"

SUSE_BUILD="ca-certificates curl git gcc clang-devel pkgconf libinput-devel systemd-devel libseat-devel libgbm-devel Mesa-libEGL-devel libdrm-devel libxkbcommon-devel wayland-devel pipewire-devel fontconfig-devel freetype2-devel dbus-1-devel pam-devel"
SUSE_RUN="dbus-1 fontconfig pam polkit xkeyboard-config xwayland xdg-desktop-portal xdg-desktop-portal-gtk xdg-utils pipewire wireplumber pipewire-pulseaudio"
SUSE_REC="google-noto-coloremoji-fonts NetworkManager bluez upower sound-theme-freedesktop ffmpeg flatpak"
SUSE_NVIDIA=""

# --------------------------------------------------------------------- detection
pkg_family() {
    if command -v pacman >/dev/null 2>&1; then echo arch
    elif command -v apt-get >/dev/null 2>&1 && command -v dpkg-query >/dev/null 2>&1; then echo debian
    elif command -v dnf >/dev/null 2>&1 || command -v dnf5 >/dev/null 2>&1; then echo fedora
    elif command -v zypper >/dev/null 2>&1; then echo suse
    else echo unknown
    fi
}

nvidia_driver_present() {
    [ -d /sys/module/nvidia ] && return 0
    command -v modinfo >/dev/null 2>&1 && modinfo nvidia >/dev/null 2>&1
}

DNF=dnf
command -v dnf >/dev/null 2>&1 || DNF=dnf5

# is package $1 installed?
pkg_installed() {
    case $FAMILY in
        arch) pacman -Qq "$1" >/dev/null 2>&1 ;;
        debian) [ "$(dpkg-query -W -f '${db:Status-Status}' "$1" 2>/dev/null)" = installed ] ;;
        fedora|suse) rpm -q "$1" >/dev/null 2>&1 || rpm -q --whatprovides "$1" >/dev/null 2>&1 ;;
        *) return 1 ;;
    esac
}

# do the repositories have package $1?
pkg_available() {
    case $FAMILY in
        arch) pacman -Si "$1" >/dev/null 2>&1 || pacman -Sg "$1" >/dev/null 2>&1 ;;
        debian)
            c=$(apt-cache policy "$1" 2>/dev/null | sed -n 's/^ *Candidate: *//p' | head -n 1)
            [ -n "$c" ] && [ "$c" != "(none)" ] ;;
        fedora) [ -n "$($DNF -q repoquery --available --qf '%{name}\n' "$1" 2>/dev/null | head -n 1)" ] ;;
        suse) zypper -q --non-interactive search -x --match-exact "$1" >/dev/null 2>&1 ;;
        *) return 1 ;;
    esac
}

# resolve a list of "a|b" groups to installable names; sets PKGS, prints skipped ones
resolve_pkgs() { # optional(0/1) groups...
    optional=$1; shift
    for group in "$@"; do
        chosen=
        IFS_SAVE=$IFS; IFS='|'
        # shellcheck disable=SC2086
        set -- $group
        IFS=$IFS_SAVE
        for p in "$@"; do
            if pkg_installed "$p"; then chosen=-; break; fi
        done
        if [ -z "$chosen" ]; then
            for p in "$@"; do
                if pkg_available "$p"; then chosen=$p; break; fi
            done
        fi
        case $chosen in
            -) ;;
            '') if [ "$optional" -eq 1 ]; then SKIPPED="$SKIPPED $group"; else MISSING="$MISSING $group"; fi ;;
            *) PKGS="$PKGS $chosen" ;;
        esac
    done
}

refresh_index() {
    case $FAMILY in
        # no -Sy on Arch: refreshing without upgrading is a partial upgrade
        arch) ;;
        debian) root env DEBIAN_FRONTEND=noninteractive apt-get update -q >/dev/null 2>&1 || warn "apt-get update failed" ;;
        fedora) root $DNF -q makecache >/dev/null 2>&1 || warn "$DNF makecache failed" ;;
        suse) root zypper -q --non-interactive refresh >/dev/null 2>&1 || warn "zypper refresh failed" ;;
    esac
}

install_pkgs() { # names...
    [ $# -gt 0 ] || return 0
    case $FAMILY in
        arch) root pacman -S --needed --noconfirm "$@" || { warn "if packages were not found (404), update first: sudo pacman -Syu"; return 1; } ;;
        debian) root env DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends "$@" ;;
        fedora) root $DNF install -y --setopt=install_weak_deps=False "$@" ;;
        suse) root zypper --non-interactive install --no-recommends "$@" ;;
    esac
}

install_dependencies() {
    FAMILY=$(pkg_family)
    step "Installing dependencies ($FAMILY)"
    case $FAMILY in
        arch) B=$ARCH_BUILD; R=$ARCH_RUN; C=$ARCH_REC; N=$ARCH_NVIDIA ;;
        debian) B=$DEB_BUILD; R=$DEB_RUN; C=$DEB_REC; N=$DEB_NVIDIA ;;
        fedora) B=$FED_BUILD; R=$FED_RUN; C=$FED_REC; N=$FED_NVIDIA ;;
        suse) B=$SUSE_BUILD; R=$SUSE_RUN; C=$SUSE_REC; N=$SUSE_NVIDIA ;;
        *) warn "unknown package manager — install the build dependencies listed in README.md by hand"; return 0 ;;
    esac
    # a usable rustup already provides cargo: do not pull the distro's (possibly old) rust
    if user_has_rustup; then
        B=$(printf '%s\n' $B | grep -v -E '^(rust|cargo|rustc)(\||$)' | tr '\n' ' ')
    fi
    [ "$BUILD" = no ] && B=""
    refresh_index
    PKGS=; MISSING=; SKIPPED=
    # shellcheck disable=SC2086
    resolve_pkgs 0 $B $R
    # shellcheck disable=SC2086
    resolve_pkgs 1 $C
    if nvidia_driver_present && [ -n "$N" ]; then
        # shellcheck disable=SC2086
        resolve_pkgs 1 $N
    fi
    [ -n "$MISSING" ] && warn "not in your repositories (skipped):$MISSING"
    [ -n "$SKIPPED" ] && warn "optional, not available here:$SKIPPED"
    if [ -n "$PKGS" ]; then
        echo "  installing:$PKGS"
        # shellcheck disable=SC2086
        install_pkgs $PKGS || die "package installation failed (see above)"
        ok "packages installed"
    else
        ok "all dependencies already installed"
    fi
}

# ------------------------------------------------------------------- Rust toolchain
rust_required() {
    sed -n 's/^rust-version *= *"\([0-9.]*\)".*/\1/p' "$SRC/Cargo.toml" | head -n 1
}

# version_ge A B : A >= B (dotted numbers)
version_ge() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -t. -k1,1n -k2,2n -k3,3n | head -n 1)" = "$2" ]
}

user_sh() { as_user sh -lc "{ [ -f \"\$HOME/.cargo/env\" ] && . \"\$HOME/.cargo/env\"; true; }; $1"; }

user_has_rustup() { user_sh 'command -v rustup' >/dev/null 2>&1; }

user_rustc_version() {
    user_sh 'rustc --version' 2>/dev/null | sed -n 's/^rustc \([0-9][0-9.]*\).*/\1/p'
}

# Debian/Ubuntu ship newer toolchains as cargo-1.xx / rustc-1.xx
versioned_cargo() {
    need=$1
    for c in $(ls /usr/bin/cargo-1.* 2>/dev/null | sort -t. -k2,2n -r); do
        v=${c#/usr/bin/cargo-}
        if version_ge "$v" "$need" && [ -x "/usr/bin/rustc-$v" ]; then echo "$v"; return 0; fi
    done
    return 1
}

# rustc version the repositories offer (empty if unknown)
distro_rust_version() {
    case ${FAMILY:-} in
        arch) pacman -Si rust 2>/dev/null | sed -n 's/^Version *: *\([0-9:]*\)\{0,1\}\([0-9][0-9.]*\).*/\2/p' | head -n 1 ;;
        debian) apt-cache policy rustc 2>/dev/null | sed -n 's/^ *Candidate: *//p' | sed -n 's/^\([0-9]*:\)\{0,1\}\([0-9][0-9.]*\).*/\2/p' | head -n 1 ;;
        fedora) $DNF -q repoquery --available --latest-limit=1 --qf '%{version}\n' rust 2>/dev/null | head -n 1 ;;
        suse) zypper -q --non-interactive info rust 2>/dev/null | sed -n 's/^Version *: *\([0-9][0-9.]*\).*/\1/p' | head -n 1 ;;
    esac
}

ensure_rust() {
    need=$(rust_required)
    [ -n "$need" ] || need=1.88
    have=$(user_rustc_version)
    if [ -n "$have" ] && version_ge "$have" "$need"; then
        ok "Rust $have (needs $need)"
        return 0
    fi
    if v=$(versioned_cargo "$need"); then
        CARGO_BIN="env RUSTC=/usr/bin/rustc-$v /usr/bin/cargo-$v"
        ok "Rust $v (cargo-$v)"
        return 0
    fi
    # the distribution's toolchain when it is new enough (Fedora, Arch, Debian testing …)
    if [ "$DEPS" -eq 1 ] && ! user_has_rustup; then
        FAMILY=${FAMILY:-$(pkg_family)}
        dv=$(distro_rust_version)
        if [ -n "$dv" ] && version_ge "$dv" "$need"; then
            step "Installing the distribution's Rust $dv"
            case $FAMILY in
                arch) install_pkgs rust ;;
                debian) install_pkgs cargo rustc ;;
                fedora|suse) install_pkgs cargo rust ;;
            esac || warn "could not install the distribution's Rust"
            have=$(user_rustc_version)
            if [ -n "$have" ] && version_ge "$have" "$need"; then
                ok "Rust $have"
                return 0
            fi
        fi
    fi
    if [ -n "$have" ]; then
        step "Rust $have is older than $need — installing the stable toolchain with rustup"
    else
        step "Rust not found — installing the stable toolchain (Rust >= $need needed) with rustup"
    fi
    if ! user_has_rustup; then
        FAMILY=${FAMILY:-$(pkg_family)}
        if [ "$DEPS" -eq 1 ] && pkg_available rustup && ! { [ "$FAMILY" = arch ] && pkg_installed rust; }; then
            install_pkgs rustup >/dev/null 2>&1 || true
        fi
    fi
    if user_has_rustup; then
        user_sh 'rustup toolchain install stable --profile minimal && rustup default stable' \
            || die "rustup could not install the stable toolchain"
    else
        command -v curl >/dev/null 2>&1 || die "curl is needed to install rustup"
        user_sh 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path' \
            || die "rustup installation failed"
    fi
    have=$(user_rustc_version)
    if [ -z "$have" ] || ! version_ge "$have" "$need"; then
        die "Rust $need or newer is needed (found ${have:-none}); install it with rustup and re-run"
    fi
    ok "Rust $have (rustup)"
}
