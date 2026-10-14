# Text assistance

Aqua provides native glass suggestions, spelling correction, optional T9 and explicit password selection. No browser extension, OCR, clipboard injection or learning from typed text is used.

## Application support

The focused, enabled `text-input-v3` transaction supplies surrounding text, UTF-8 cursor/selection offsets, sensitivity hints and a caret rectangle. An external input method takes precedence. `AQUA_TEXT_ASSIST=0` disables assistance, including accessibility discovery.

An off-thread AT-SPI bridge supplements incomplete native context and supports accessible GTK fields without a native input method. Discovery is bounded and restricted to the focused process family, matching active window and focused editable widget. D-Bus failures do not block the compositor. Native delivery is preferred; accessibility-only delivery requires `EditableText` support. Applications that expose neither interface are unsupported. Accessibility must be enabled in the session.

Field bounds and caret position are separate. AT-SPI provides a rectangular field bound, not its CSS outline or corner radius. Unknown geometry stays unknown; a field rectangle may anchor a popup without being reported as a real caret. The optional `aqua-text-field-v1` protocol supplies surface-local rounded bounds and a corner radius, applied on `wl_surface.commit` and cleared on blur. Aqua's shared UI `Field` component and its Finder/Store search, rename and address controls report their rendered bounds. Arbitrary paths and uninstrumented custom canvas fields are not inferred.

## Suggestions and delivery

- ↑ / ↓ selects; Tab or a click accepts; Escape dismisses.
- Physical input, focus changes, locking and invalidated context cancel pending delivery.
- Accepted text is sent one Unicode grapheme at a time, scheduled at 8 ms intervals. Plain native edits require an observed matching text state; missing acknowledgement stops delivery rather than allowing an unbounded queue. Protected browser fields may omit text-input updates and instead require a fresh focus/protection/origin permit for every grapheme. Scheduling and accessibility calls may increase actual intervals.
- A bounded, zeroized acknowledgement history tolerates duplicate native acknowledgements without advancing the queue twice. Expected delivery text is not exposed as observed surrounding text while typing.
- Corrections replace only the validated current prefix. Selection and mid-word insertion do not trigger suggestions.

`AQUA_T9=1` enables trailing 2–9 predictions in ordinary text fields. Latin and Russian mappings are supported; numeric and PIN fields are excluded.

Completions use FST; one-edit corrections use SymSpell's Unicode strategy. A custom word-frequency dictionary can be placed in `~/.config/aqua/words.txt` or selected with `AQUA_DICTIONARY`. Loading runs off-thread and is limited to 100,000 entries and 8 MiB. Aqua never adds words from application input.

## Passwords and privacy

Aqua uses the system Secret Service through the Rust `secret-service` library and an encrypted DH session. A compatible service such as GNOME Keyring, KeePassXC or KWallet must already be available. No second password database is created; existing untagged passwords are not enumerated or imported.

Listing requires selecting “Passwords…”. Fetching requires selecting an entry. Only records tagged `xdg:schema=org.aqua.Password` and the exact scope are eligible:

- Desktop applications use `aqua-app-id`. Application IDs are client claims, not authenticated identities; check the displayed application before choosing a credential.
- Supported system-installed browsers use `aqua-origin`, a canonical HTTPS origin including a non-default port. The browser's own AT-SPI connection must belong to its verified system-owned executable. Document ancestry must agree on one origin. HTTP, unavailable document URLs, cross-origin frames and untrusted browser binaries fail closed; they never fall back to an application-wide browser scope.
- Browser password delivery revalidates the focused protected widget and origin before each grapheme. Permission requests have a deadline and do not block rendering. Chromium’s protected Disable/Enable pairs suspend delivery for at most 250 ms; re-enable preserves no authorization, and the old widget must pass a new generation’s check before another grapheme is sent. XWayland password delivery is disabled because its advertised PID is not an authenticated credential identity.

Password/PIN and hidden/sensitive surrounding text is not retained, indexed, logged or exposed over IPC. Accessibility event payloads are not decoded. Plain text context is bounded and zeroized; temporary secrets and queued graphemes are zeroized on drop. Focus generations invalidate asynchronous results. Wayland/D-Bus transport and receiving applications necessarily contain copies that heap zeroization cannot erase.

The native IME and virtual-keyboard globals are hidden from sandbox clients. Unsandboxed local processes still share the desktop user's trust boundary. Accessibility is not a security boundary against a compromised desktop session. AT-SPI exposes document URLs, not TLS certificate validation state; an HTTPS scope must not be mistaken for certificate attestation.

## Testing

`cargo test -p aqua-text -p aqua-compositor` covers Unicode offsets and graphemes, dictionary bounds/ranking, T9, sensitive-state clearing, generations, acknowledgement history and geometry.

`tests/input-assist.sh target/release/aqua /tmp/aqua-input-assist MODE` runs an isolated nested compositor with a private HOME, runtime directory and D-Bus, and captures before/after screenshots. Modes cover native Wayland, real GTK (Wayland/X11), Chromium without extensions, Russian input, T9, focus cancellation and Finder field geometry.

Password fixtures require `cargo build --release -p aqua-text --example password_fixture` and `gnome-keyring-daemon`. `password-fill` and `password-cancel` use a private test keyring; browser password modes additionally use an ephemeral local HTTPS fixture and a certificate-specific SPKI pin. They never enumerate the live password store. `browser-password-http` checks refusal of unverifiable website scope.
