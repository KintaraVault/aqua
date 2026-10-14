#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-$root/target/release/aqua}
out=${2:-/tmp/aqua-input-assist}
mode=${3:-plain}
mkdir -p "$out"
out=$(realpath "$out")
rt=$(mktemp -d /tmp/aqua-input-assist.XXXXXX)
chmod 700 "$rt"
mkdir -p "$rt/home" "$rt/config" "$rt/data" "$rt/cache" "$rt/state"
xp=; controller=
cleanup() {
    [[ -z "$controller" ]] || kill "$controller" 2>/dev/null || true
    if [[ -n "$xp" ]]; then kill "$xp" 2>/dev/null || true; wait "$xp" 2>/dev/null || true; fi
    # The private runtime directory scopes cleanup to this test, never the live desktop.
    for p in $(pgrep -u "$(id -u)" || true); do
        if [[ -r /proc/$p/environ ]] && tr '\0' '\n' 2>/dev/null < "/proc/$p/environ" | grep -qx "XDG_RUNTIME_DIR=$rt"; then
            kill "$p" 2>/dev/null || true
        fi
    done
    [[ $rt != /tmp/aqua-input-assist.* ]] || rm -rf -- "$rt"
}
trap cleanup EXIT
protocols=$(pkg-config --variable=pkgdatadir wayland-protocols)
for spec in "xdg-shell:$protocols/stable/xdg-shell/xdg-shell.xml" \
            "text-input:$protocols/unstable/text-input/text-input-unstable-v3.xml" \
            "text-field:$root/protocols/aqua-text-field-v1.xml"; do
    name=${spec%%:*}; xml=${spec#*:}
    wayland-scanner client-header "$xml" "$rt/$name-client.h"
    wayland-scanner private-code "$xml" "$rt/$name-code.c"
done
cc -std=c11 -O2 -Wall -Wextra -Werror -I"$rt" "$root/tests/input-assist-client.c" "$rt/"*-code.c \
   $(pkg-config --cflags --libs wayland-client cairo) -o "$rt/client"
Xvfb -noreset -nolisten tcp -displayfd 3 -screen 0 1280x800x24 3>"$rt/display" >"$out/xvfb-$mode.log" 2>&1 & xp=$!
for _ in $(seq 1 50); do [[ ! -s "$rt/display" ]] || break; sleep 0.1; done
[[ -s "$rt/display" ]]
for _ in $(seq 1 50); do
    if DISPLAY=":$(cat "$rt/display")" xdotool getdisplaygeometry >/dev/null 2>&1; then break; fi
    kill -0 "$xp" 2>/dev/null || { cat "$out/xvfb-$mode.log"; exit 1; }
    sleep 0.1
done
DISPLAY=":$(cat "$rt/display")" xdotool getdisplaygeometry >/dev/null
: > "$rt/control"
client_command="$rt/client $mode"
if [[ $mode == password-fill || $mode == password-cancel ]]; then
    command -v gnome-keyring-daemon >/dev/null
    fixture="$root/target/release/examples/password_fixture"
    [[ -x "$fixture" ]] || { echo "Build: cargo build --release -p aqua-text --example password_fixture"; exit 1; }
    client_command="printf aqua-test-keyring | gnome-keyring-daemon --unlock --components=secrets >'$rt/keyring.log' 2>&1; AQUA_TEST_KEYRING=1 '$fixture' >'$out/fixture-$mode.log' 2>&1 && '$rt/client' '$mode' 2>'$rt/cache/aqua/logs/client.log'"
fi
if [[ $mode == gtk* || $mode == browser* ]]; then
    backend=wayland; [[ $mode != gtk-x11 ]] || backend=x11
    fixture="$root/tests/input-assist-gtk.py"; [[ $mode != browser* ]] || fixture="$root/tests/input-assist-browser.py"
    client_command="gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' >/dev/null; GDK_BACKEND=$backend GTK_IM_MODULE=gtk-im-context-simple NO_AT_BRIDGE=0 python '$fixture' '$mode' >'$rt/cache/aqua/logs/client.log' 2>&1"
fi
if [[ $mode == finder* ]]; then
    client_command="gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' >/dev/null; (sleep 3; echo INPUT_ENABLED >'$rt/cache/aqua/logs/client.log') & '$root/target/release/aqua-finder' '$rt/home'"
fi
# A seed-only dictionary makes acceptance assertions deterministic.
cp "$root/crates/aqua-text/src/seed.txt" "$rt/dictionary"
(
    for _ in $(seq 1 120); do grep -q INPUT_ENABLED "$rt/cache/aqua/logs/client.log" 2>/dev/null && break; sleep 0.1; done
    sleep 2
    if [[ $mode == finder* ]]; then
        key=ctrl+k; [[ $mode != finder-goto ]] || key=ctrl+shift+g
        printf 'hotkey %s\n' "$key" >> "$rt/control"; sleep 2
    fi
    printf 'dump %s/state-%s.txt\n' "$out" "$mode" >> "$rt/control"
    printf 'shot %s/before-%s.png\n' "$out" "$mode" >> "$rt/control"
    sleep 1
    if [[ $mode != password && $mode != invalid && $mode != finder* ]]; then
        printf 'hotkey tab\n' >> "$rt/control"
    fi
    if [[ $mode == password-fill || $mode == password-cancel || $mode == browser-password || $mode == browser-password-cancel || $mode == browser-password-protected-cancel || $mode == browser-password-http ]]; then
        sleep 1
        printf 'shot %s/choices-%s.png\n' "$out" "$mode" >> "$rt/control"
        sleep 1
        printf 'hotkey tab\n' >> "$rt/control"
    fi
    sleep 1
    printf 'shot %s/after-%s.png\n' "$out" "$mode" >> "$rt/control"
) & controller=$!
env -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u AQUA_SOCKET -u AQUA_NESTED_DBUS HOME="$rt/home" \
    XDG_CONFIG_HOME="$rt/config" XDG_DATA_HOME="$rt/data" XDG_CACHE_HOME="$rt/cache" XDG_STATE_HOME="$rt/state" XDG_RUNTIME_DIR="$rt" \
    DISPLAY=":$(cat "$rt/display")" LIBGL_ALWAYS_SOFTWARE=1 AQUA_CTL="$rt/control" \
    AQUA_DICTIONARY="$rt/dictionary" AQUA_T9=1 AQUA_TEXT_ASSIST=1 AQUA_NO_AUTOSTART=1 AQUA_NO_POLKIT=1 \
    timeout 25 dbus-run-session -- "$bin" --winit --size 1280x800 --quit-after 12 \
    -c "$client_command" > "$out/session-$mode.log" 2>&1
cp "$rt/cache/aqua/logs/client.log" "$out/client-$mode.log"
cp "$rt/cache/aqua/logs/compositor.log" "$out/compositor-$mode.log"
[[ -s "$out/before-$mode.png" && -s "$out/after-$mode.png" ]]
case "$mode" in
    plain|multi|gtk|gtk-x11|browser) grep -q 'TEXT=hello$' "$out/client-$mode.log"; [[ $(grep -c '^INSERT ' "$out/client-$mode.log") == 2 ]] ;;
    cancel|gtk-cancel|browser-cancel) [[ $(grep -c '^INSERT ' "$out/client-$mode.log") == 1 ]]; grep -q INPUT_DISABLED_AFTER_FIRST "$out/client-$mode.log" ;;
    unicode|gtk-unicode|browser-unicode) grep -q 'TEXT=привет$' "$out/client-$mode.log" ;;
    t9|browser-t9) grep -q 'TEXT=hello$' "$out/client-$mode.log" ;;
    browser-password) [[ $(grep -c '^INSERT ' "$out/client-$mode.log") == 11 ]]; grep -q 'PASSWORD_LENGTH=11$' "$out/client-$mode.log"; grep -q 'origin-verified=true' "$out/state-$mode.txt" ;;
    browser-password-cancel|browser-password-protected-cancel) [[ $(grep -c '^INSERT ' "$out/client-$mode.log") == 1 ]]; grep -q INPUT_DISABLED_AFTER_FIRST "$out/client-$mode.log" ;;
    browser-password-http) ! grep -q '^INSERT ' "$out/client-$mode.log"; grep -q 'origin-verified=false' "$out/state-$mode.txt" ;;
    password-fill) [[ $(grep -c '^INSERT ' "$out/client-$mode.log") == 11 ]]; grep -q FIXTURE_READY "$out/fixture-$mode.log" ;;
    password-cancel) [[ $(grep -c '^INSERT ' "$out/client-$mode.log") == 1 ]]; grep -q INPUT_DISABLED_AFTER_FIRST "$out/client-$mode.log" ;;
    finder|finder-goto) grep -q 'radius-known=true' "$out/state-$mode.txt"; grep -q 'caret-known=true' "$out/state-$mode.txt" ;;
    password|invalid) ! grep -q '^INSERT ' "$out/client-$mode.log" ;;
esac
printf 'PASS %s — screenshots in %s\n' "$mode" "$out"
