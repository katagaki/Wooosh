# Wooosh — Implementation Status

Last updated: 2026-07-24. Specs: [DESIGN.md](DESIGN.md), [PROTOCOL.md](PROTOCOL.md).

## Summary

| Component | Builds | Runtime verified | Notes |
|---|---|---|---|
| `Core/` (Rust) | ✅ | ✅ 25/25 unit, 11/11 integration | QUIC + pairing + transfers + resume |
| `Apple/` macOS | ✅ | ✅ transfers to iOS and to Android, plus both directions with the CLI | unsandboxed run only |
| `Apple/` iOS | ✅ | ✅ on the user's iPhone: transfers, **Photos library routing**, **camera QR scanning** | share extension with a real App Group still unverified |
| `Android/` | ✅ | ✅ **physical Pixel 7a**: discovery, send **and receive**, pairing, revoke, KEY_CHANGED | — |
| Windows (WinUI 3) | — | — | not started; needs a Windows machine |

Build commands: `cargo test --release` (PATH needs `/opt/homebrew/opt/rustup/bin`), `xcodebuild -scheme Wooosh -destination 'platform=macOS'`, `JAVA_HOME=<Android Studio jbr> ./gradlew assembleDebug`. FFI artifacts rebuild via `Core/build-bindings.sh`.

## What is proven working

**All three shipped platforms interoperate on real hardware.** iOS ↔ Android works in both directions (sending and receiving each way), and macOS transfers to both iOS and Android. Every pairing of platforms in the project has now moved real files. This is the headline result. It exercises the full stack app-to-app with no CLI in the loop: native mDNS discovery on both sides interoperating on the shared TXT format, QUIC/TLS 1.3 with mutual Ed25519 pinning, the offer/consent flow, and platform-correct storage routing at each end (Photos or Files on iOS, Downloads via MediaStore on Android). It also closes the last untested path in the project: Android *receiving* core-produced bytes, which neither an emulator nor this Wi-Fi could previously exercise.

Real end-to-end transfers between the native apps and `wooosh-cli` (same protocol, independent implementation):
- **macOS ↔ CLI**: QR pairing, app→CLI 8 MB at ~50 MB/s, CLI→app into `~/Downloads`, mDNS discovery → `connect_peer`, all SHA-256 verified. Clean quit in 0.03 s.
- **Android ↔ CLI**: QR pairing, discovery→connect→send, revoke, and a real `KEY_CHANGED` (started a different CLI identity on the same address — transfer failed closed with both fingerprints shown).
- **Core**: 200 MiB + 500×100 KiB in one transfer (83 MB/s receiver-side), resume-after-kill without re-sending verified bytes, SAS codes provably diverge under a relayed MITM, pin mismatch detected.

Identity is unified: one Ed25519 keypair per install, held by the core, stored through the shell's Keychain/Keystore. Device IDs are BLAKE3-derived (the SHA-256 placeholder is gone from both shells).

## Bugs found and fixed during integration

1. **Shutdown deadlock** (`api.rs`): `stop()` joined the event thread while still holding the engine that owned the channel's sender. Would have hung the app on quit.
2. **Missing tokio runtime context**: sync FFI calls arrive on host threads with no reactor, so arming the SAS timeout panicked → poisoned mutex → panic in a destructor → **process abort**. Would have hard-crashed both mobile apps on camera-less pairing.
3. **Ledger write race** (`ledger.rs`): a fixed temp filename plus load-mutate-save from disk, called concurrently by up to 4 slot streams, corrupted the ledger; the error then tore down a slot stream and stranded every file queued behind it, so large mixed transfers never completed.
4. **Opaque rejections**: QUIC `close()` discards buffered data, so a rejecting peer's HELLO/`PAIR_REJECT` never arrived. Rejections are now carried by close codes mapped to typed errors.
5. **SAS-paired peers connected unpinned** — the shell had no way to obtain a peer's public key, so §4.5 pinning silently did not apply to them. The core now pins from its own trust store, and public keys are exposed on events.
6. **`start()` blocked the UI thread** inside the host Keychain call (Apple). The threading contract is now documented and normative.
7. **Android 17 launch crash** (physical device only): `NsdManager.registerService` threw `SecurityException` under the new Local Network Protections. The emulator did not enforce it. Fixed by declaring/requesting `ACCESS_LOCAL_NETWORK` and, separately, by not letting a discovery failure take the whole app down.
8. **A SAS pairing could be confirmed without user intent** (Apple, security). `SheetActions` applied `.keyboardShortcut(.defaultAction)` to whatever sat in the primary slot, and on the SAS sheet that slot is "Codes Match" → `confirmSAS(accepted: true)`. The sheet is presented by a **remote-triggered** event (`pairingSAS` arrives from the peer, no local action required), so it materialises under the user's hands and immediately owns the key window's default button: a Return aimed at anything else — a queued keystroke, an autorepeat, Return in a text field — confirmed the pairing. `.interactiveDismissDisabled()` left keystrokes nowhere else to go. This defeats the entire SAS ceremony, whose only purpose is a deliberate human comparison. Fixed by opting the SAS confirm button out of the default-action shortcut; every other sheet keeps Return.
9. **QR pairing looked broken in the field**: hints were dialled *serially* at 10 s each and the QR embedded a `127.0.0.1` hint that a scanner dials against its own loopback, so a real pairing sat silent for ~19 s and the user force-quit. Hints are now raced concurrently (two dead hints ahead of a live one: 19 s → 1.1 ms), the connect budget is 6 s, and all seven previously-silent failure paths now emit `PairingResult{success:false}`. The same bug was inflating the whole test suite: ~55 s → ~12 s.

