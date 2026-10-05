# Wooosh — Implementation Status

Last updated: 2026-10-05. Specs: [DESIGN.md](DESIGN.md), [PROTOCOL.md](PROTOCOL.md), [COPY_STYLE.md](COPY_STYLE.md).

## Summary

| Component | Builds | Runtime verified | Notes |
|---|---|---|---|
| `Core/` (Rust) | ✅ | ✅ 37/37 unit, 22/22 integration (+1 `#[ignore]`d, passing against the Worker under `cf dev`) | QUIC + pairing + transfers + resume; internet path through Wooosh Rendezvous with in-house hole punching and a capped relay (PROTOCOL.md §9). iroh removed |
| `Apple/` macOS | ✅ | ✅ transfers to iOS and to Android, both directions with the CLI | unsandboxed run only; internet transfer never run across networks |
| `Apple/` iOS | ✅ | ✅ on device: transfers, Photos routing, camera QR scanning; launches clean in the iOS 26.5 Simulator and discovers a real peer | share extension with a real App Group unverified; internet transfer never run across networks |
| `Android/` | ✅ | ✅ physical Pixel 7a: discovery, send and receive, pairing, revoke, KEY_CHANGED, QR scanning | internet transfer never run at all, on any network |
| `Worker/` (Cloudflare) | ✅ (40 KiB gzipped) | ✅ 22/22 in the Workers runtime; end to end with the core under `cf dev` | Never deployed. Production secrets and `PLAY_CERT_SHA256` not set |
| `Windows/` (WinUI 3) | ❌ never compiled | — | scaffold only, written on a Mac; needs a Windows machine |

Build commands: `cargo test --release` (PATH needs `/opt/homebrew/opt/rustup/bin`), `xcodebuild -scheme Wooosh -destination 'platform=macOS'`, `JAVA_HOME=<Android Studio jbr> ./gradlew assembleDebug`. FFI artifacts rebuild via `Core/build-bindings.sh`.

**Binding freshness gotcha.** Apple references `Core/bindings/swift/` and `Core/dist/WoooshCore.xcframework` in place, so it picks up a core rebuild automatically. **Android copies** the Kotlin bindings and `jniLibs`, so after every `build-bindings.sh` they must be re-copied into `Android/app/src/main/` or the app silently keeps running an old core. This has already bitten once.

## What is proven working

**All three shipped platforms interoperate on real hardware.** iOS ↔ Android in both directions, and macOS to both. Every pairing of platforms in the project has moved real files. This exercises the full stack app-to-app with no CLI in the loop: native mDNS discovery on both sides agreeing on the shared TXT format, QUIC/TLS 1.3 with mutual Ed25519 pinning, the offer/consent flow, and platform-correct storage routing at each end.

Against `wooosh-cli` (same protocol, independent implementation):
- **macOS ↔ CLI**: QR pairing, app→CLI 8 MB at ~50 MB/s, CLI→app into `~/Downloads`, mDNS discovery → `connect_peer`, all SHA-256 verified. Clean quit in 0.03 s.
- **Android ↔ CLI**: QR pairing, discovery→connect→send, revoke, and a real `KEY_CHANGED` (a different CLI identity on the same address: transfer failed closed with both fingerprints shown).
- **Core**: 200 MiB + 500×100 KiB in one transfer (83 MB/s receiver-side), resume-after-kill without re-sending verified bytes, SAS codes provably diverge under a relayed MITM, pin mismatch detected.

Identity is unified: one Ed25519 keypair per install, held by the core, stored through the shell's Keychain/Keystore. Device IDs are BLAKE3-derived.

## Bugs found and fixed

