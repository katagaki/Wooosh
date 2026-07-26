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
  │                                         ├─ consent UI (paired senders accept silently)
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
begin_internet_ticket() -> String         // receiver publishes an iroh ticket, §9.1
end_internet_ticket()                     // withdraw it immediately
redeem_ticket(ticket: String) -> peer_id  // sender redeems, connects, then send(), §9.1
parse_internet_ticket(ticket) -> TicketInfo // label the UI before redeeming

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
| `begin_internet_ticket` | binding the iroh endpoint on first use, then up to ~15 s discovering a home relay — also the first moment Wooosh contacts any relay |
| `redeem_ticket` | binding the iroh endpoint, a ≈30 s hole-punch/dial budget, then the 20 s pairing-reply timeout |
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

**Arrival notification (mobile).** Once a received transfer has finished *and* every file has been routed, the shell posts a notification whose tap opens what arrived. It waits for routing to finish on purpose: a notification that opened a file still sitting in staging would be a lie, and routing is asynchronous, so whichever of "transfer done" and "last file routed" happens second is what posts.

| | Tapping a single received file | Several at once |
|---|---|---|
| iOS | Document: Quick Look inside Wooosh, from the URL kept at routing time. Photo or video: hands off to Photos. Add-only library access means Wooosh never holds a readable URL for what it inserted and cannot deep-link to the asset, so opening Photos itself is the honest ceiling — escalating to full library read access just to preview one photo is the wrong trade | Opens Wooosh, where the transfer card lists them |
| Android | `ACTION_VIEW` on the `MediaStore` content URI retained at insert time (`RoutedFile.uri`), with a read grant. On API 26-28 there is no insert, so the URI comes from the media scan and the notification degrades to opening Wooosh if the scan does not return one | `ACTION_VIEW_DOWNLOADS` |

Android needs `<queries>` entries for `VIEW` and `VIEW_DOWNLOADS`, or API 30+ package-visibility filtering makes every viewer invisible and the tap always falls back to Wooosh. Both platforms ask for notification permission when a transfer is actually incoming, not at launch.

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
| iOS | `UIApplication.isIdleTimerDisabled = true` while active | **`BGContinuedProcessingTask`** (iOS 26, `BackgroundTransferTask.swift`), submitted when the first incoming transfer starts. This is the sanctioned mechanism for user-initiated work that must outlive backgrounding, and it renders its own Live Activity — Lock Screen and Dynamic Island, progress bar driven by `task.progress`, Stop button included. Resume (§ above) is still the safety net |
| macOS | n/a | `NSProcessInfo.beginActivity(.idleSystemSleepDisabled, …)` during transfers; app lives in Dock and/or menu bar |
| Windows | n/a | `SetThreadExecutionState(ES_CONTINUOUS \| ES_SYSTEM_REQUIRED)` during transfers; minimizes to tray and keeps receiving |

Notes on the iOS path, because it has sharp edges:

- **No second Live Activity.** The system already draws one for the continued-processing task and Wooosh cannot restyle it. Shipping an ActivityKit activity alongside would put two progress bars for one transfer on the same Lock Screen, so Wooosh ships none.
- **One task for the whole receive session**, not one per transfer or per file. Apple's guidance is explicit that many small tasks is the wrong shape, and the scheduler caps how many run at once.
- **Progress must keep moving, but not too fast.** A task that looks stalled is force-expired; a task updated dozens of times a second is expired for lock traffic. The core emits `Progress` every 8 MiB per file, which on a fast LAN is well past that, so the shell coalesces to 4/s.
- **Expiration and the user pressing Stop are indistinguishable** through this API — both just call `expirationHandler`. Wooosh treats it as "stop" only when backgrounded, where the transfer could not have continued anyway; on screen it keeps going, having lost nothing but the assertion.
- **Submission requires the foreground**, so a transfer that somehow begins while backgrounded simply runs without an assertion.
- Runtime is not guaranteed or unbounded, and the Simulator does not support it at all. Receiving still works best with Wooosh open, and the UI says so.

macOS and Windows have no equivalent constraint; Android's foreground service is the closest analogue and is strictly stronger.

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

**The receiver publishes the ticket; the sender redeems it.** An earlier draft of this section had it the other way round, which inverts the LAN roles — there the connector is always the sender and originates `OFFER` — and would have forced a second, mirror-image transfer path. Publishing from the receiver makes the internet path the QR pairing flow with a relay in place of a camera, so one engine serves both. Ticket format and the full flow are normative in PROTOCOL.md §9.

UX: one entry point, **Other Device**, which opens a single screen with two segments — **Send** and **Receive**. Segmented picker on Apple, tabs on Android, a `SelectorBar` on Windows.

- **Send**: choose files, then Wooosh publishes a **ticket** (compact string, rendered as a QR and as copyable text: identity key + one-time token + relay hint + expiry) and waits. Redemption starts the transfer.
- **Receive**: scan or paste the ticket the other device is showing. Redeeming it *is* the consent, so no second prompt follows; the incoming transfer appears in the device list like any other.

The ticket travels over any channel the users already share (Messages, email, in person). Works across accounts, networks, and mobile data.

Both halves live behind one entry point rather than behind a question asked first. An earlier build presented a Send / Receive dialog before opening either screen, which made the user commit to a direction while looking at nothing and turned a wrong guess into a full back-and-forth. Two segments show both directions at once and cost a tap to change. The entry point lives in the device list, not in Pair a Device, because "a device that is not on this network" is a destination, which is what a device row already is.

