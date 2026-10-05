# Wooosh — System Design

AirDrop-style file sharing for Android, iOS, macOS, and Windows. Minimal UI, native on every platform, end-to-end encrypted, no accounts. The only server is a stateless rendezvous Worker on Cloudflare that introduces devices across networks and stores nothing (§9).

Companion document: [PROTOCOL.md](PROTOCOL.md) — wire protocol, discovery format, and the key-sharing/pairing cryptography.

---

## 1. Requirements

### Functional
- Automatic discovery of nearby devices on the same LAN (AirDrop-like, zero configuration), and of devices in Bluetooth range on any network (§9.6).
- Send/receive any file type, 1 file or thousands, 500 kB or 4 GB+.
- Secure pairing (key sharing) so transfers are end-to-end encrypted and devices are mutually authenticated; man-in-the-middle on open Wi-Fi must be detectable.
- Received media is routed automatically:
  - **iOS**: photos/videos → Photos library; documents → Files (app's Documents folder, visible in the Files app).
  - **Android**: everything → `Downloads/` (via MediaStore).
  - **macOS / Windows**: everything → `~/Downloads`.
- Share extension on every platform (system share sheet / Share Target / ACTION_SEND).
- Device list sorted by **discovery timestamp**, never reordered; devices that vanish from discovery are **grayed out in place**, not removed (prevents mis-taps from list shifting).
- Screen stays awake during active transfers on mobile; long transfers must not lock the user out of their device.
- Off-LAN sharing (different networks, mobile data) peer to peer, introduced by a rendezvous service that never stores data and relays only as a capped fallback (§9).
- App accent color follows the system accent where the OS supports it.

### Non-functional
- Throughput: saturate typical Wi-Fi (target ≥ 40 MB/s on Wi-Fi 5, ≥ 100 MB/s on wired/Wi-Fi 6E between desktops).
- Small-file efficiency: 1,000 × 500 kB files should not cost 1,000 round trips.
- Resume after interruption (Wi-Fi drop, app kill) without re-sending completed bytes.
- Integrity: every file verified by hash before it is placed in its final location.
- Privacy: no telemetry, no cloud account, discovery broadcast contains no sensitive data.

### Constraints
- Native UI per platform: Kotlin/Jetpack Compose (Android), SwiftUI (iOS + macOS), WinUI 3 (Windows).
- The only backend is Wooosh Rendezvous, a Cloudflare Worker on the free plan (§9.1). It stores nothing, admits only attested clients, and carries file data only on the capped relay fallback. Everything else stays peer to peer.

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
│   Internet path: rendezvous room + STUN + hole punching     │
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
| Internet P2P (§9) | STUN, punching and path selection written once | Reimplement hole punching 4× — not realistic |
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
connect_peer(addr, expected_pubkey?)     // shell resolved it via native mDNS; also the Tailscale / direct-IP path, §9.7
listen_addr() -> String                  // bound "ip:port" for the shell to publish in its mDNS TXT record
trusted_peers() -> [TrustedPeer]         // the pinned set, read from the trust store — the shell's trust list
revoke_peer(pubkey) -> bool              // un-pin; next contact is untrusted again
fingerprint_phrase_for(pubkey) -> String // 6-word verification phrase; shells never reimplement the wordlist
device_id_for(pubkey) -> String          // rendered DeviceID == the peer_id used in events
begin_internet_ticket() -> String         // sender publishes a rendezvous ticket, §9.1
end_internet_ticket()                     // withdraw it immediately
redeem_ticket(ticket: String) -> peer_id  // sender redeems, connects, then send(), §9.1
parse_internet_ticket(ticket) -> TicketInfo // label the UI before redeeming
set_rendezvous_url(url?)                  // null = default Worker, "" = off, §9.1
set_attestor(attestor)                    // platform proof for admission, §9.2
// Planned, not implemented:
set_paired_rendezvous(pubkey, enabled)    // "Reach Over the Internet" per paired device, §9.1.1
ble_info() -> bytes                       // INFO payload for the shell's GATT server, PROTOCOL.md §3.4
connect_nearby(info: bytes, expected_pubkey?) -> peer_id // LAN race, else rendezvous room

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

`PlatformAdapters` (implemented natively, passed in at `start`): `key_store` (get/put identity key), `keep_awake(bool)`, `staging_dir`, `notify(event)`. The `Attestor` (platform evidence for rendezvous admission over a server nonce, §9.2) is installed after `start` with `set_attestor`, so `start`'s signature did not change; without one the device is admitted as Unverified. `Config` gains `rendezvous_url` and `stun_server`, both optional.

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
| `begin_internet_ticket` | joining the room and admission: a platform attestation call (network calls to Apple or Google) at most about once a day, a cached token otherwise. The first moment Wooosh contacts the rendezvous |
| `redeem_ticket` | admission, waiting for the publisher (≤30 s), ≤2 s candidate gathering, the 5 s punch window, a ≤20 s QUIC handshake and the 20 s pairing-reply timeout |
| `connect_nearby` | the LAN hint race (6 s), then the same as `redeem_ticket` without the token |
| `send` / `resume_transfer` | the core runtime while the transfer is registered (streaming itself is asynchronous) |
| `stop` | ≈2 s runtime shutdown, then joining the event thread |

A host `KeyStore` implementation **may block** — Keychain, Android Keystore and DPAPI all can, arbitrarily long on first unlock or behind a biometric prompt. That is why `start` in particular must be dispatched to a background executor (the iOS shell froze its main thread inside a Keychain call before this was written down). Hop back to the UI thread with the result.

Conversely, `CoreEventListener.on_event` is always delivered on the core's own event thread, never on the caller's, so shells must marshal to the UI thread there.

---

## 5. Device list UI (all platforms)

The core maintains the canonical list; shells render it verbatim.

- **Ordering**: strictly by `discovered_at` of the *first* sighting in this app session. New devices append to the bottom. Order never changes afterward — no re-sorting on rename, RSSI, or re-announce.
- **Staleness**: a peer missing from both mDNS and BLE for > 10 s emits `PeerStale`. UI grays the row, disables hit-testing, keeps its position and height. `PeerReturned` restores it in place. Rows are only cleared when the app is relaunched or the user pulls-to-refresh explicitly.
- **Row content**: device-type icon (phone/tablet/laptop/desktop), device name, state line (Ready · Paired ✓ · Away · Receiving…), plus the route when it is not the LAN: **Nearby** (Bluetooth, §9.6) or **Over the Internet** (§9.1). A paired-checkmark, not a separate section — sections would move rows. The route updates in the state line; it never moves the row.
- **Tap** → file picker (or immediate send when launched from the share extension).
- **Incoming offer** → modal sheet: sender name + verification state, file count/total size, thumbnail strip, Accept / Decline. Unpaired senders additionally show the fingerprint code with a "Pair & remember" option.
- Empty state explains visibility mode, says that nearby devices appear while Wooosh is open on both, and offers Pair a Device.

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

## 9. Off-LAN and nearby sharing

> **Status:** §9.1, §9.1.2, §9.2 (iOS, macOS and Android), §9.3 and §9.5 are implemented and tested locally; nothing has run across real networks yet. §9.1.1 (paired rendezvous), §9.2 for Windows and the CLI (TPM), and §9.6 (Bluetooth) are not implemented. iroh, which this replaces, is described in this document as of commit `5daa4d9`. STATUS.md has the detail.

All of these reuse the exact same pairing trust model and transfer protocol. Only the path packets take changes.

### 9.1 Primary: Wooosh Rendezvous (internet P2P through a Cloudflare Worker)

Wooosh runs one piece of infrastructure: **Wooosh Rendezvous**, a Cloudflare Worker with one Durable Object per "room". It introduces two devices and then gets out of the way. Each device learns its public address from a STUN server, the two swap address candidates through the room, and both hole punch so that QUIC runs **directly** between them. Only when punching fails does the room forward traffic itself, and then only for files up to 1 GiB (§9.1.2). Wire format and every normative rule are in PROTOCOL.md §9.

Why a Worker and not iroh, which this replaces:

| | Wooosh Rendezvous | iroh (previous) |
|---|---|---|
| Who runs the introducer | The project, on Cloudflare's free plan | n0's public relays, shared with every iroh app |
| Abuse control | Device attestation before any room is joined (§9.2) | None available to Wooosh |
| What a ticket discloses | Key, token, room id | Key, token, relay URL, **LAN and public IPs** |
| Paired devices over the internet | Possible without a code (PROTOCOL.md §9.9) | Needed a fresh ticket every time |
| NAT traversal code | Ours: STUN, candidate exchange, punching, path selection (`inet/` in the core) | iroh's, mature and multipath |
| Binary size | Drops most of iroh's dependency tree (`portmapper` and `netwatch` remain, so Apple link flags must be re-checked) | Full iroh stack |

The honest cost of the switch is the NAT traversal row: iroh's traversal is more mature than a first in-house implementation will be. The migration therefore keeps iroh behind a build flag until the new path has been measured across real networks (STATUS.md).

**The Worker cannot do STUN.** Workers have no UDP. STUN comes from `stun.cloudflare.com:3478`, Cloudflare's free public STUN server, which sees only a binding request from an IP and port.

**Cloudflare TURN is not used.** Outside Cloudflare's SFU product it costs $0.05/GB with no free allowance, and the room relay covers the capped fallback at no cost (§9.4).

**The sender publishes the ticket; the receiver redeems it.** The sender has already chosen the files, so it is the side with something to publish, and the code it shows is the thing being offered. This does invert the LAN rule that the connector originates `OFFER`, but the engine carries either direction unchanged.

UX: one entry point, **Other Device**, which opens a single screen with two segments, **Send** and **Receive**. Segmented picker on Apple, tabs on Android, a `SelectorBar` on Windows.

- **Send**: choose files, then Wooosh publishes a **ticket** (rendered as a QR and as copyable text) and waits. Redemption starts the transfer.
- **Receive**: scan or paste the ticket the other device is showing. Redeeming it *is* the consent, so no second prompt follows.

The ticket travels over any channel the users already share (Messages, email, in person). Works across accounts, networks, and mobile data.

Both halves live behind one entry point rather than behind a question asked first. An earlier build presented a Send / Receive dialog before opening either screen, which made the user commit to a direction while looking at nothing. Two segments show both directions at once and cost a tap to change. The entry point lives in the device list, not in Pair a Device, because "a device that is not on this network" is a destination, which is what a device row already is.

On Apple and Android the entry point is a synthetic row in the device list, pinned **first**, never appended. Appended, every new discovery would push it down, which is precisely the moving-target mis-tap the list rules exist to prevent (§5). It is hidden entirely when the internet path is off. Windows puts it in the command bar instead: its list is bound straight to the registry's append-only collection.

**Tickets never pair.** A redeemed ticket authorises one transfer and dies with it (PROTOCOL.md §9.8). Nothing is written to the trust store. Durable off-LAN reachability comes from pairing in person (QR, or Bluetooth §9.6) and then opting the device into paired rendezvous (§9.1.1).

Rules the shells follow here, unchanged from the iroh path:

- **One scanner, both code types.** `wooosh-pair:` and `wooosh-net:` payloads look identical to a camera, so the Scan tab takes either and dispatches on the scheme.
- **Nothing is published until the user presses.** Getting a code contacts the rendezvous, so the tab explains what will happen and waits. The ticket is invalidated when the user leaves the screen, not when it expires.
- **Redeeming hands off to the transfer list.** The moment the connection lands the shell leaves the scanner (Apple dismisses the sheet, Android pops to the main screen) and the incoming transfer appears where every other transfer does.
- **The scanner stays armed across a failed code.** Expired, malformed, and unanswerable codes are the common case. The camera pauses only while a ceremony is in flight, and detection is acknowledged by the scanner itself, with a haptic and a visible state change.

A peer reached this way has no mDNS record behind it, so it enters the device list from `PeerConnected` alone, as a connection-only row (append-only like every other row, §5). Sending to such a row skips `connect_peer`.

**A ticket row is never shown as Paired, and it is spent when the transfer ends.** The row says **Ready · Over the Internet** and carries no checkmark, even when the peer is in the trust store from an earlier pairing: the single-use ticket admitted it, not the pin. Once that transfer settles, the row greys out in place, disabled. The shells hold the ticket origin on the row itself and clear it only when the peer connects again for real.

The rendezvous is contacted **lazily**: on the first ticket operation, a Bluetooth introduction that has no LAN route, or a paired room the user opted into. A user who only ever shares on a LAN never contacts it.

**Errors the user can act on** are distinct, not one generic failure:

| Core error | Meaning | Copy direction |
|---|---|---|
| `RelayFileTooLarge` | No direct path; a file is over 1 GiB | The networks can't connect directly, so files over 1 GB can't be sent this way |
| `RelayUnavailable` | No direct path, and this device is not verified for relaying (§9.2) | These networks can't connect directly |
| `RendezvousQuota` | This device's daily allowance is used up | Try again tomorrow |
| `RendezvousUnavailable` | The service is down or over Cloudflare's daily limit | Try again later |

**Rendezvous selection** is `Config.rendezvous_url`, changeable at runtime via `set_rendezvous_url`, surfaced in Settings on every shell as **Internet Connections**, with the choices Wooosh, My Own, and Off:

| Value | Meaning |
|---|---|
| `null` | The project's Worker (default). |
| `""` | Off. The UI hides Other Device and paired rendezvous, and the core refuses every rendezvous operation, so a code path that got past the UI still cannot reach the service. |
| a URL | A self-deployed copy of `Worker/`. Both devices must use the same one; a ticket does not carry the URL. |

#### 9.1.1 Paired devices over the internet

Each paired device has a per-device toggle, **Reach Over the Internet**, off by default. While Wooosh is in the foreground it holds one hibernated WebSocket per opted-in device, in a room derived from the two keys (PROTOCOL.md §9.9). If the other device is doing the same, they meet, and its row reads **Ready · Over the Internet** and accepts sends like any paired row, with no code. The toggle is per device and off by default because the rendezvous can see that the same pair met more than once in a UTC day.

Both devices must have Wooosh open. Reaching a closed app would mean push notifications, which means the Worker storing push tokens, and that breaks the storage rule (§9.5). Not planned.

#### 9.1.2 Relay fallback

`run_send` waits up to 15 s for a direct path. If one appears there is no limit. If none does, the session is relayed through the room and a **per-file cap of 1 GiB** applies, checked before the OFFER on both sides (PROTOCOL.md §9.6). Unverified clients are never relayed.

Relaying is QUIC datagrams batched into WebSocket frames, forwarded by the Durable Object. It is slower than a direct path and is bounded by the shared daily quota, which is why the cap exists. The batching rule in PROTOCOL.md §9.6 is what makes a relayed gigabyte cost hundreds of quota units rather than tens of thousands.

### 9.2 Device attestation, per platform

Admission is abuse control for a shared free quota, not transfer security (PROTOCOL.md §9.2). The core owns the protocol; the platform evidence comes from the shell through a new `attestor` platform adapter, because App Attest and Play Integrity are platform APIs.

| Platform | Mechanism | Tier | Shell work |
|---|---|---|---|
| iOS | App Attest (`DCAppAttestService`). Key generated once; attested once; assertions afterward | Verified app | `com.apple.developer.devicecheck.appattest-environment` entitlement (`production` in release). Key ID kept in the Keychain |
| macOS | App Attest. The Mac gained it in macOS 27; before that `isSupported` was false on every Mac, which is why the minimum OS was raised to 27 | Verified app (pending the Developer ID check below), else Unverified | Same entitlement, set to `production` explicitly: a Developer ID build does not get production automatically the way an App Store build does |
| Android | Play Integrity standard request | Verified app | Play Console link and a Cloud project, whose number goes in the Gradle property `wooosh.playCloudProjectNumber` (unset: Play Integrity off, Unverified). Google's default quota is 10 000 calls/day, shared between token preparation and server decode, so roughly 5 000 admissions/day; the 24 h admission token keeps that well above daily active internet users for a long time. Quota increases take about a week |
| Windows | TPM 2.0 credential activation (EK certificate chain + `MakeCredential`/`ActivateCredential`) | Verified hardware | AK created through the Platform Crypto Provider; EK certificate read from the OS store (firmware TPMs fetch it from the vendor) |
| `wooosh-cli` Linux / Windows | TPM 2.0, implemented in the core (`tss-esapi` on Linux, TBS on Windows) | Verified hardware | n/a |
| `wooosh-cli` macOS | App Attest, only when run from inside the signed `Wooosh.app` bundle; unverified whether Apple allows it for a bundled helper (PROTOCOL.md §9.2.1). Otherwise Unverified | Verified app, else Unverified | Bundle the CLI as `Contents/MacOS/wooosh-cli` |

**Distribution decides what is available.** iOS ships on the App Store and Android on Google Play, so App Attest and Play Integrity (which requires `PLAY_RECOGNIZED`, i.e. a Play-installed build) work as specified. The desktop apps ship **outside** their stores: macOS as a notarized Developer ID app, Windows as a sideloaded MSIX. TPM attestation does not care how Windows was installed. For the Mac, Apple documents App Attest only for TestFlight, App Store, and Enterprise distribution, and does not say whether a Developer ID provisioning profile can include the capability. That is the first spike of phase 1 (STATUS.md); if it cannot, Macs and the macOS CLI are Unverified.

**No hardware ID strings.** SMBIOS UUIDs, `MachineGuid`, and MAC addresses were considered for Windows and the CLI and rejected: they are self-reported, so anyone can send any value, and they are stable tracking identifiers. A TPM proves the key is in real hardware; a string proves nothing. TPM attestation proves real hardware, not a genuine Wooosh binary, which is why Windows and the CLI sit in their own tier with the same quota.

Windows PCs without a TPM 2.0 that exposes an EK certificate (some older or VM installs) fall to Unverified.

### 9.3 Where the Worker lives

`Worker/` at the repo root, TypeScript, configured in `cloudflare.config.ts` and run and deployed with the `cf` CLI (`cf dev`, `cf deploy`), never Wrangler directly. One Worker script and one Durable Object class (`Room`), which also runs admission, plus an in-memory `Quota` Durable Object per quota key. Secrets (`SERVER_SECRET`, which keys every token and quota key, and the Play Integrity service account) are Worker secrets, uploaded with `cf deploy --secrets-file` from a gitignored file and never in the repo; locally they come from `Worker/.dev.vars`, also gitignored. The Apple App Attestation root and the TPM manufacturer roots are bundled as generated constants (`Worker/scripts/update-roots.sh`).

The core learns the production URL at build time from `WOOOSH_RENDEZVOUS_URL` (see `Core/build-bindings.sh`); a build without it reports that no service is configured until Settings names one.

`cloudflare.config.ts` declares no KV, D1, or R2 bindings, `observability.enabled = false`, and `logpush = false`. `--mode dev` builds a second Worker carrying the debug-build allowlist; the production Worker has no bypass. The test suite derives its Miniflare setup from the same file, so the two cannot drift.

### 9.4 Cost on the Cloudflare free plan

Free-plan limits as of October 2026: Workers 100 000 requests/day at 10 ms CPU each; Durable Objects (SQLite-backed only, used here without writing to storage) 100 000 requests/day and 13 000 GB-s/day; incoming WebSocket messages billed at 20:1. **There is no overage on the free plan**: past a limit, requests fail until 00:00 UTC. The worst case is an outage on a busy day, never a bill.

| Operation | Worker requests | Durable Object requests | Notes |
|---|---|---|---|
| Admission | 2 | 1 (quota) | Once per 24 h per device |
| One ticket transfer, direct | 2 (WebSocket upgrades) | 2 + ~1 (≈10 messages ÷ 20) | Typical case |
| One relayed GiB | 0 | ≈400–1 200 | Both directions, with PROTOCOL.md §9.6 batching. Without batching it would be tens of thousands |
| Paired room held open | 1 per app launch per opted-in device | 1 | Hibernated, so no duration cost while waiting |

Rough ceilings: around 30 000 direct transfers a day, or about 80–250 relayed GiB a day across all users, whichever runs out first. Durable Object duration is not the binding limit: a relayed GiB at ~20 MB/s keeps one object busy for ~55 s, about 7 GB-s. The admission step's CPU (one ECDSA or Ed25519 verification, or one RSA-OAEP and AES pass for TPM) fits inside 10 ms; App Attest first-time attestation, which verifies a certificate chain, is the heaviest call and must be measured.

If usage outgrows this, Workers Paid ($5/month) raises every limit by orders of magnitude. Check current pricing at that point rather than trusting these figures.

### 9.5 Privacy

The rendezvous **stores nothing**: no Durable Object storage writes, no KV/D1/R2, no logs (PROTOCOL.md §9.1).

What it necessarily **sees**, in memory, while a room is open: both clients' IP addresses and address candidates, the room id, message timing and sizes, and each client's admission tier. What it **never sees**: identity keys, ticket tokens, display names, file names, file contents. Relayed bytes are QUIC ciphertext.

What is outside the project's control: Cloudflare's edge keeps standard request metadata (IP, time, URL path) under its own policy; Apple, Google, and TPM vendors learn that a device performed an attestation for Wooosh. Settings and the privacy policy say both plainly.

Compared with the iroh path, tickets no longer carry IP addresses, and the introducer is no longer shared with every other iroh application.

### 9.6 Nearby over Bluetooth

BLE is how Wooosh finds a device in the same room, regardless of network, and how pairing gets easy. It is never a data transport (PROTOCOL.md §3.4).

- **Discovery.** BLE sightings feed the same registry as mDNS: one row per device, first sighting from either source sets its place, stale only when **both** sources go quiet for 10 s. A device on another network but within Bluetooth range appears as **Ready · Nearby**.
- **Sending to a nearby row** connects over the LAN if the devices turn out to share one, otherwise through a rendezvous room. That route is cheap (a handful of quota units) and still direct; it needs internet access on both devices.
- **Pairing without a camera.** **Pair a Device** opens on a list of nearby unpaired devices from BLE and mDNS. Tapping one runs the existing SAS ceremony (PROTOCOL.md §4.3): both screens show the same six digits, and each user presses **Confirm** (a deliberate press, never a default action). QR is still offered below the list for devices that cannot use Bluetooth. This also gives SAS the UI entry point it never had.
- **Visibility.** Under Paired only, a stranger in range sees nothing but an anonymous UUID; under Everyone, the device's name and type are readable over BLE, as they already are over mDNS. The visibility setting's description says so.
- **Foreground only.** iOS hides background BLE advertisements from non-iOS scanners, so BLE runs while Wooosh is open. The empty state says Wooosh finds nearby devices while it is open on both.
- **Permissions**, asked when Pair a Device or the device list first needs them, not at launch: iOS/macOS `NSBluetoothAlwaysUsageDescription` (plus the sandbox Bluetooth entitlement on macOS); Android `BLUETOOTH_SCAN` with `neverForLocation`, `BLUETOOTH_ADVERTISE`, `BLUETOOTH_CONNECT`; Windows the `bluetooth` capability.

### 9.7 Power-user: Tailscale as the network
If both users run Tailscale and share nodes across their tailnets (Tailscale's built-in cross-account node sharing invite, free tier), each device just has a stable WireGuard-encrypted IP. Wooosh needs almost nothing: listen on all interfaces (already true) and offer **Add device manually** (`add_manual_peer(host)`) accepting a Tailscale IP/MagicDNS name. mDNS doesn't cross tailnets, so discovery is manual, but pairing and transfer are unchanged, and Wooosh's E2E crypto still applies on top of WireGuard. Zero code beyond the manual-add field; document it as a recipe.

### 9.8 Zero-infrastructure fallback: wormhole-style codes
Adopt the Magic Wormhole model: a short human-speakable code (`7-guitar-sunset`) performs a PAKE (SPAKE2) through the public wormhole mailbox/relay servers. It overlaps the rendezvous's job with a worse trust ceremony. Hold unless users ask.

### 9.9 Same room, no network at all
Bluetooth (§9.6) finds and pairs devices with no shared network, but moving the files still needs a path:
- **Apple ↔ Apple**: Multipeer Connectivity / AWDL, with Wi-Fi and Bluetooth on and no network needed. Used as an alternate transport under the same protocol.
- **Wi-Fi Aware** (iOS 26 added it; Android has had it since 8, hardware permitting) could give a direct, fast iPhone ↔ Android link with no network at all. Not yet evaluated: it needs an entitlement on iOS and is device-dependent on Android. Spike before committing.
- **Android/Windows ↔ anyone**: the sending device raises a **local-only hotspot** (Android `startLocalOnlyHotspot`, Windows Mobile Hotspot) and shows a QR encoding SSID + password + its listen address; iPhones join via the camera QR and land on a normal LAN transfer. Clunkier, but universal.

**Not chosen**: WebRTC (ICE would solve traversal, but it would replace QUIC and fork the transfer protocol per path); Cloudflare TURN ($0.05/GB with no free allowance, §9.1); Tailscale Funnel (exposes a public HTTPS endpoint, the wrong privacy posture); Bluetooth as a file transport (too slow for the 4 GB requirement; it is used for discovery and introduction only); hardware ID strings as attestation (§9.2).

---

## 10. Security summary (threat model in PROTOCOL.md §7)

- Every connection is TLS 1.3 (QUIC) with **both** peers presenting keys; trust is key pinning, not CAs.
- First contact is authenticated by QR (out-of-band key exchange) or SAS numeric comparison (MITM detection); after that, pinned keys authenticate silently.
- Incoming offers from a **paired** sender are accepted with no prompt. Pairing already is the consent (PROTOCOL.md §4); re-asking on every transfer trains the user to dismiss the sheet unread, which costs exactly the case the sheet exists for — the unpaired sender, who still gets the full sheet with the fingerprint to verify. The core's `trusted` verdict on the pinned key decides this, never a shell-side guess.
- Visibility modes: **Everyone** (announce + accept offers from unpaired with consent), **Paired only** (announce, but reject unpaired connections at the handshake — with a carve-out for a pending QR/SAS pairing the user just initiated, PROTOCOL.md §4.2, else the mode would block its own pairing flow), **Off** (no announcements, no listener).
- **The default is Paired only**, on every shell. The core deliberately has no default (`Config.visibility` is required), so each shell sets it. A fresh install on a shared network — an office, a dorm, a café — should not be reachable by strangers before the user has opted in, and the QR carve-out means the safe default still cannot block its own pairing flow. `wooosh-cli` keeps `--visibility everyone` as its default because it is a test harness driven by an explicit flag, not a shipped product surface.
- Discovery broadcasts contain only: display name, device type, protocol version, port, and a *rotating* discovery ID — the long-term identity key is never broadcast, so passive listeners can't track a device across networks by key.
- Files never touch their final destination until hash-verified; staging is app-private.
- No accounts, no telemetry. The one server, Wooosh Rendezvous, stores nothing and never sees keys, tokens, names, or file contents (§9.5).

---

## 11. Testing strategy

- **Interop matrix in CI**: the Rust core makes this cheap — a headless `wooosh-cli` (same core) runs sender/receiver pairs in CI across Linux/macOS/Windows runners for every protocol change; device-lab runs (Android/iOS) per release.
- **Chaos suite**: simulated packet loss/latency (`netem`), mid-transfer Wi-Fi drop, app kill + resume, clock skew, disk-full on receiver, 10k × 100 kB and 1 × 8 GB fixtures.
- **Fuzzing**: CBOR control-message parser and manifest parser fuzzed (cargo-fuzz) — these are the attack surface reachable pre-consent.
- **Benchmarks**: throughput regression gates on loopback + reference Wi-Fi hardware.
- **Rendezvous**: the Worker has its own suite inside the Workers runtime (`npm test` in `Worker/`): attestation verifiers (including a byte-for-byte TPM MakeCredential check against an independent implementation), and rooms over real WebSockets. The core's integration suite runs against an in-process mock of the room protocol, and an `#[ignore]`d test runs two cores against the real Worker under `cf dev`, direct and forced-relay. NAT traversal is tested with Linux network namespaces behind endpoint-independent and endpoint-dependent NATs, so both the punched and the relayed path run in CI. Neither replaces the one test that matters: a phone on mobile data and a laptop on home broadband.

## 12. What to revisit as it grows
- Flow-control window sizes and stream-pool size per platform (measure, don't guess).
- iOS background story if Apple ever grants a proper background-transfer entitlement, or once `BGContinuedProcessingTask` can tell a user Stop apart from a system expiry.
- Directory/folder transfers with structure preservation (manifest already carries `relPath` — UI work only).
- Trusted-device sync across a user's own devices (currently pair each pair of devices; could gossip trust between already-paired devices).
- Wi-Fi Aware as a no-network transport between iPhone and Android (§9.9).
- Workers Paid if the free plan's daily limits start failing real users (§9.4).
- Protocol versioning is in place from day one (`v` in discovery TXT + HELLO); v2 candidates: compression negotiation for compressible types, delta transfer for re-sends.
