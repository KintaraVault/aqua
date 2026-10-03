# Aqua — Wayland desktop in Rust

Aqua is its own Wayland compositor and shell, built on [Smithay](https://github.com/Smithay/smithay). It is not a KDE/GNOME theme. It recreates the macOS 26/27 "Liquid Glass" look: wallpaper, menu bar, Dock, glass Control Center, Apple menu, the Applications panel (Launchpad), rounded windows with shader shadows and traffic‑light titlebars, and an icon pipeline that swaps in macOS‑style icons automatically.

## Crates (modular)
| crate | role |
|---|---|
| `aqua-config` | TOML config, glass style, metrics, dock items, shortcuts; `paths` — shared runtime/home/XDG user-dir/control-socket paths |
| `aqua-gfx` | tiny‑skia canvas, SF Pro text rendering, shapes |
| `aqua-wallpaper` | wallpaper loading/scaling, built‑in procedural wallpaper |
| `aqua-icons` | icon resolution: built‑in icons → Linux app equivalents → App Store icon fetch (cache `~/.cache/aqua/icons`) → freedesktop theme; squircle masking |
| `aqua-apps` | `.desktop` scanning, categories, app‑id matching |
| `aqua-sys` | system state over D-Bus/sysfs: audio, network, battery, backlight, session |
| `aqua-notify` | notification daemon (`org.freedesktop.Notifications`) and XDG settings portal |
| `aqua-tray` | system tray: `StatusNotifierWatcher`/host, `StatusNotifierItem` + `com.canonical.dbusmenu` |
| `aqua-shell` | menu bar (`bar/`), dock, panels (`panels/`), shared drawing helpers (`kit/`) — pure 2D layers |
| `aqua-render` | GLES shaders: glass (blur + refraction + rim + legibility), rounded clip, shadows, genie |
| `aqua-i18n` | UI translations (gettext `.po` catalogs compiled in, plural forms, `AQUA_LANG`) |
| `aqua-wm` | window-management rules as plain, unit-tested data: Spaces, placement, zoom, Mission Control grid |
| `aqua-screencast` | PipeWire video sources for the ScreenCast portal (optional `screencast` feature, on by default) |
| `aqua-ui` | Slint apps: Finder + file chooser, System Settings, polkit agent, greetd greeter |
| `aqua-compositor` | binary `aqua`: Smithay compositor |

### Compositor layout (`crates/aqua-compositor/src`)
| module | role |
|---|---|
| `main.rs`, `cli.rs` | entry point, command line |
| `backend/` | shared event loop + frame scheduling (`mod.rs`), `udev.rs` (DRM/KMS, TTY), `winit.rs` (nested) |
| `session.rs` | session start (XWayland, environment export, tray, polkit agent, autostart), config reload |
| `state/` | the `Aqua` state struct |
| `wm/` | windows, Spaces, Mission Control, outputs, move/resize grabs |
| `input/` | keyboard/pointer handling, libinput config, cursors, focus, gestures |
| `wayland/` | protocol handlers, blur protocol, XWayland |
| `selection/` | clipboard, file drag and drop |
| `capture/` | screenshots, screen recording |
| `system/` | env, sound, idle, lock, logind, PAM, GTK/Qt theming |
| `render/` | scene assembly (`mod.rs`), `layers.rs`, `window.rs`, `snapshot.rs`, `pointer.rs`, `capture.rs` (screenshots/pixel readback) |
| `control.rs`, `ipc.rs`, `portal.rs` | `aqua msg` control socket, test hooks, screenshot portal |

## Build & run
Build dependencies: Rust ≥ 1.88, clang, pkg-config and the development files of libinput,
libudev, libseat, gbm, EGL, libdrm, xkbcommon, wayland, PipeWire, fontconfig, freetype, D-Bus and PAM:
```sh
sudo pacman -S --needed rust clang pkgconf libinput seatd mesa libdrm libxkbcommon wayland pipewire fontconfig freetype2 dbus pam
sudo dnf install cargo rust clang-devel pkgconf-pkg-config libinput-devel systemd-devel libseat-devel mesa-libgbm-devel \
    mesa-libEGL-devel libdrm-devel libxkbcommon-devel wayland-devel pipewire-devel fontconfig-devel freetype-devel dbus-devel pam-devel
sudo apt install cargo rustc clang libclang-dev pkgconf libinput-dev libudev-dev libseat-dev libgbm-dev libegl-dev libdrm-dev \
    libxkbcommon-dev libwayland-dev libpipewire-0.3-dev libfontconfig-dev libfreetype-dev libdbus-1-dev libpam0g-dev
```
Without PipeWire: `cargo build --release -p aqua-compositor --no-default-features` (no native
ScreenCast portal).

```sh
cargo build --release
./target/release/aqua --winit -c gnome-calculator      # nested in an existing session
./target/release/aqua --tty                             # from a TTY (DRM/KMS + libinput via libseat)
./dist/install.sh                                       # build if needed + install/update (asks for sudo)
```
`dist/install.sh` is safe to run over an existing install: it rebuilds stale binaries,
replaces files atomically, removes old copies from other prefixes (`/usr/bin`,
`/usr/local/bin`, `~/.cargo/bin`, `~/.local/bin`) that would shadow the new ones,
refreshes fonts/caches and reloads the running session. Options: `--prefix DIR`,
`--no-build`, `--rebuild`, `--no-reload`, `--uninstall`.

### Packages
All three use `dist/stage-install.sh DESTDIR [--prefix /usr] [--target DIR] [--pam system-auth|common]`,
which lays out the same files as `dist/install.sh` (binaries, session/portal/desktop entries,
`/etc/pam.d/aqua`, fonts, greetd example config, `config.schema.json`) in a staging root.

| distribution | files | build |
|---|---|---|
| Arch | `dist/arch/PKGBUILD` | `cd dist/arch && makepkg -si` |
| Fedora | `dist/fedora/aqua-desktop.spec` | `dist/fedora/build-rpm.sh` (`--vendor` for an offline, mock-style build), then `sudo dnf install ~/rpmbuild/RPMS/*/aqua-desktop-*.rpm` |
| Debian / Ubuntu | `debian/` | `dpkg-buildpackage -us -uc -b`, then `sudo apt install ../aqua-desktop_*.deb` |

The builds need network access for crates.io and the pinned Smithay git revision unless a
`cargo vendor` tree is supplied. Debian's PAM file includes `common-auth`/`common-account`,
the others `system-auth`.

Talk to a running session with `aqua msg <command>` (e.g. `aqua msg reload`,
`aqua msg screenshot area`, `aqua msg spotlight`).
Options: `--size WxH`, `--scale S`, `-c CMD` (repeatable), `--screenshot PATH --after SECS`, `--quit-after SECS`.
`aqua check-config [FILE]` validates a config file and lists every problem; `aqua config-schema`
prints its JSON Schema (installed as `/usr/share/aqua/config.schema.json` and referenced from the
`#:schema` line of `config.toml`, so TOML editors complete and check the keys).

## Development
- Formatting: `cargo fmt --all` (settings in `rustfmt.toml`, width 120); check with `cargo fmt --all -- --check`.
- Lints: `cargo clippy --workspace --all-targets --all-features` — kept warning-free; `clippy.toml`
  pins the MSRV so no newer APIs are suggested, project-wide allows are in `[workspace.lints]`.
- Tests: `cargo test --workspace --all-features`. Unit tests live next to the code (`#[cfg(test)] mod tests`);
  `crates/aqua-shell/tests/shell.rs` renders the shell headlessly (needs the bundled assets/fonts,
  no display server). Compositor tests cover CLI parsing, shortcut chords, IPC and screenshot helpers;
  `aqua-wm` covers Spaces, placement and the Mission Control grid; `aqua-screencast/tests/pipewire_roundtrip.rs`
  and `aqua-tray/tests/xembed_xvfb.rs` skip themselves when PipeWire / Xvfb are not available.
  Tests compare UI text through `tr(…)`, so the suite also passes in Russian: `AQUA_LANG=ru cargo test --workspace`.
- MSRV: Rust 1.88.
- Shell layout: `aqua-shell/src/lib.rs` (shell state) + `types.rs`, `input.rs`, `actions.rs`,
  `visibility.rs`; Spotlight is `panels/spotlight/` (`calc.rs`, `search.rs`). System Settings panes
  are wired in `aqua-ui/src/bin/aqua-settings/*.rs` (`wire_*` functions).

## Languages
The UI follows `AQUA_LANG`, then `LC_ALL` / `LC_MESSAGES` / `LANG`; English is the source language,
Russian is bundled. Two catalogs, both compiled into the binaries:
* `crates/aqua-i18n/po/ru.po` — everything drawn or set from Rust: menu bar, Dock, panels, compositor
  messages, and what System Settings, Finder, the polkit agent and the greeter set from Rust code
  (statuses, file kinds, dates, alerts, context menus). API: `tr("…")`, `trf("… {name}", &[("name", &v)])`,
  `ntr("{n} item", "{n} items", n)` (`msgid_plural` + `msgstr[0..2]` in the catalog).
* `crates/aqua-ui/translations/ru/LC_MESSAGES/aqua-ui.po` — the `@tr(…)` strings of the `.slint` files.

A string present in both catalogs must have the same translation, and placeholders must match —
both checked by `cargo test -p aqua-i18n`. To add a language, add `po/<lang>.po` to `CATALOGS` in
`aqua-i18n/src/lib.rs` and `translations/<lang>/LC_MESSAGES/aqua-ui.po`.

## Shortcuts
`Super`=⌘ (or `AQUA_CMD=alt`). ⌘Space/F4 opens Applications. ⌘Return opens Terminal. ⌘Q quits, ⌘W closes, ⌘M minimises. ⌘⌥H hides others. F5 opens Control Center. ⌘⇧3/Print captures the screen, ⌘⇧4/Shift+Print a selected portion (Space → a window), Alt+Print a window, ⌘⇧5 opens the screenshot toolbar (timer, save location, floating thumbnail, show pointer). Screenshots go to the clipboard and `~/Pictures/Screenshots` (`aqua-screenshot [ui|full|area|window]`). Ctrl+Alt+Backspace exits.

Every system shortcut can be remapped or turned off in **System Settings → Keyboard →
Keyboard Shortcuts…** (click a shortcut and press the new keys; ⌫ turns it off, ⎋ cancels),
and *Custom Shortcuts* bind any chord to a named action or a shell command. They are
stored in the config:
```toml
[shortcuts]            # id = "chord, chord" ("" disables; ids in aqua-config/src/shortcuts.rs)
spotlight = "super+k"
mission = "F3, ctrl+up"
minimize = ""
[[bindings]]           # custom shortcuts: a named action or a command
keys = "ctrl+alt+t"
action = "terminal"
```
While the editor records a chord it creates `$XDG_RUNTIME_DIR/aqua-shortcut-capture`, so the
compositor lets the keys through instead of running the old shortcut.

## Config
`~/.config/aqua/config.toml` (every field optional), e.g.
```toml
wallpaper = "/home/me/Pictures/tahoe.jpg"
window_radius = 16.0
dock_icon_size = 54.0
[glass]
blur = 22.0
tint = [1.0, 1.0, 1.0, 0.18]
refraction = 9.0
```
On first start Aqua installs SF Pro to `~/.local/share/fonts/aqua` and writes GTK/GSettings settings (traffic lights on the left, SF Pro font). It never overwrites files it did not write.

### Windows, Dock and pointer
```toml
dock_click = "focus"            # click on a running app: focus | minimize | cycle | expose | new
dock_autohide = false           # hide the Dock until the pointer touches the bottom edge
dock_bounce = true              # bounce icons of launching apps
titlebar_double_click = "zoom"  # zoom | minimize | none
animate_windows = true          # animated zoom / full-screen transitions
reduce_motion = false           # Accessibility: turn animations off
cursor_size = 1.0               # pointer size (1–4)
do_not_disturb = false
notification_previews = true
menubar_autohide = "fullscreen" # fullscreen | always | never  (slides away like the Dock)
apple_icons = "all"             # all | selected | off — show Linux apps as their macOS
apple_icon_apps = []            #   counterparts (Firefox → Safari …); "selected" uses this list
[pointer]
scroll_factor = 1.0             # Settings → Mouse → Scrolling speed
accel_profile = "adaptive"      # adaptive | flat (Pointer acceleration)
```
The pointer follows what is under it: resize arrows on window borders, the client's own
shape (I-beam, hand, crosshair, grab, busy beach ball…) through `wp_cursor_shape`, drawn in a
macOS style by the compositor. Full-screen windows hide the menu bar and the Dock completely —
they don't pop out at the top or bottom screen edge (an auto-hidden Dock / menu bar on the
normal desktop still reveals at its edge). Menu bar and Dock slide out/in (animated);
full-screen windows are drawn without rounded corners and shadow. A window counts as
full screen when it asked for it, or when it is undecorated and keeps covering the whole
output for a quarter of a second while not maximised or animating (browsers' F11, X11 games) —
so apps that briefly commit oversized frames while zooming or minimising don't make the menu
bar and the Dock blink. Glass isn't rate-limited (`blur_max_fps`) while windows animate.