## Known gaps / next steps

**Needs a human at the keyboard**
- iOS share extension with a real App Group (needs signed builds). Photos-library routing and the camera QR scanner are **confirmed working on device** by the user.
- ~~Android receive-from-remote~~ **done**: verified iPhone → Pixel 7a, and macOS → Pixel 7a, on real hardware. (Kept as a note because it was long-blocked: emulators cannot route inbound UDP. An agent also reported Mac → phone timing out on this Wi-Fi; that has since been contradicted by a working macOS → Android transfer, so the earlier failure was situational rather than a network policy.)
- Sandboxed macOS run end to end (ad-hoc re-signing invalidates the container ACL between rebuilds).

**Core follow-ups (queued, not blocking)**
- `PeerDisconnected` only fires on QUIC idle timeout (>20 s), so rows show as connected after a peer is gone.
- `PeerConnected` carries no remote address, so a freshly discovered row can't be tied to a pinned DeviceID before the first connect — that first tap falls back to the core's `last_addr` pin resolution.
- `PairingResult.message` is overloaded (device name on success, reason on failure).
- `PairingSas` / `TransferStarted` carry only `peer_id`; every other peer-bearing event now self-describes.
- `connect_peer` both throws `KeyChanged` and emits the event, forcing shells to suppress one.

**In flight / immediately queued**
- Liquid Glass redesign on iOS **and** macOS, minimum OS raised to 26, larger sheet buttons, photo/video filename retention, QR tap-to-focus, platform-explicit device icons, and pairing progress/cancel UI — one agent per shell.
- **`Core/build-bindings.sh` must be re-run** once those land: the pairing fix changed `Core/src` but not the FFI surface, so `bindings/` and `dist/` are stale. Then reinstall to the Pixel and retry app-to-app pairing.
- Copy and localization pass across all shells per [COPY_STYLE.md](COPY_STYLE.md) (en, ja, ko, zh-Hans, zh-Hant, de, es, fr, it, nl, pt-BR, pl, ru, sv).
- Align the core's `DeviceType` enum with the new platform-explicit `dt` vocabulary (PROTOCOL.md §3.1); the shells already use their own TXT-derived type, so this is HELLO parity only.

**Features not started**
- Windows / WinUI 3 app (needs a Windows machine).
- Internet path (iroh tickets, DESIGN.md §9.1) — the design is settled, no code exists.
- UDP broadcast discovery fallback (PROTOCOL.md §3.2).
- SAS pairing has no UI entry point on either platform (the plumbing exists end to end).
- `resume_transfer` is never called by either shell; a dropped transfer surfaces as a resumable error with no retry affordance.
- Staging/outbox cleanup after sends.

## Conventions worth knowing
- Discovery is **native per platform**, not in the core (DESIGN.md §2). Shells own the peer registry and the append-only/gray-out list rules.
- `MockCore` still exists on both platforms behind a debug toggle for demoing without a peer; `RealCore` is the default.
- Neither shell may reimplement core crypto — use the exported `fingerprint_phrase_for` / `device_id_for`.
