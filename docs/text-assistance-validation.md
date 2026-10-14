# Native text assistance — validation

Implementation checkpoint: `677ad82` on local branch `native-input-assist`.

## Final main-workspace results

- `cargo build --release --workspace`: passed, including all bundled applications.
- `cargo test --workspace --all-features`: 448 passed, 0 failed, 3 explicitly ignored manual rendering/installed-app audit helpers.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- 23 isolated nested-session scenarios passed on the main-workspace release binary.
- 52 real 1280×800 screenshots captured. No mockups or extensions.
- GTK Wayland/X11, Chromium Latin/Russian/T9, native Wayland and Finder shared/address controls tested.
- Password fixtures used a private Secret Service store and ephemeral local HTTPS origin, never the live keyring.
- Focus cancellation passed for normal and protected destination fields. HTTP password selection was attempted and refused.
- Finder rounded field bounds were checked; unknown toolkit/CSS radius is not invented.

Scenarios: plain, multi, unicode, t9, cancel, password, invalid, password-fill, password-cancel, gtk, gtk-unicode, gtk-cancel, gtk-x11, browser, browser-unicode, browser-t9, browser-cancel, browser-password-http, browser-password, browser-password-cancel, browser-password-protected-cancel, finder, finder-goto.

## Follow-up fixes

Chromium can ignore native prefix deletion. Browser T9/correction now selects only the independently validated prefix through AT-SPI and then types through native grapheme commits. Prefix verification checks focus generation, widget identity, protection, caret, selection and exact text. Native non-browser deletion is a separate transaction whose result must be observed before insertion. No clipboard, extension or broadcast synthetic-keymap fallback is used.

In-flight predicted text and offsets are not exposed as independently observed field context. Regression tests cover Unicode prefix bounds, config compatibility/roundtrips, marquee timing/bounds and rounded artwork at both scales. The shell test module was moved after production items for strict Clippy. The harness waits for actual fixture readiness and exits after the screenshot instead of racing a fixed 12-second desktop lifetime; all behavioral assertions remain enabled.

## Build isolation correction

The earlier report incorrectly attributed 20 main-directory build errors to unfinished desktop code. The missing definitions were already present in the saved original source. Two worktrees sharing one Cargo target directory had reused stale release libraries; cleaning only development artifacts did not fix them.

The worktrees now use separate physical build directories. Invalidating local release packages restored the main build with existing desktop/appearance features intact. Pre-existing changes were preserved in checkpoint `be75d68`, then formatted separately in `532a8d9`. No missing API stubs or guessed defaults were introduced. Backups remain in `~/aqua-backups/`.

## Scope and delivery

Screenshots were captured and geometry assertions passed. The image-viewing tool was unavailable, so these captures are not represented as visually reviewed.

See `docs/text-assistance.md` for compatibility/privacy restrictions. Precise detection requires application text-input/accessibility metadata; unsupported applications are not inferred through OCR. AT-SPI does not provide arbitrary CSS contours or TLS certificate attestation. Browser credentials require exact verified HTTPS scope and never fall back to browser-wide application scope. XWayland password delivery is disabled.

Commits are local. This validation did not push the branch, install the new binaries or restart the original desktop session.
