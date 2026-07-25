# Wooosh

Local-network encrypted file sharing. Native apps for Android, iOS, macOS, and Windows over a shared Rust core. No accounts, no servers.

Specs are authoritative and kept in sync with the code: [Assets/Documentation/DESIGN.md](Assets/Documentation/DESIGN.md), [Assets/Documentation/PROTOCOL.md](Assets/Documentation/PROTOCOL.md), [Assets/Documentation/COPY_STYLE.md](Assets/Documentation/COPY_STYLE.md), [Assets/Documentation/STATUS.md](Assets/Documentation/STATUS.md).

## Layout
| Path | What |
|---|---|
| `Core/` | Rust workspace: `wooosh-core` (UniFFI) + `wooosh-cli`. Owns pairing, crypto, transfers. |
| `Apple/` | SwiftUI, iOS + macOS, one app target plus a share extension and the FFI framework. |
| `Android/` | Kotlin + Compose. |
| `Assets/Documentation/` | All specs and generated docs. Nothing lives in a top-level `docs/`. |
| `Assets/Screenshots/` | All screenshots, under `Apple/` and `Android/`. Never inside the platform folders. |
| Windows | Not started. Needs a Windows machine. |

Identifiers: `com.tsubuzaki.Wooosh` (Apple), `com.tsubuzaki.WoooshGo` (Android).

## Build
Rust is not on `PATH`: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.

```
cargo test --lib --release && cargo test --release --test integration -- --test-threads=1
xcodebuild build -project Wooosh.xcodeproj -scheme Wooosh -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO
JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home" ./gradlew assembleDebug
```
After changing `Core/src`, run `Core/build-bindings.sh`. Apple references `Core/` in place and updates itself; **Android copies**, so re-copy the Kotlin bindings and `jniLibs` into `Android/app/src/main/` or the app silently keeps running an old core.

## Rules

**`Apple/Wooosh.xcodeproj` is authoritative and hand-edited. Never run `xcodegen`, and never recreate `project.yml`** (it was deleted deliberately). Regeneration rebuilds the project wholesale and wipes every change made in Xcode's UI: signing, capabilities, build settings, schemes. Change the project in Xcode, or edit `project.pbxproj` directly and carefully.

**Never change `DEVELOPMENT_TEAM`.** It is `YYM4Z6MU8F` and the user owns it.

**Platform**
- Minimum OS 26 on iOS and macOS. Liquid Glass is adopted unconditionally, so there are no `#available` fallbacks.
- Native UI only: Compose (Android), SwiftUI (Apple), WinUI 3 (Windows).
- Accent follows the system accent. No `AccentColor` asset on Apple; Material You on Android.

**Device list (non-negotiable)**
- Ordered by first-discovery timestamp. Append-only. Never re-sorted.
- Peers that stop advertising grey out **in place**, disabled, never removed. Prevents mis-taps from rows shifting.
- Stale threshold is 10 s and stays 10 s. Scanning is every 2 s, so that is ~5 missed announces, not 2. Faster scanning finds devices sooner, it does not drop them sooner.

**Design**
- Liquid Glass on both Apple platforms, macOS as a first-class target and not a port: system toolbar chrome, Settings in a `Settings { }` scene (⌘,), Mac-sized controls.
- iOS sheet buttons are large, full-width, ≥44pt (aim ~50pt). macOS keeps Mac-sized buttons.
- iOS bare Done/Cancel use `Button(role: .confirm)` / `Button(role: .cancel)` with no custom label.
- Device icons come from the platform-explicit `dt` enum (PROTOCOL.md §3.1). Non-Apple phones use SF Symbol `smartphone`. Unknown values render a neutral glyph, never a guess.

**Behaviour**
- Received media routes automatically: iOS photos/videos to Photos and documents to Files; Android, macOS, and Windows to Downloads. Files are hash-verified before leaving staging.
- Original filenames are preserved, including from the photo picker. Never rename or transcode.
- Screen stays awake during transfers on mobile. Long transfers must not lock the user out of the device.
- Share extension or share target on every platform.
- Fast for both 500 kB and 4 GB files, and for any file count.

**Security**
- A verification step the user cannot actually perform is worse than none. If one side is told to compare something, the other side must display it.
- Security confirmations require a deliberate press. Never bind them to a default/Return action.
- Pinned keys are never silently re-pinned. A changed key hard-fails.
- Shells never reimplement core crypto. Use the exported `fingerprint_phrase_for` / `device_id_for`.
- Fingerprint phrases are never translated. They are a shared artifact that must read identically on both devices.

**Copy** (full spec in [Assets/Documentation/COPY_STYLE.md](Assets/Documentation/COPY_STYLE.md))
- Professional, concise, no jargon, friendly to non-technical users.
- Wooosh names itself. Never "this app" or "the app".
- No em-dashes. Rephrase.
- Title Case for buttons, headers, and titles. **English only**: do not title-case other locales.
- Everything localized: en, ja, ko, zh-Hans, zh-Hant, de, es, fr, it, nl, pt-BR, pl, ru, sv. No hardcoded display strings in views.

**Working style**
- Verify claims by running builds and tests, not by assuming.
- Report honestly what was and was not verified. Never claim runtime success that was not observed.
- Comments explain constraints the code cannot show. No narration.
- Keep the two platforms at feature and settings parity, using one term per concept.