1. **Shutdown deadlock** (`api.rs`): `stop()` joined the event thread while still holding the engine that owned the channel's sender. Would have hung the app on quit.
2. **Missing tokio runtime context**: sync FFI calls arrive on host threads with no reactor, so arming the SAS timeout panicked, poisoning a mutex, then panicking in a destructor: **process abort**. Would have hard-crashed both mobile apps on camera-less pairing.
3. **Ledger write race** (`ledger.rs`): a fixed temp filename plus load-mutate-save from disk, called concurrently by up to 4 slot streams, corrupted the ledger. The error then tore down a slot stream and stranded every file queued behind it, so large mixed transfers never completed.
4. **Opaque rejections**: QUIC `close()` discards buffered data, so a rejecting peer's HELLO/`PAIR_REJECT` never arrived. Rejections now travel as close codes mapped to typed errors.
5. **SAS-paired peers connected unpinned**: the shell had no way to obtain a peer's public key, so §4.5 pinning silently did not apply to them. The core now pins from its own trust store, and public keys are exposed on events.
6. **`start()` blocked the UI thread** inside the host Keychain call (Apple). The threading contract is now normative.
7. **Android 17 launch crash** (physical device only): `NsdManager.registerService` threw `SecurityException` under the new Local Network Protections; the emulator did not enforce it. Fixed by declaring and requesting `ACCESS_LOCAL_NETWORK`, and separately by not letting a discovery failure take the whole app down.
8. **A SAS pairing could be confirmed without user intent** (Apple, security). The primary sheet button carried `.keyboardShortcut(.defaultAction)`, and on the SAS sheet that button is "Codes Match". That sheet is presented by a **remote-triggered** event, so it appears under the user's hands and immediately owns the key window's default button: a Return aimed at anything else confirmed the pairing. This defeats the entire ceremony, whose only purpose is a deliberate human comparison. Fixed by opting that one button out; every other sheet keeps Return.
9. **QR pairing looked broken in the field**: hints were dialled serially at 10 s each, and the QR embedded a `127.0.0.1` hint that a scanner dials against its own loopback. A real pairing sat silent for ~19 s and the user force-quit. Hints are now raced concurrently (two dead hints ahead of a live one: 19 s → 1.1 ms), the connect budget is 6 s, and all seven previously-silent failure paths emit `PairingResult{success:false}`. The same bug was inflating the test suite: ~55 s → ~12 s.
10. **The sender never showed its own fingerprint** while the receiver was asked to compare it. The core only emits `TransferStarted` after the receiver's decision, so during the consent window the sending device had no UI state at all. A verification step the user cannot perform is worse than none.
11. **Confirming a SAS left its 60 s countdown running** (Android), so the timer would later abort a pairing the user had already approved.
12. **An unreachable relay published a ticket carrying only local addresses** — a code that looks valid and can only ever work on the publisher's own network. Found by a test that took 15 s to fail for the wrong reason. Publishing now fails loudly instead.
13. **The internet path was unreachable from Android's empty state** (found by running it, not by review). The "Other Device…" row lives inside the peer list, and the empty state *replaces* that list, so with no devices nearby there was no way to start an internet transfer at all. The Apple empty state had the affordance; Android did not. It compiled and read fine either way.
14. **`PairedOnly` killed every internet transfer** (`engine.rs`) — the shipped default on all three shells. The post-HELLO check carved out a pending ticket; the OFFER check did not, and this path never pairs, so `trusted` is always false. The token was redeemed *and burned*, then the connection closed on the OFFER, leaving a dead code. Every pre-existing internet test used `Visibility::Everyone`, which is why it survived. Both directions now tested under `PairedOnly`.
15. **Android treated ticket redemption as pairing.** `PairingManager` waited for a `PairingResult` this path never emits (it emits `TicketRedeemed`), so a working transfer sat behind a spinner until a 75 s watchdog reported a false "pairing timed out" — labelled "Pairing" throughout, which §9.4 says it is not. A stale `api.rs` doc comment promising `PairingResult` is what led the shell there, having already propagated into the generated bindings and an Apple comment.
16. **A second internet send silently did nothing** (Android): the recorded redemption was never cleared, so revisiting the Send tab fired against a dead code and returned early on an empty outbox. Apple already cleared it.
17. **QR scanning appeared to do nothing** (Android) — two causes behind one symptom. ZXing's separate `CaptureActivity` leaves Wooosh no UI surface between detection and the activity finishing, so no alert could appear during the one moment the user is watching. *And* its full-screen preview negotiated a 2560×1280 ultra-wide on which a real pairing QR never decoded at all: 27 s and zero decodes, versus 41 ms once the preview was square. Fixed by embedding the scanner in Compose.

## Repository conventions

- `Apple/`, `Android/`, `Core/`, `Windows/`, `Assets/`. Specs in `Assets/Documentation/`, screenshots in `Assets/Screenshots/`.
- **`Apple/Wooosh.xcodeproj` is hand-edited and authoritative. Never run `xcodegen`; never recreate `project.yml`** (deleted deliberately). Regeneration wipes signing, capabilities, and build settings.
- Discovery is **native per platform**, not in the core (DESIGN.md §2). The shells own the peer registry and the append-only, grey-out-in-place list rules.
- Shells never reimplement core crypto: use the exported `fingerprint_phrase_for` / `device_id_for`.
- No mock or fake-data implementations remain. `RealCore` is the only implementation on both platforms; the debug harnesses were removed once the real core was verified on device.
- 225 strings across 14 locales, one glossary shared by both shells.
- The Android QR scanner is composed in-app (`QrScanner.kt`), not ZXing's `CaptureActivity`. Detection must happen with the app's own UI on screen, or no alert can be shown during the handover.

## What is left

