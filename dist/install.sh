#!/bin/sh
# Aqua installer / updater.
#
#   ./dist/install.sh              build (if needed) and install to /usr/local, asks for sudo
#   sudo ./dist/install.sh         same; the build runs as $SUDO_USER, never as root
#
# Options:
#   --prefix DIR     install prefix (default /usr/local; /usr when installed by the PKGBUILD)
#   --no-build       do not run cargo, install what is in target/release
#   --rebuild        force relinking the Aqua binaries
#   --no-reload      do not touch the running session
#   --no-deps        do not install build/runtime dependencies with the package manager
#   --deps-only      only install the dependencies (and the Rust toolchain), then exit
#   --no-nvidia      do not configure NVIDIA kernel modesetting
#   --uninstall      remove everything this script installs (any prefix)
#   -h, --help
#
# Dependencies (Arch, Debian/Ubuntu, Fedora; openSUSE best effort): every package is
# checked against the distribution's repositories first, so names that do not exist on a
# given release are skipped instead of failing the whole transaction. A Rust toolchain
# older than the workspace's rust-version (e.g. Debian 13's rustc 1.85) is replaced by
# rustup (the distribution's rustup package, else rustup.rs) for the building user.
#
# NVIDIA: drivers before 560 (the 550 LTS series in Debian 13 / Ubuntu 24.04 …) do not
# enable nvidia_drm.modeset, which every Wayland compositor needs, and the desktop has no
# rights to change boot settings. The installer runs aqua-nvidia-setup as root: modprobe.d
# option, kernel command line (GRUB / grubby / systemd-boot) and initramfs; reboot after.
#
# Safe for updates on top of an existing install:
#   * always runs cargo first and installs the fresh target/release binaries (never a
#     stale target-ui/ copy);
#   * removes older copies of Aqua binaries from the other prefixes (/usr/bin,
#     /usr/local/bin, ~/.cargo/bin, ~/.local/bin) that would otherwise shadow the new ones
#     in $PATH — the classic "the settings changes do nothing" symptom;
#   * replaces files atomically (works while Aqua is running: no "Text file busy");
#   * replaces the font directory instead of merging, refreshes font/desktop caches;
#   * restarts helper processes of the running session (System Settings, file chooser,
#     polkit agent), asks the running compositor to reload its configuration and tells you
#     when a re-login is needed for the new compositor binary.
set -eu

PREFIX=/usr/local
BUILD=yes
RELOAD=1
UNINSTALL=0
REBUILD=0
DEPS=1
DEPS_ONLY=0
NVIDIA=1
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX=${2:?--prefix needs a directory}; shift ;;
        --prefix=*) PREFIX=${1#--prefix=} ;;
        --no-build) BUILD=no ;;
        --rebuild) BUILD=yes; REBUILD=1 ;;
        --no-reload) RELOAD=0 ;;
        --uninstall) UNINSTALL=1 ;;
        --no-deps) DEPS=0 ;;
        --deps-only) DEPS_ONLY=1 ;;
        --no-nvidia) NVIDIA=0 ;;
        -h|--help) sed -n '2,42p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
    esac
    shift
done

cd "$(dirname "$0")/.."
SRC=$(pwd)
[ "$REBUILD" -eq 1 ] && touch crates/aqua-compositor/src/main.rs crates/aqua-ui/build.rs 2>/dev/null || true

c_b=''; c_g=''; c_y=''; c_r=''; c_0=''
if [ -t 1 ]; then c_b='\033[1m'; c_g='\033[32m'; c_y='\033[33m'; c_r='\033[31m'; c_0='\033[0m'; fi
step() { printf "${c_b}==> %s${c_0}\n" "$*"; }
ok()   { printf "  ${c_g}✓${c_0} %s\n" "$*"; }
warn() { printf "  ${c_y}!${c_0} %s\n" "$*" >&2; }
die()  { printf "${c_r}error:${c_0} %s\n" "$*" >&2; exit 1; }

# ---------------------------------------------------------------- who is the user
if [ "$(id -u)" -eq 0 ]; then
    AS_ROOT=1
    USER_NAME=${SUDO_USER:-${DOAS_USER:-}}
    [ "$USER_NAME" = root ] && USER_NAME=
else
    AS_ROOT=0
    USER_NAME=$(id -un)
