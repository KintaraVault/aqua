#!/bin/sh
# Install a built Aqua tree into a staging root — shared by the distribution packages
# (dist/arch/PKGBUILD, dist/fedora/aqua-desktop.spec, debian/rules).
#
#   dist/stage-install.sh DESTDIR [--prefix /usr] [--target DIR] [--pam system-auth|common]
#
# --target  directory holding the release binaries (default: target/release)
# --pam     PAM stack the lock screen includes: "system-auth" (Arch, Fedora) or
#           "common" (Debian/Ubuntu: common-auth / common-account)
# For installing straight onto a running system use dist/install.sh instead.
set -eu

die() { echo "stage-install: $*" >&2; exit 1; }

[ $# -ge 1 ] || die "usage: $0 DESTDIR [--prefix /usr] [--target DIR] [--pam system-auth|common]"
DESTDIR=$1
shift
PREFIX=/usr
TARGET=target/release
PAM=system-auth
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) PREFIX=${2:?}; shift 2 ;;
        --target) TARGET=${2:?}; shift 2 ;;
        --pam) PAM=${2:?}; shift 2 ;;
        *) die "unknown option $1" ;;
    esac
done
case $PAM in system-auth | common) ;; *) die "--pam must be system-auth or common" ;; esac

SRC=$(cd "$(dirname "$0")/.." && pwd)
cd "$SRC"
case $TARGET in /*) ;; *) TARGET=$SRC/$TARGET ;; esac

BINS="aqua aqua-settings aqua-finder aqua-filechooser aqua-polkit-agent aqua-greeter"
for b in $BINS; do
    [ -x "$TARGET/$b" ] || die "$TARGET/$b not found — run: cargo build --release -p aqua-compositor -p aqua-ui"
done

R=$DESTDIR$PREFIX
# Session/portal files are looked up in /usr/share only, whatever the prefix.
U=$DESTDIR/usr/share

for b in $BINS; do install -Dm755 "$TARGET/$b" "$R/bin/$b"; done
install -Dm755 dist/aqua-session "$R/bin/aqua-session"
install -Dm755 dist/aqua-screenshot "$R/bin/aqua-screenshot"

install -d "$U/wayland-sessions"
sed -e "s|^Exec=.*|Exec=$PREFIX/bin/aqua-session|" -e "s|^TryExec=.*|TryExec=$PREFIX/bin/aqua|" \
    dist/aqua.desktop > "$U/wayland-sessions/aqua.desktop"
chmod 644 "$U/wayland-sessions/aqua.desktop"
install -Dm644 dist/aqua.portal "$U/xdg-desktop-portal/portals/aqua.portal"
install -Dm644 dist/aqua-portals.conf "$U/xdg-desktop-portal/aqua-portals.conf"
install -Dm644 dist/aqua-screenshot.desktop "$R/share/applications/aqua-screenshot.desktop"
install -Dm644 dist/org.aqua.finder.desktop "$R/share/applications/org.aqua.finder.desktop"

install -d "$DESTDIR/etc/pam.d"
if [ "$PAM" = common ]; then
    cat > "$DESTDIR/etc/pam.d/aqua" <<'PAM'
#%PAM-1.0
# Used by the Aqua lock screen to verify the user's password.
@include common-auth
@include common-account
PAM
else
    cp dist/pam.d/aqua "$DESTDIR/etc/pam.d/aqua"
fi
chmod 644 "$DESTDIR/etc/pam.d/aqua"

S=$R/share/aqua
install -d "$S/fonts"
cp -R assets/fonts/. "$S/fonts/"
find "$S/fonts" -type d -exec chmod 755 {} +
find "$S/fonts" -type f -exec chmod 644 {} +
install -Dm644 dist/greetd/config.toml "$S/greetd-config.toml"
install -Dm644 dist/arch/nvidia.conf "$S/nvidia-modprobe.conf"
# JSON Schema referenced by the `#:schema` line System Settings writes into config.toml
"$TARGET/aqua" config-schema > "$S/config.schema.json"
chmod 644 "$S/config.schema.json"

install -Dm644 LICENSE "$R/share/licenses/aqua-desktop/LICENSE"
install -Dm644 README.md "$R/share/doc/aqua-desktop/README.md"