On Apple and Android the entry point is a synthetic row in the device list, pinned **first**, never appended. Appended, every new discovery would push it down, which is precisely the moving-target mis-tap the list rules exist to prevent (§5). Pinned first it is a fixed target and the discovered rows keep their own order below it. It is hidden entirely when the internet path is off. Windows puts it in the command bar instead: its list is bound straight to the registry's append-only collection, and wrapping that collection to inject one non-device row is the sort of indirection the ordering rules exist to keep out.

**Pairing is same-network only.** The internet path never pairs (PROTOCOL.md §9.4): a QR is required for every internet transfer, the sender presents it, the recipient scans and downloads, and the code dies with the transfer. Nothing is written to the trust store.

Two rules the shells follow here:

- **One scanner, both code types.** `wooosh-pair:` and `wooosh-net:` payloads look identical to a camera, so the Scan tab takes either and dispatches on the scheme. Asking the user to pre-classify a code somebody else generated is a question they cannot answer.
- **Nothing is published until the user presses.** Getting a code is the only action in Wooosh that contacts a server of any kind, so the tab explains what will happen and waits. The ticket is invalidated when the user leaves the screen, not when it expires.

A peer reached this way has no mDNS record behind it, so it enters the device list from `PeerConnected` alone, as a connection-only row (append-only like every other row, DESIGN.md §5). Sending to such a row skips `connect_peer`: the connection is already up and is identified by DeviceID.

Because the ticket carries the publisher's identity key out of band, the internet path inherits the QR ceremony's MITM resistance; SAS (PROTOCOL.md §4.3) is also available over iroh, deriving from the same TLS exporter, for anyone who wants to compare digits.

The iroh endpoint is bound **lazily**, on the first ticket operation. A user who only ever shares on a LAN never contacts a relay.

**A relay may carry files, but only small ones.** `run_send` waits up to 15 s for hole punching to produce a direct path. If one appears there is no limit. If none does, the connection is relayed and a per-file cap of **100 MiB** applies; an oversized file fails with `RelayFileTooLarge` before the OFFER is sent, so the receiver is never asked to accept a transfer that cannot run. The receiver enforces the same rule on arrival, because the relay being spent is usually its own.

The reasoning is bandwidth, not secrecy. A relay cannot read anything — the TLS session is end to end and the identity key pins it whichever path is taken. But n0's public relays are free, rate-limited and shared with every other iroh application, and a self-hosted relay is still a server someone pays for. Pushing a 4 GB archive through either is not a good-faith use of it. 100 MiB keeps photos, documents and clips working from anywhere while keeping large transfers on the direct path they should have been on.

The cap is **per file, not per transfer** (PROTOCOL.md §9.1.1), so it bounds what any one stream costs a relay rather than what a session does. Bounding the total was considered and rejected: it would make a transfer's admissibility depend on how the user happened to batch it.

**Relay selection** is `Config.relay_urls`, changeable at runtime via `set_relay_urls` and surfaced in Settings on both shells:

| Value | Meaning |
|---|---|
| `null` | n0's free public relays (default). |
| `[]` | No relay and no address lookup. Surfaced in both shells as **Off**: the UI additionally refuses to publish or redeem tickets, so the internet path is switched off rather than merely relay-free. The core value stays `[]` so that a code path which got past the UI still cannot reach a relay. |
| a list | A chosen or self-hosted relay (`iroh-relay` is open source). |

A device's tickets advertise **its own** home relay, so a self-hosted relay needs configuring on one device only: the redeemer reads the URL out of the ticket and uses it with no setting of their own. Because the ticket publisher is the receiver, the relay belongs to whoever receives. Changing the setting rebinds the iroh endpoint rather than restarting the core, and invalidates any outstanding ticket, which names a relay the device no longer uses.

Relays on but none reachable is an error, not a silent fallback: publishing a ticket that quietly carries only local addresses would hand the user a code that looks fine and can only ever work on their own network.

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
- Incoming offers from a **paired** sender are accepted with no prompt. Pairing already is the consent (PROTOCOL.md §4); re-asking on every transfer trains the user to dismiss the sheet unread, which costs exactly the case the sheet exists for — the unpaired sender, who still gets the full sheet with the fingerprint to verify. The core's `trusted` verdict on the pinned key decides this, never a shell-side guess.
- Visibility modes: **Everyone** (announce + accept offers from unpaired with consent), **Paired only** (announce, but reject unpaired connections at the handshake — with a carve-out for a pending QR/SAS pairing the user just initiated, PROTOCOL.md §4.2, else the mode would block its own pairing flow), **Off** (no announcements, no listener).
- **The default is Paired only**, on every shell. The core deliberately has no default (`Config.visibility` is required), so each shell sets it. A fresh install on a shared network — an office, a dorm, a café — should not be reachable by strangers before the user has opted in, and the QR carve-out means the safe default still cannot block its own pairing flow. `wooosh-cli` keeps `--visibility everyone` as its default because it is a test harness driven by an explicit flag, not a shipped product surface.
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
- iOS background story if Apple ever grants a proper background-transfer entitlement, or once `BGContinuedProcessingTask` can tell a user Stop apart from a system expiry.
- Directory/folder transfers with structure preservation (manifest already carries `relPath` — UI work only).
- Trusted-device sync across a user's own devices (currently pair each pair of devices; could gossip trust between already-paired devices).
- Protocol versioning is in place from day one (`v` in discovery TXT + HELLO); v2 candidates: compression negotiation for compressible types, delta transfer for re-sends.