fi
USER_HOME=
USER_UID=
if [ -n "$USER_NAME" ]; then
    USER_HOME=$(getent passwd "$USER_NAME" | cut -d: -f6)
    USER_UID=$(id -u "$USER_NAME")
fi

# run a command as the desktop user (with their login environment: cargo in PATH etc.)
as_user() {
    if [ "$AS_ROOT" -eq 0 ]; then
        "$@"
    elif [ -n "$USER_NAME" ]; then
        if command -v runuser >/dev/null 2>&1; then
            runuser -u "$USER_NAME" -- env HOME="$USER_HOME" XDG_RUNTIME_DIR="/run/user/$USER_UID" "$@"
        else
            sudo -u "$USER_NAME" -H env XDG_RUNTIME_DIR="/run/user/$USER_UID" "$@"
        fi
    else
        "$@"
    fi
}

# run a command with root rights
SUDO=
if [ "$AS_ROOT" -eq 0 ]; then
    if command -v sudo >/dev/null 2>&1; then SUDO=sudo
    elif command -v doas >/dev/null 2>&1; then SUDO=doas
    else die "run this script as root or install sudo"
    fi
fi
root() { $SUDO "$@"; }

BINS="aqua aqua-settings aqua-finder aqua-store aqua-filechooser aqua-polkit-agent aqua-greeter"
SCRIPTS="aqua-session aqua-screenshot aqua-nvidia-setup"
ALL_PREFIXES="/usr /usr/local"

# who owns a file (pacman / dpkg / rpm), empty if nobody
pkg_owner() {
    if command -v pacman >/dev/null 2>&1; then
        pacman -Qqo "$1" 2>/dev/null && return 0
    fi
    if command -v dpkg-query >/dev/null 2>&1; then
        dpkg-query -S "$1" 2>/dev/null | cut -d: -f1 && return 0
    fi
    if command -v rpm >/dev/null 2>&1; then
        rpm -qf "$1" 2>/dev/null | grep -v 'not owned' && return 0
    fi
    return 0
}

