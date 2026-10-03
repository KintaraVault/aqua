#!/bin/sh
# Build the Fedora RPM from this checkout:  dist/fedora/build-rpm.sh [--vendor]
#   --vendor  also create a `cargo vendor` tarball and build offline (as mock/koji would)
set -eu
SRC=$(cd "$(dirname "$0")/../.." && pwd)
SPEC=$SRC/dist/fedora/aqua-desktop.spec
NAME=aqua-desktop
VER=$(sed -n 's/^Version: *//p' "$SPEC")
TOP=$(rpm --eval '%{_topdir}')
mkdir -p "$TOP/SOURCES" "$TOP/SPECS"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# Source tarball without build output (works with and without git)
mkdir "$tmp/$NAME-$VER"
(cd "$SRC" && tar --exclude=./target --exclude=./target-ui --exclude=./.git --exclude=./vendor -cf - .) |
    tar -xf - -C "$tmp/$NAME-$VER"
tar -czf "$TOP/SOURCES/$NAME-$VER.tar.gz" -C "$tmp" "$NAME-$VER"
WITH=""
if [ "${1:-}" = "--vendor" ]; then
    (cd "$tmp/$NAME-$VER" && cargo vendor --locked vendor >/dev/null)
    tar -cJf "$TOP/SOURCES/$NAME-$VER-vendor.tar.xz" -C "$tmp/$NAME-$VER" vendor
    WITH="--with vendor"
fi
cp "$SPEC" "$TOP/SPECS/"
# shellcheck disable=SC2086
rpmbuild -ba $WITH "$TOP/SPECS/$NAME.spec"
