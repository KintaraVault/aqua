# Building Aqua

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

## Worktrees and build isolation

Keep a separate physical `target` directory for each Git worktree. Do not symlink one
worktree's `target` to another or reuse the same `CARGO_TARGET_DIR` for source trees with
different local changes. Restored source timestamps can make stale local libraries look
fresh and produce misleading missing-field or missing-method errors.

If this has happened, stop builds in the other tree and invalidate the affected local
packages in **both** profiles (`cargo clean -p PACKAGE` and
`cargo clean --release -p PACKAGE`) before rebuilding. Cleaning the development profile
alone does not remove release artifacts. Ordinary development uses the workspace's own
`target` directory; no global dependency-cache cleanup is needed.

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
