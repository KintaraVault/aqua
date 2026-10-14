# Native text assistance — validation

Implementation: `bc78a9f`; HTTP refusal harness: `3106fb4`.

## Results

- 58 unit tests passed: compositor 39, text 13, localization 6.
- Strict Clippy passed for the compositor binary and UI library (`-D warnings`).
- 23 isolated nested-session scenarios passed on the final compositor binary.
- 52 real 1280×800 screenshots captured. No mockups or extensions.
- GTK Wayland/X11, Chromium Latin/Russian/T9, native Wayland, Finder shared/address controls tested.
- Password tests used a private Secret Service store and ephemeral local HTTPS fixture, never the live keyring.
- Focus cancellation passed for both normal and protected destination fields. HTTP selection was attempted and refused.
- Exact rounded field bounds verified in Finder; unknown toolkit/CSS radius is not invented.

Scenarios: plain, multi, unicode, t9, cancel, password, invalid, password-fill, password-cancel, gtk, gtk-unicode, gtk-cancel, gtk-x11, browser, browser-unicode, browser-t9, browser-cancel, browser-password-http, browser-password, browser-password-cancel, browser-password-protected-cancel, finder, finder-goto.

## Integration and remaining blocker

Changes were integrated into `/home/valance/aqua`, branch `native-input-assist`, preserving existing unstaged edits and new files. The original desktop session was not restarted. Backups are in `~/aqua-backups/input-assist-final`.

The clean worktree `~/aqua-atspi-check` builds and passes the checks above. The main directory still has pre-existing unfinished desktop/visual changes: its build reports 20 errors, including missing `desktop_icons`, `desktop_snap`, `tint_file`, `marquee_offset`, `Canvas::marquee` and `Canvas::draw_pixmap_rounded`. These unrelated features were not guessed, disabled or removed; their intended completion needs confirmation.

Screenshots were captured and geometry assertions passed. The image-viewing tool was unavailable, so these captures are not represented as having received a visual review.

See `docs/text-assistance.md` for compatibility, privacy and exact scope restrictions. Precise input detection requires application text-input/accessibility metadata. AT-SPI does not supply arbitrary CSS contours or TLS certificate attestation. Browser credential scope never falls back to a browser-wide app ID. XWayland password injection is disabled.

The commits are local; this validation did not push a branch or replace the installed/live desktop binary.