**Migration: iroh → Wooosh Rendezvous (2026-10-05)**

Decisions: the project runs one server, the Worker; Windows and the CLI attest with a TPM 2.0 rather than a hardware ID string, which would be forgeable; relayed files are capped at 1 GiB; the Apple minimum is OS 27 so every Mac has App Attest; iOS and Android ship on their stores, desktop outside them.

Done and verified locally:
- **Worker** (`Worker/`): rooms, admission on the room socket, App Attest, Play Integrity, TPM credential activation, per-device and per-network quotas, relay forwarding, no storage and no logs. `npm test`: 22 tests in the Workers runtime. The TPM MakeCredential output matches an independent implementation (tpm2-pytss's algorithm on pyca/cryptography) byte for byte; App Attest is tested against a locally minted chain in Apple's format, not a real device's attestation.
- **Core**: iroh is gone, outright rather than behind a feature flag. Internet sessions are plain quinn connections over a path-selecting socket (`rendezvous/pathsock.rs`); STUN, MAC'd candidate exchange and probes, relay batching, admission caching, and `wooosh-net:2` tickets that no longer carry IP addresses. Integration tests cover direct and relayed transfers, the 1 GiB cap, refused relaying for Unverified clients, and withdrawn or reused codes failing immediately. The `#[ignore]`d test passed against the real Worker under `cf dev`: 8 MiB hash-verified, direct and fully relayed.
- **Shells**: Apple installs an App Attest attestor and Android a Play Integrity attestor; Settings shows Internet Connections with Wooosh, My Own, and Off (the old iroh setting migrates only "Off"); new errors have copy in all 14 locales. Builds: macOS, iOS device, iOS Simulator, Android debug.

Not done:
- **Deployment.** The Worker has never run on Cloudflare. Needs `SERVER_SECRET` and the Play Integrity service account uploaded as secrets (`cf deploy --secrets-file`), `PLAY_CERT_SHA256` from Play Console, then the production URL compiled into release cores with `WOOOSH_RENDEZVOUS_URL`. Until then a build has no default service and internet transfers report it.
- **Store-side setup.** The App Attest entitlement (`production`) is now in both Apple entitlements files, so signed builds need the App Attest capability on the App ID. Android needs `wooosh.playCloudProjectNumber` in Gradle properties; without it Play Integrity stays off and the device is Unverified.
- **Real networks.** Nothing has crossed a real NAT. The check that matters is still a phone on mobile data and a laptop on home broadband, now also through the relay.
- **Real attestation.** No real App Attest object or Play Integrity verdict has been verified by the Worker. App Attest on a Developer ID Mac is unverified (phase 0 below).
- **Windows and CLI attestation (TPM).** Worker side done; neither client side exists, so both are Unverified for now.
- **NAT-PMP/PCP/UPnP mapped candidates.** Not implemented.
- **Bluetooth discovery and pairing, and paired rendezvous.** Not started.
- **Apple link flags.** `SystemConfiguration` and `Network` were added for iroh. The apps still build and link with them; whether they can now be removed is untested.

Remaining phases:

0. **Developer ID + App Attest spike.** In the developer portal, check whether a Developer ID provisioning profile for `com.tsubuzaki.Wooosh` can include App Attest; then confirm `DCAppAttestService.isSupported` is true and a `production` attestation verifies, in a notarized build on macOS 27. Repeat with `wooosh-cli` bundled at `Contents/MacOS/`. If the profile cannot carry it, Macs are Unverified (direct paths only) and the specs say so.
1. **Deploy and run across real networks**, direct and relayed.
2. **TPM attestors** for Windows (Platform Crypto Provider) and the CLI (`tss-esapi`, TBS).
3. **Bluetooth LE** discovery and SAS pairing from Pair a Device, Apple and Android together.
4. **Paired rendezvous** (Reach Over the Internet).

Known gaps that are accepted, not bugs: a standalone macOS CLI cannot attest (PROTOCOL.md §9.2.1), so it uses direct paths only; the Android Play Integrity quota (10 000 calls/day by default) needs a raise request before it binds; hole punching will fail between some networks, and those users get the relay or a clear error.

Distribution prerequisites not yet in place: notarization requires the hardened runtime, and the macOS targets currently build with `ENABLE_HARDENED_RUNTIME = NO`; a sideloaded MSIX must be signed with a certificate users' machines trust (a purchased code-signing certificate or Azure Trusted Signing), or it will not install.

**The internet path is one-shot and never pairs (rewritten 2026-07-26)**

Pairing is same-network only. An internet transfer needs a fresh code every time: the **sender** publishes it, the receiver redeems it, and it dies with the transfer. Nothing is written to either trust store, which the integration suite asserts directly. Because the path never pairs, `trusted` is false even for a legitimate transfer and cannot gate anything — the redeemed token does, at both the HELLO and OFFER checks. Normative rules in PROTOCOL.md §9.1.1 and §9.4 as of `5daa4d9` (now §9.6 and §9.8).

**Host-environment note**

`/Applications/Xcode-beta.app` is missing its `PrivateFrameworks`, so `SimulatorKit.framework` is absent and the live simulator panel plus all MCP simulator control fail. Screenshots still work through `simctl`, and the Simulator window can be opened from the stable Xcode. Fix: `sudo xcode-select -s /Applications/Xcode.app`.

**Background receiving and arrival notifications (added 2026-07-26)**

Visibility now defaults to **Paired only** on all three shells (DESIGN.md §10); the core still has no default of its own, and `wooosh-cli` keeps `--visibility everyone`.

iOS receives in the background through `BGContinuedProcessingTask` (iOS 26), which also supplies the Lock Screen / Dynamic Island Live Activity, so no ActivityKit target was added. Both mobile shells post an arrival notification that opens what landed. Caveats are in DESIGN.md §7; the load-bearing ones are that the API cannot distinguish a user pressing Stop from a system expiry, and that iOS gives no runtime guarantee.

**Needs a human at the keyboard**
- **None of the background/notification work has been run on a device.** It compiles for iOS, macOS and Android, and nothing below was observed:
  - `BGContinuedProcessingTask` is **unsupported in the Simulator**, so submission, the system Live Activity, its Stop button, and expiry behaviour all need a real iPhone. Note the open Apple bug where these tasks pause on a genuinely locked device (FB19916760).
  - Arrival notifications on both platforms: permission prompt, posting, and tap-to-open (Quick Look, the Photos hand-off, `ACTION_VIEW` on a `MediaStore` URI, and the API 26-28 media-scan path, which no current test device runs).
- **The internet path has never run between two real devices at all**, on any network. Core, FFI and both shells are wired end to end and the ticket flow is covered by six integration tests (five relay-free, one against n0's real relays), but every run has been two nodes on one Mac — which exercises the protocol, not NAT traversal, and did not catch bug 14. The Android shell fixes for bugs 15 and 16 have only ever been compiled. The real check is a phone on mobile data redeeming a code from a Mac on home broadband.
- **The relayed-size cap has never fired in a real scenario.** On one machine a direct path always forms, so the uncapped path is well covered but the 100 MiB relayed cap (PROTOCOL.md §9.1.1 as of `5daa4d9`) is not. Reproducing it needs two genuinely hostile NATs, or a build with hole punching disabled.
- **A self-hosted relay has not been run.** That a chosen relay is *advertised* in the ticket follows from the one line that publishes the endpoint's home relay, and validation plus the unreachable-relay failure are tested; that a real self-hosted relay carries an introduction end to end is not. It needs an actual `iroh-relay` deployment.
- iOS share extension with a real App Group (needs signed builds).
- Sandboxed macOS run end to end (ad-hoc re-signing invalidates the container ACL between rebuilds).
- macOS Settings via ⌘, was never confirmed to actually open; the scene compiles.

**Windows**
- Nothing has been compiled. First step is opening the solution in Visual Studio and getting a build.
- Unfinished by design: the UniFFI record/enum codecs and the `KeyStore` / `CoreEventListener` callback VTables. Field order must match Rust exactly, so this needs the real header in front of you.
- Most likely to be wrong: the `StreamSocketListener`-as-registration-anchor trick in `DnssdAdvertiser`, and the DNS-SD property shapes.

**Core follow-ups (queued, not blocking)**
- `PeerDisconnected` only fires on QUIC idle timeout (>20 s), so rows show as connected after a peer is gone.
- `PeerConnected` carries no remote address, so a freshly discovered row cannot be tied to a pinned DeviceID before the first connect.
- `PairingResult.message` is overloaded (device name on success, reason on failure).
- `PairingSas` / `TransferStarted` carry only `peer_id` while every other peer-bearing event self-describes.
- `connect_peer` both throws `KeyChanged` and emits the event, forcing shells to suppress one.
- Align the core's `DeviceType` enum with the platform-explicit `dt` vocabulary (PROTOCOL.md §3.1). The shells already use their own TXT-derived type, so this is HELLO parity only.

**Features not started**
- UDP broadcast discovery fallback (PROTOCOL.md §3.2), unimplemented on every platform.
- SAS pairing has no UI entry point on either platform, so it only ever appears when the other side initiates.
- `resume_transfer` is never called by either shell; a dropped transfer surfaces as a resumable error with no retry affordance.
- Staging and outbox cleanup after sends.
- Folder transfers with structure preserved (the manifest already carries `rel_path`).
