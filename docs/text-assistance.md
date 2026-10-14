# Text assistance

## Compatibility

Aqua observes the focused, enabled `text-input-v3` transaction. It knows the surrounding text, UTF-8 cursor/selection offsets, content hints, purpose and caret rectangle supplied by the application. It does not read arbitrary window contents or use OCR.

The standard protocol describes a caret, not the bounds of a field. The optional `aqua-text-field-v1` protocol adds precise surface-local bounds and a corner radius, applied on `wl_surface.commit`. Applications must clear this geometry on blur. A rounded rectangle is supported; arbitrary paths are not represented. Unknown field bounds remain unknown.

Suggestions currently attach to toplevels and their popups. An external input method takes precedence. Applications without text-input-v3, XWayland applications and toolkits that do not report text-input state cannot use native assistance yet. An accessibility bridge is not implemented.

## Controls

- ↑ / ↓: select a suggestion.
- Tab: accept the selected suggestion. Escape: dismiss.
- Click a row: accept without moving keyboard focus.
- Any physical key, focus change, lock or invalidated field cancels pending typing.

Acceptance delivers one Unicode grapheme per timer tick, scheduled at 8 ms intervals (actual delivery depends on event-loop load), without a blocking sleep or clipboard fallback. Spelling correction deletes only the current prefix using UTF-8 byte counts. Selection and mid-word insertion do not trigger suggestions.

`AQUA_TEXT_ASSIST=0` hides assistance. `AQUA_T9=1` enables predictions for trailing 2–9 sequences in ordinary text fields. The T9 mapping supports Latin and Russian letters. Numeric and PIN fields are not treated as T9 fields.

A custom frequency dictionary can be placed in `~/.config/aqua/words.txt` (or `$XDG_CONFIG_HOME/aqua/words.txt`). `AQUA_DICTIONARY` overrides its location. Each line contains a word and frequency separated by whitespace. Dictionaries are loaded off the compositor thread and bounded to 100,000 entries and 8 MiB. Completions use an FST; one-edit corrections use SymSpell's Unicode strategy. Aqua never builds a dictionary from your typed text.

## Passwords and privacy

Aqua uses the existing Secret Service D-Bus API through the Rust `secret-service` library, with an encrypted DH session. It does not create a second password database. No Secret Service daemon is bundled; a compatible service such as GNOME Keyring, KeePassXC or KWallet must already be available on the session bus.

Only entries explicitly tagged `xdg:schema=org.aqua.Password` and `aqua-app-id=<application-id>` are listed. These tags prevent accidental broad enumeration; application IDs are client claims, not authenticated website origins. Browser-wide autofill is deliberately unsupported. Check the target application shown in the popup before selecting a password. Existing untagged passwords are not automatically imported or searched.

Listing requires an explicit click/Tab on “Passwords…”. A secret is fetched only after selecting an entry; a locked store may ask you to unlock it. Work happens off the compositor thread. A response from an old focus/field generation is discarded. Password/PIN and hidden/sensitive surrounding text is not retained, indexed, logged or exposed over IPC. Temporary secret buffers and queued typing are zeroized on drop. The Wayland transport necessarily carries the selected secret to the receiving application; Rust heap zeroization cannot guarantee removal of all transport/library copies.

The native IME and virtual-keyboard globals are hidden from sandbox clients. Local unsandboxed processes still share the desktop user's trust boundary.

## Testing

`cargo test -p aqua-text -p aqua-compositor` covers offsets, Unicode graphemes, dictionary bounds/ranking, T9, sensitive-state clearing, generation cancellation and popup placement. `tests/input-assist.sh` exercises a real Wayland text-input client in an isolated nested compositor and captures screenshots without changing the live session.

To test password selection with **test data only**, build `cargo build --release -p aqua-text --example password_fixture` and run `tests/input-assist.sh target/release/aqua /tmp/aqua-input-assist password-fill` (requires `gnome-keyring-daemon`). The script uses a private D-Bus, runtime directory and HOME; it never enumerates your real password store. `password-cancel` verifies cancellation after the first grapheme.