remove_stale() { # path — remove an old copy unless a package owns it
    f=$1
    [ -e "$f" ] || [ -L "$f" ] || return 0
    owner=$(pkg_owner "$f")
    if [ -n "$owner" ]; then
        warn "$f belongs to package '$owner' and shadows/duplicates this install — remove that package"
        return 0
    fi
    case "$f" in
        "$USER_HOME"/*) rm -rf "$f" 2>/dev/null || root rm -rf "$f" ;;
        *) root rm -rf "$f" ;;
    esac
    ok "removed old $f"
}

# --------------------------------------------------------------------- uninstall
if [ "$UNINSTALL" -eq 1 ]; then
    step "Removing Aqua"
    for p in $ALL_PREFIXES; do
        for b in $BINS $SCRIPTS; do remove_stale "$p/bin/$b"; done
        remove_stale "$p/share/aqua"
        remove_stale "$p/share/applications/aqua-screenshot.desktop"
        remove_stale "$p/share/applications/aqua-settings.desktop"
        remove_stale "$p/share/applications/org.aqua.store.desktop"
    done
    remove_stale /etc/xdg/autostart/org.aqua.store-updates.desktop
    remove_stale /usr/share/wayland-sessions/aqua.desktop
    remove_stale /usr/share/xdg-desktop-portal/portals/aqua.portal
    remove_stale /usr/share/xdg-desktop-portal/aqua-portals.conf
    remove_stale /usr/share/polkit-1/actions/org.aqua.nvidia-setup.policy
    echo "Kept /etc/pam.d/aqua and ~/.config/aqua (delete them by hand if you want)."
    exit 0
fi

# ------------------------------------------------------------------ dependencies
. "$SRC/dist/deps.sh"
if [ "$DEPS" -eq 1 ] || [ "$DEPS_ONLY" -eq 1 ]; then
    install_dependencies
fi
if [ "$BUILD" != no ] || [ "$DEPS_ONLY" -eq 1 ]; then
    ensure_rust
fi
[ "$DEPS_ONLY" -eq 1 ] && { ok "dependencies installed"; exit 0; }

# ------------------------------------------------------------------------- build
TARGET=${CARGO_TARGET_DIR:-$SRC/target}
case "$TARGET" in /*) ;; *) TARGET=$SRC/$TARGET ;; esac
REL=$TARGET/release

if [ "$BUILD" != no ]; then
    # cargo itself decides what is stale (a no-op build takes a second); never trust
    # timestamps or leftover binaries from older builds
    step "Building Aqua (release)"
    if [ "$AS_ROOT" -eq 1 ] && [ -z "$USER_NAME" ]; then
        warn "building as root (no SUDO_USER); consider running the script as your user"
    fi
    # login shell so ~/.cargo/env and rustup are picked up even under sudo
    as_user sh -lc "cd '$SRC' && { [ -f \"\$HOME/.cargo/env\" ] && . \"\$HOME/.cargo/env\"; true; } && \
        CARGO_TARGET_DIR='$TARGET' ${CARGO_BIN:-cargo} build --release --locked -p aqua-compositor -p aqua-ui" \
        || die "cargo build failed (see the errors above)"
    ok "build finished"
fi

for b in $BINS; do
    [ -x "$REL/$b" ] || die "$REL/$b not found — build first (or drop --no-build)"
done
if [ -d "$SRC/target-ui" ]; then
    warn "ignoring the old target-ui/ directory (it held stale binaries); you can delete it"
fi

# ----------------------------------------------------------------------- install
BIN=$PREFIX/bin
SHARE=$PREFIX/share/aqua

put() { # mode src dst — atomic replace (safe while the old binary is running)
    root install -d "$(dirname "$3")"
    root install -m "$1" "$2" "$3.aqua-new"
    root mv -f "$3.aqua-new" "$3"
}

step "Installing to $PREFIX"
for b in $BINS; do
    put 755 "$REL/$b" "$BIN/$b"
done
ok "binaries: $BINS"
put 755 dist/aqua-session "$BIN/aqua-session"
put 755 dist/aqua-screenshot "$BIN/aqua-screenshot"
put 755 dist/aqua-nvidia-setup "$BIN/aqua-nvidia-setup"
ok "scripts: $SCRIPTS"

# session entry: absolute Exec so display managers never pick an old aqua-session
tmp=$(mktemp)
sed -e "s|^Exec=.*|Exec=$BIN/aqua-session|" -e "s|^TryExec=.*|TryExec=$BIN/aqua|" dist/aqua.desktop > "$tmp"
put 644 "$tmp" /usr/share/wayland-sessions/aqua.desktop
rm -f "$tmp"
put 644 dist/aqua.portal /usr/share/xdg-desktop-portal/portals/aqua.portal
put 644 dist/aqua-portals.conf /usr/share/xdg-desktop-portal/aqua-portals.conf
put 644 dist/aqua-screenshot.desktop "$PREFIX/share/applications/aqua-screenshot.desktop"
put 644 dist/org.aqua.finder.desktop "$PREFIX/share/applications/org.aqua.finder.desktop"
put 644 dist/org.aqua.store.desktop "$PREFIX/share/applications/org.aqua.store.desktop"
put 644 dist/org.aqua.store-updates.desktop /etc/xdg/autostart/org.aqua.store-updates.desktop
tmp=$(mktemp)
sed "s|@BINDIR@|$BIN|g" dist/org.aqua.nvidia-setup.policy > "$tmp"
put 644 "$tmp" /usr/share/polkit-1/actions/org.aqua.nvidia-setup.policy
rm -f "$tmp"
ok "session, portal and desktop entries"

if [ ! -f /etc/pam.d/aqua ]; then
    put 644 dist/pam.d/aqua /etc/pam.d/aqua
    ok "PAM config /etc/pam.d/aqua"
elif ! cmp -s dist/pam.d/aqua /etc/pam.d/aqua; then
    root install -m 644 dist/pam.d/aqua /etc/pam.d/aqua.aqua-new
    warn "/etc/pam.d/aqua differs from the shipped one; new version saved as /etc/pam.d/aqua.aqua-new"
fi

# data: replace the whole directory so removed/renamed fonts do not linger
tmpd=$(mktemp -d)
mkdir -p "$tmpd/fonts"
cp -R assets/fonts/. "$tmpd/fonts/"
cp dist/greetd/config.toml "$tmpd/greetd-config.toml"
[ -f dist/arch/nvidia.conf ] && cp dist/arch/nvidia.conf "$tmpd/nvidia-modprobe.conf"
# zenity / kdialog file dialogs → Aqua's open/save panel (the session puts shims/ first in PATH;
# every other zenity/kdialog dialog runs the real program)
mkdir -p "$tmpd/shims"
for s in zenity kdialog; do ln -s "$BIN/aqua-filechooser" "$tmpd/shims/$s"; done
# JSON Schema referenced by the `#:schema` line System Settings writes into config.toml
"$REL/aqua" config-schema > "$tmpd/config.schema.json" 2>/dev/null || rm -f "$tmpd/config.schema.json"
chmod -R a+rX "$tmpd"
root rm -rf "$SHARE.aqua-new"
root cp -R "$tmpd" "$SHARE.aqua-new"
root rm -rf "$SHARE"
root mv "$SHARE.aqua-new" "$SHARE"
rm -rf "$tmpd"
ok "fonts and data in $SHARE"

# -------------------------------------------------------------- remove old copies
step "Removing stale copies from other locations"
for p in $ALL_PREFIXES; do
    [ "$p" = "$PREFIX" ] && continue
    for b in $BINS $SCRIPTS; do remove_stale "$p/bin/$b"; done
    # an old font dir in /usr/share/aqua would be found before /usr/local/share/aqua
    remove_stale "$p/share/aqua"
    remove_stale "$p/share/applications/aqua-screenshot.desktop"
done
if [ -n "$USER_HOME" ]; then
    for d in "$USER_HOME/.cargo/bin" "$USER_HOME/.local/bin"; do
        for b in $BINS $SCRIPTS; do remove_stale "$d/$b"; done
    done
    remove_stale "$USER_HOME/.local/share/aqua/fonts"
fi
ok "done"

# check what $PATH resolves to now
if [ -n "$USER_NAME" ]; then
    for b in aqua aqua-settings; do
        w=$(as_user sh -lc "command -v $b" 2>/dev/null || true)
        if [ -n "$w" ] && [ "$w" != "$BIN/$b" ]; then
            warn "'$b' in your PATH is $w, not $BIN/$b"
        fi
    done
fi

# --------------------------------------------------------------------- caches
step "Refreshing caches"
command -v fc-cache >/dev/null 2>&1 && root fc-cache -f "$SHARE/fonts" >/dev/null 2>&1 && ok "font cache"
command -v update-desktop-database >/dev/null 2>&1 && root update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null && ok "desktop database"

# ---------------------------------------------------------------------- NVIDIA
if [ "$NVIDIA" -eq 1 ]; then
    step "NVIDIA kernel modesetting"
    if ! root "$BIN/aqua-nvidia-setup"; then
        warn "aqua-nvidia-setup failed — run: sudo $BIN/aqua-nvidia-setup"
    fi
fi

# ---------------------------------------------------------------- running session
if [ "$RELOAD" -eq 1 ] && [ -n "$USER_NAME" ]; then
    step "Updating the running session"
    rt=/run/user/$USER_UID
    # helpers are restarted on demand by the compositor / portal
    for h in aqua-settings aqua-finder aqua-store aqua-filechooser; do
        if pkill -u "$USER_NAME" -x "$h" 2>/dev/null; then ok "closed old $h (reopen it to get the new version)"; fi
    done
    if pkill -u "$USER_NAME" -x aqua-polkit-agent 2>/dev/null; then ok "restarted polkit agent"; fi
    # the compositor rewrites this on its next tick with the real mode lists
    [ -f "$rt/aqua-outputs" ] && rm -f "$rt/aqua-outputs" 2>/dev/null || true
    if pgrep -u "$USER_NAME" -x aqua >/dev/null 2>&1; then
        if as_user "$BIN/aqua" msg reload >/dev/null 2>&1; then
            ok "running compositor reloaded its configuration"
        fi
        running=$(readlink "/proc/$(pgrep -u "$USER_NAME" -x aqua | head -n 1)/exe" 2>/dev/null || true)
        case "$running" in
            *"(deleted)"*|"") warn "Aqua is running an older binary — log out and back in (or: sudo systemctl restart greetd) to start the new version" ;;
            *) [ "$running" != "$BIN/aqua" ] && warn "the running Aqua is $running — log out and back in to switch to $BIN/aqua" ;;
        esac
    fi
fi

echo
printf "${c_g}Aqua installed to $PREFIX.${c_0}\n"
echo "Pick \"Aqua\" on the login screen, run 'aqua-session' from a TTY,"
echo "or use greetd: sudo cp $SHARE/greetd-config.toml /etc/greetd/config.toml"
