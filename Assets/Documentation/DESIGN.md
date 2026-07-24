# Wooosh — System Design

AirDrop-style local file sharing for Android, iOS, macOS, and Windows. Minimal UI, native on every platform, end-to-end encrypted, no accounts, no servers to run.

Companion document: [PROTOCOL.md](PROTOCOL.md) — wire protocol, discovery format, and the key-sharing/pairing cryptography.

---

## 1. Requirements

### Functional
- Automatic discovery of nearby devices on the same LAN (AirDrop-like, zero configuration).
- Send/receive any file type, 1 file or thousands, 500 kB or 4 GB+.
- Secure pairing (key sharing) so transfers are end-to-end encrypted and devices are mutually authenticated; man-in-the-middle on open Wi-Fi must be detectable.
- Received media is routed automatically:
  - **iOS**: photos/videos → Photos library; documents → Files (app's Documents folder, visible in the Files app).
  - **Android**: everything → `Downloads/` (via MediaStore).
  - **macOS / Windows**: everything → `~/Downloads`.
- Share extension on every platform (system share sheet / Share Target / ACTION_SEND).
- Device list sorted by **discovery timestamp**, never reordered; devices that vanish from discovery are **grayed out in place**, not removed (prevents mis-taps from list shifting).
- Screen stays awake during active transfers on mobile; long transfers must not lock the user out of their device.
- Off-LAN sharing paths (different networks, mobile data) without a self-hosted middleman server (§9).
- App accent color follows the system accent where the OS supports it.

### Non-functional
- Throughput: saturate typical Wi-Fi (target ≥ 40 MB/s on Wi-Fi 5, ≥ 100 MB/s on wired/Wi-Fi 6E between desktops).
- Small-file efficiency: 1,000 × 500 kB files should not cost 1,000 round trips.
- Resume after interruption (Wi-Fi drop, app kill) without re-sending completed bytes.
- Integrity: every file verified by hash before it is placed in its final location.
- Privacy: no telemetry, no cloud account, discovery broadcast contains no sensitive data.

### Constraints
- Native UI per platform: Kotlin/Jetpack Compose (Android), SwiftUI (iOS + macOS), WinUI 3 (Windows).
- No backend infrastructure operated by the project.

---

## 2. High-level architecture

**Core decision: one shared Rust core, four native shells.**

The protocol, cryptography, discovery, and transfer engine live in a single Rust library (`wooosh-core`) exposed to each platform via [UniFFI](https://mozilla.github.io/uniffi-rs/) (Kotlin + Swift bindings) and a C ABI consumed from C#/WinRT on Windows. The UI layer on every platform is 100% native.

```
┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ Android     │ │ iOS         │ │ macOS       │ │ Windows     │
│ Compose UI  │ │ SwiftUI     │ │ SwiftUI     │ │ WinUI 3     │
│ + FGService │ │ + Share Ext │ │ + Share Ext │ │ + ShareTgt  │
│ + ShareTgt  │ │ + LiveActvy │ │ + Menu bar  │ │ + Tray      │
└──────┬──────┘ └──────┬──────┘ └──────┬──────┘ └──────┬──────┘
       │ UniFFI (Kt)   │ UniFFI (Swift)│              │ C ABI / P-Invoke
┌──────┴───────────────┴───────────────┴───────────────┴──────┐
│                        wooosh-core (Rust)                    │
│  ┌──────────┐ ┌──────────┐ ┌───────────┐ ┌───────────────┐  │
│  │ Identity │ │ Pairing  │ │ Transfer  │ │ Trust store   │  │
│  │ & keys   │ │ QR + SAS │ │ engine    │ │ (pinned keys) │  │
│  │          │ │          │ │ QUIC/TLS  │ │               │  │
│  └──────────┘ └──────────┘ └───────────┘ └───────────────┘  │
│  (discovery + peer-list ordering live in the shells — below) │
│        Transport: QUIC (quinn) · TLS 1.3 · BLAKE3           │
│        Internet path: iroh (relay + hole punching)          │
└──────────────────────────────────────────────────────────────┘
   Platform adapters (injected from the shell, per platform):
   secure key storage · file staging/routing · wake locks
```

### Why a shared core (trade-off analysis)

| | Shared Rust core | 4× native protocol implementations |
|---|---|---|
| Crypto correctness | One audited implementation | Four chances to get AEAD/nonce/pinning wrong |
| Interop bugs | Impossible by construction | Cross-platform matrix testing forever |
| QUIC availability | `quinn` everywhere | Uneven: no public QUIC API on iOS < 15 / Android; Windows MsQuic differs |
| Internet P2P (iroh, §9) | Drops in directly (Rust) | Reimplement hole punching 4× — not realistic |
| Build complexity | Rust toolchain + FFI plumbing | Plain per-platform builds |
| UI nativeness | Unaffected — UI is fully native | Same |

The FFI cost is paid once; a four-way interop matrix is paid on every protocol change. **Rust core wins.** The FFI boundary is kept narrow and coarse-grained (commands in, event stream out — see §4), so the bindings stay small.

### What the shell owns vs. the core

- **Core**: pairing/crypto, transfer state machine, resume ledger, hashing, network I/O.
- **Discovery is native, not core** (implementation deviation from the original plan, adopted in Milestone 1): each shell runs its own mDNS stack — NWListener/NWBrowser on Apple, NsdManager on Android — because the OS APIs are well-supported, already interoperate byte-for-byte on the TXT format in PROTOCOL.md §3.1, and avoid shipping a second mDNS responder that fights the system one. The shell therefore owns the peer registry (discovery-timestamp ordering and stale/alive state) and hands the core a resolved address via `connect_peer(addr, expected_pubkey?)`; the core advertises nothing and reports its bound port through `listen_addr()` for the shell to publish in the TXT record.
- **Shell (native)**: all UI, file pickers, final storage routing (Photos/MediaStore/Downloads), keep-awake, foreground services / background modes, share extension entry points, notifications, key storage primitives (Keychain / Keystore / DPAPI — the core stores its identity key *through* a platform adapter so hardware-backed storage is used where available).

---

## 3. Data flow

### Send (happy path)
```
Sender                                    Receiver
  │ user picks files / share sheet          │ (idle, advertising via mDNS)
  │ taps receiver in device list            │
  ├── QUIC connect (TLS1.3, both pinned) ──►│
  ├── OFFER {manifest: names,sizes,hashes}─►│
  │                                         ├─ consent UI (auto-accept if paired+enabled)
  │◄──────── DECISION {accepted ids} ───────┤
  ├── file streams (≤4 concurrent) ────────►│ write → .part in staging
  │      small files pipelined,             │ verify BLAKE3
  │      big files single stream            │ route to Photos/Downloads/Files
  │◄──────── per-file DONE / errors ────────┤
  └── summary UI                            └── summary UI + notification
```

### First contact (pairing) — details in PROTOCOL.md §4
Unknown device → connection still gets TLS 1.3 encryption, but trust isn't established. Two paths:
1. **QR pairing** (preferred): receiver shows QR containing its public key + one-time token; sender scans → both sides pin each other. Immune to MITM because the key travels out-of-band.
2. **Numeric comparison (SAS)**: both devices derive a 6-digit code from the TLS session's exporter secret and display it; users visually compare. An active MITM produces two different codes.

Paired devices are remembered (pinned public keys) — subsequent transfers authenticate silently.

---

## 4. Core ↔ shell API contract (FFI)

Deliberately coarse: the shell never sees sockets or crypto.

```
// Commands (shell → core)
start(config: Config, adapters: PlatformAdapters)
set_visibility(mode: Everyone | PairedOnly | Off)
begin_pairing_qr() -> QrPayload
pair_with_qr(payload: QrPayload)
request_sas_pairing(peer_id)             // initiate camera-less SAS pairing (PROTOCOL.md §4.3 step 1)
confirm_sas(peer_id, accepted: bool)
send(peer_id, files: [StagedFile]) -> transfer_id
respond_to_offer(transfer_id, accepted_file_ids: [FileId])
cancel(transfer_id)
connect_peer(addr, expected_pubkey?)     // shell resolved it via native mDNS; also the Tailscale / direct-IP path, §9
listen_addr() -> String                  // bound "ip:port" for the shell to publish in its mDNS TXT record
trusted_peers() -> [TrustedPeer]         // the pinned set, read from the trust store — the shell's trust list
revoke_peer(pubkey) -> bool              // un-pin; next contact is untrusted again
fingerprint_phrase_for(pubkey) -> String // 6-word verification phrase; shells never reimplement the wordlist
device_id_for(pubkey) -> String          // rendered DeviceID == the peer_id used in events
redeem_ticket(ticket: String)            // iroh internet path, §9

// Event stream (core → shell), the single source of UI truth
PeerAppeared   { peer, discovered_at }   // core assigns the ordering timestamp
PeerStale      { peer_id }               // gray out, do NOT remove
PeerReturned   { peer_id }
PairingSas     { peer_id, six_digits }
PairingResult  { peer_id, peer_pubkey, fingerprint, success, message? }  // dismisses SAS/QR UI, updates trust list
PeerConnected  { peer_id, peer_pubkey, device_name, device_type?, fingerprint, trusted }
IncomingOffer  { transfer_id, from, manifest }
TransferStarted{ transfer_id, peer, direction: Send|Receive, manifest }  // sends: resolved manifest for progress UI
Progress       { transfer_id, file_id, bytes, rate, eta }
FileReady      { transfer_id, file_id, staged_path, kind: Photo|Video|Document }
TransferDone   { transfer_id, summary }  // incl. duration_ms of this attempt
TransferError  { transfer_id, error, resumable: bool }
KeyChanged     { peer_id, expected_pubkey, presented_pubkey? }
```

`FileReady` is the storage-routing hook: the core finishes verification in its private staging directory, then the **shell** moves the file to its platform-correct destination (§6) and reports back so staging can be cleaned.

`PlatformAdapters` (implemented natively, passed in at `start`): `key_store` (get/put identity key), `keep_awake(bool)`, `staging_dir`, `notify(event)`.

### Peer identity crosses the FFI as a public key (normative)

`peer_id` is a *one-way* BLAKE3 derivation of the peer's key, so it cannot be fed back into `connect_peer` / `revoke_peer`. Every event that names a peer therefore also carries `peer_pubkey` (raw 32 bytes), and `trusted_peers()` exposes the pinned set. Two consequences for shells:

- **Never mirror the trust store.** Render and revoke from `trusted_peers()`; a shell-side copy drifts from `trust.json` the moment pairing happens on the other side of a QR.
- **Never re-derive core crypto.** `fingerprint_phrase_for(pubkey)` and `device_id_for(pubkey)` are exported for exactly that reason; reimplementing the wordlist in Swift/Kotlin is a conformance bug waiting to happen.

Pinning itself is *not* delegated to the shell: passing `expected_pubkey = null` to `connect_peer` does not disable it. The core resolves the identity behind the address from its own trust store and re-applies the pin, so a peer paired over SAS (which never hands the shell a key at pairing time) is protected even if the shell forgets. Supplying the key explicitly is still preferred — it pins the very first reconnect, before the core has seen that address. See PROTOCOL.md §4.5.

### Threading contract (normative)

**Every exported call is synchronous and blocking. None of them may be called on a UI thread.**

| Call | Blocks on |
|---|---|
| `start` | the host `KeyStore` (invoked **synchronously, on the calling thread**), then binding the QUIC endpoint |
| `pair_with_qr` | full QUIC handshake + PAIR_ACCEPT — worst case ≈10 s per address hint + 20 s reply timeout |
| `connect_peer` | QUIC handshake + HELLO exchange — up to ≈10 s on an unreachable address |
| `send` / `resume_transfer` | the core runtime while the transfer is registered (streaming itself is asynchronous) |
| `stop` | ≈2 s runtime shutdown, then joining the event thread |

A host `KeyStore` implementation **may block** — Keychain, Android Keystore and DPAPI all can, arbitrarily long on first unlock or behind a biometric prompt. That is why `start` in particular must be dispatched to a background executor (the iOS shell froze its main thread inside a Keychain call before this was written down). Hop back to the UI thread with the result.

Conversely, `CoreEventListener.on_event` is always delivered on the core's own event thread, never on the caller's, so shells must marshal to the UI thread there.

---

## 5. Device list UI (all platforms)

The core maintains the canonical list; shells render it verbatim.

- **Ordering**: strictly by `discovered_at` of the *first* sighting in this app session. New devices append to the bottom. Order never changes afterward — no re-sorting on rename, RSSI, or re-announce.
- **Staleness**: a peer missing from mDNS for > 10 s (2 missed announce intervals + grace) emits `PeerStale`. UI grays the row, disables hit-testing, keeps its position and height. `PeerReturned` restores it in place. Rows are only cleared when the app is relaunched or the user pulls-to-refresh explicitly.
- **Row content**: device-type icon (phone/tablet/laptop/desktop), device name, state line (Ready · Paired ✓ · Away · Receiving…). A paired-checkmark, not a separate section — sections would move rows.
- **Tap** → file picker (or immediate send when launched from the share extension).
- **Incoming offer** → modal sheet: sender name + verification state, file count/total size, thumbnail strip, Accept / Decline. Unpaired senders additionally show the fingerprint code with a "Pair & remember" option.
- Empty state explains visibility mode and offers the QR pairing button.

Platform notes:
- **Android**: Compose `LazyColumn`, `animateItem` disabled for reorder (there is none), Material 3 dynamic color.
- **iOS/macOS**: SwiftUI `List` bound to an `@Observable` store fed by the core event stream; `.disabled(true)` + reduced opacity for stale rows.
- **Windows**: WinUI 3 `ListView` with `x:Bind` to an `ObservableCollection` that is append-only.

### Accent color
- **Android**: Material You dynamic color (`dynamicLightColorScheme` / `dynamicDarkColorScheme`, API 31+; static fallback below).
- **Windows**: use `SystemAccentColor` theme resources (`AccentFillColorDefaultBrush` etc.) — free with WinUI 3.
- **macOS**: ship **no** custom `AccentColor` asset → `Color.accentColor` automatically follows the user's system accent.
- **iOS**: iOS has no user-selectable system accent. Use the default tint (system blue) rather than a brand color, so it matches system apps. (Documented limitation, not a bug.)

---

## 6. Storage routing (shell responsibility, on `FileReady`)

| Platform | Photos & videos | Everything else |
|---|---|---|
| iOS | `PHAssetCreationRequest` into the Photos library (permission: add-only `PHPhotoLibrary` access) | App `Documents/` with `UIFileSharingEnabled` + `LSSupportsOpeningDocumentsInPlace` → shows up in Files under *On My iPhone → Wooosh* |
| Android | `MediaStore.Downloads` (API 29+; `IS_PENDING=1` during move, then publish) | Same — everything goes to `Downloads/` per requirement |
| macOS | `~/Downloads` (user-selected-file access via one-time folder grant if sandboxed) | `~/Downloads` |
| Windows | `KnownFolders.DownloadsFolder` | Same |

Rules:
- Classification by MIME sniff + extension (core supplies `kind`, shell may override).
- Name collisions: append ` (2)`, ` (3)`… — never overwrite.
- The move is atomic per file: verify hash → move/insert → only then report success to the sender. A transfer is never reported complete for a file the user can't find.
- Multi-file receives on Android/desktop optionally land in a `Wooosh/<date>` subfolder of Downloads when count > 20 (keeps Downloads usable); single files always go to Downloads root.

---

## 7. Transfer engine & performance

Transport is **QUIC** (quinn) with TLS 1.3 — one UDP socket, multiplexed streams, no head-of-line blocking between files. Full framing in PROTOCOL.md §5–6.

- **Big files (4 GB+)**: one QUIC stream per file, 64-bit lengths everywhere. Receiver preallocates (`fallocate` / `F_PREALLOCATE` / `SetFileInformationByHandle`), streams to `<name>.part` in staging, BLAKE3 hashed incrementally (BLAKE3 is fast enough to never be the bottleneck; multithreaded for local hashing on send).
- **Many small files**: the manifest is sent once up front; accepted files are then **pipelined** over a small pool of streams (≤ 4) with no per-file round trip — stream framing carries `file_id` headers back-to-back. 1,000 files ≈ 1 RTT of control overhead total, not 1,000.
- **Flow control tuning**: initial per-stream window 8 MiB, connection window 32 MiB, UDP GSO/GRO where the OS supports it. These numbers are the "revisit later" knobs — benchmark per platform.
- **Resume**: receiver keeps a ledger `{transfer_id, file_id → verified_offset}` — an optimization only, whose failures are never fatal to a transfer (authority is the `.part` bytes + BLAKE3; see PROTOCOL.md §6). On reconnect the sender re-offers with the same `transfer_id`; receiver replies with per-file offsets (it re-hashes the `.part` prefix to trust its own ledger); sender seeks and continues. Survives app kill because the ledger is persisted with the `.part` files.
- **Integrity**: per-file BLAKE3 in the manifest, verified before routing. Mismatch → the file (only that file) is re-requested once, then failed.
- All disk I/O on blocking worker threads (Rust `tokio` + `spawn_blocking`), never on the FFI or UI threads.

### Keep-awake & long-running behavior

| Platform | Screen on during transfer | App keeps running |
|---|---|---|
| Android | `FLAG_KEEP_SCREEN_ON` on the activity | **Foreground service** (`dataSync` type) with progress notification + partial wake lock — transfers survive screen-off and app-switch; this is the platform-blessed path |
| iOS | `UIApplication.isIdleTimerDisabled = true` while active | Foregrounded: fine. Backgrounded: `beginBackgroundTask` buys ~30 s to wind down/persist resume state; **Live Activity** shows progress on the Lock Screen. **Opt-in PiP workaround**: a minimal `AVPictureInPictureController` session (rendering the progress UI as its content) keeps the process executing while the user does other things — shipped behind a setting labeled honestly ("Keep transfers running in background (uses Picture in Picture)"), because it is an App Review risk and shouldn't be the silent default. Resume (§ above) is the real safety net either way |
| macOS | n/a | `NSProcessInfo.beginActivity(.idleSystemSleepDisabled, …)` during transfers; app lives in Dock and/or menu bar |
| Windows | n/a | `SetThreadExecutionState(ES_CONTINUOUS \| ES_SYSTEM_REQUIRED)` during transfers; minimizes to tray and keeps receiving |

iOS receiving requires the app (or its PiP session) to be alive — this is an iOS platform constraint, same one AirDrop solves with OS privileges we don't have. The UI communicates it ("Keep Wooosh open while receiving").

---

## 8. Share extensions

| Platform | Mechanism |
|---|---|
| iOS | Share Extension target. Extension memory is tight (~120 MB), so it never transfers by itself: it copies/bookmarks the shared items into the **App Group** container, then opens the main app via deep link (`wooosh://send?batch=…`) with the device list pre-filtered to alive peers. |
| macOS | Share Extension, same App Group handoff, but macOS extensions may also complete small sends inline (no memory pressure). |
| Android | `ACTION_SEND` / `ACTION_SEND_MULTIPLE` intent filters (all MIME types) + **Direct Share**: paired devices published as sharing shortcuts (`ShortcutManagerCompat.pushDynamicShortcut`) so they appear as one-tap targets in the system share sheet. |
| Windows | **Share Target** contract in the package manifest (activation kind `ShareTarget`), plus an optional File Explorer context-menu verb ("Send with Wooosh") for classic right-click flows. |

All four funnel into the same core `send()` call; the share UI is a trimmed device-list screen (same ordering/staleness rules).

---

## 9. Off-LAN sharing (no self-hosted server)

Ranked; ship in this order. All of them reuse the exact same pairing trust model and transfer protocol — only the path packets take changes.

### 9.1 Primary: iroh tickets (internet P2P, free public relays)
[iroh](https://iroh.computer) is a Rust QUIC stack whose node identity *is* an Ed25519 key — the same shape as Wooosh's identity (PROTOCOL.md §2). It performs NAT hole punching with relay-assisted rendezvous, falling back to relaying E2E-encrypted traffic through n0's free public relay infrastructure when punching fails (symmetric NAT, CGNAT on mobile data). Nothing to deploy; traffic stays end-to-end encrypted so relays see only ciphertext.

UX: sender taps **Share via internet** → app generates a **ticket** (compact string / QR encoding node ID + relay hint). Receiver pastes/scans it in **Receive from internet** → direct QUIC connection → identical OFFER/ACCEPT flow, plus SAS verification since the peers may be unpaired. Works across accounts, networks, and mobile data. The ticket is exchanged over any channel the users already share (Messages, email, in person).

Trade-off: dependency on n0's public relay availability (mitigated: relay URL is a config value; users *can* point at any relay, including a self-hosted one, but never need to).

### 9.2 Power-user: Tailscale as the network
If both users run Tailscale and share nodes across their tailnets (Tailscale's built-in cross-account node sharing invite — free tier), each device just has a stable WireGuard-encrypted IP. Wooosh needs almost nothing: listen on all interfaces (already true) and offer **Add device manually** (`add_manual_peer(host)`) accepting a Tailscale IP/MagicDNS name. mDNS doesn't cross tailnets, so discovery is manual, but pairing and transfer are unchanged — and Wooosh's E2E crypto still applies on top of WireGuard. Zero code beyond the manual-add field; document it as a recipe.

### 9.3 Zero-infrastructure fallback: wormhole-style codes
Adopt the Magic Wormhole model: a short human-speakable code (`7-guitar-sunset`) performs a PAKE (SPAKE2) through the public wormhole mailbox/relay servers (free, community-run, tiny traffic). Good as a compatibility story (croc/wormhole users) but it overlaps iroh's job with a worse trust ceremony — hold unless users ask.

### 9.4 Same room, no network at all
When there's no common LAN (field work, flights, no router):
- **Apple ↔ Apple**: Multipeer Connectivity / AWDL — works with Wi-Fi/Bluetooth on, no network needed. Used as an alternate transport under the same protocol.
- **Android/Windows ↔ anyone**: the sending device raises a **local-only hotspot** (Android `startLocalOnlyHotspot`, Windows Mobile Hotspot) and shows a QR encoding SSID + password + its listen address; iPhones join via the camera QR and land on a normal LAN transfer. Clunkier, but universal.

**Not chosen**: WebRTC with copy-paste SDP signaling (truly serverless but fails on symmetric NAT without TURN and the UX is dreadful); Tailscale Funnel (exposes a public HTTPS endpoint — wrong privacy posture for this app); Bluetooth transfers (too slow for the 4 GB requirement).

---

## 10. Security summary (threat model in PROTOCOL.md §7)

- Every connection is TLS 1.3 (QUIC) with **both** peers presenting keys; trust is key pinning, not CAs.
- First contact is authenticated by QR (out-of-band key exchange) or SAS numeric comparison (MITM detection); after that, pinned keys authenticate silently.
- Visibility modes: **Everyone** (announce + accept offers from unpaired with consent), **Paired only** (announce, but reject unpaired connections at the handshake — with a carve-out for a pending QR/SAS pairing the user just initiated, PROTOCOL.md §4.2, else the mode would block its own pairing flow), **Off** (no announcements, no listener).
- Discovery broadcasts contain only: display name, device type, protocol version, port, and a *rotating* discovery ID — the long-term identity key is never broadcast, so passive listeners can't track a device across networks by key.
- Files never touch their final destination until hash-verified; staging is app-private.
- No accounts, no cloud, no telemetry.

---

## 11. Testing strategy

- **Interop matrix in CI**: the Rust core makes this cheap — a headless `wooosh-cli` (same core) runs sender/receiver pairs in CI across Linux/macOS/Windows runners for every protocol change; device-lab runs (Android/iOS) per release.
- **Chaos suite**: simulated packet loss/latency (`netem`), mid-transfer Wi-Fi drop, app kill + resume, clock skew, disk-full on receiver, 10k × 100 kB and 1 × 8 GB fixtures.
- **Fuzzing**: CBOR control-message parser and manifest parser fuzzed (cargo-fuzz) — these are the attack surface reachable pre-consent.
- **Benchmarks**: throughput regression gates on loopback + reference Wi-Fi hardware.

## 12. What to revisit as it grows
- Flow-control window sizes and stream-pool size per platform (measure, don't guess).
- iOS background story if Apple ever grants a proper background-transfer entitlement (drop the PiP workaround the day that happens).
- Directory/folder transfers with structure preservation (manifest already carries `relPath` — UI work only).
- Trusted-device sync across a user's own devices (currently pair each pair of devices; could gossip trust between already-paired devices).
- Protocol versioning is in place from day one (`v` in discovery TXT + HELLO); v2 candidates: compression negotiation for compressible types, delta transfer for re-sends.
