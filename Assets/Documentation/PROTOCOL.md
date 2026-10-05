# Wooosh Protocol Specification (v1)

Wire-level companion to [DESIGN.md](DESIGN.md). Normative for `wooosh-core`.

- Transport: QUIC (RFC 9000) + TLS 1.3, single UDP socket.
- Serialization: CBOR (RFC 8949) for all control messages between devices, length-prefixed (`u32` BE) on streams. The rendezvous room (§9.4) uses JSON.
- Hash: BLAKE3 (256-bit). Signatures: Ed25519. KEX: via TLS 1.3 (X25519).
- All multi-byte integers in framing are big-endian; sizes are `u64`.

---

## 1. Ports & service registration

| Purpose | Value |
|---|---|
| mDNS service type | `_wooosh._tcp.local.` (SRV port = QUIC UDP port; type stays `_tcp` per DNS-SD convention) |
| QUIC listener | UDP, ephemeral by default, announced in SRV + TXT `p=` |
| UDP discovery fallback | broadcast + multicast on UDP **44777** |

## 2. Identity

- Each install generates one **Ed25519 identity keypair** on first launch. Private key is stored via the platform adapter: Keychain (iOS/macOS, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`), Android Keystore-wrapped (key material encrypted at rest by a Keystore AES key), DPAPI (Windows).
- **DeviceID** = `BLAKE3(pubkey)[0..16]`, rendered to users as 4 groups of base32 (`Q7KM-3PXA-...`) and as a 6-word fingerprint phrase for verification screens.
- The identity key signs the TLS certificate (below) on every path, LAN or internet (§9), so a peer paired locally is automatically authenticated over the internet path too. It is never sent to the rendezvous service.

## 3. Discovery

### 3.1 mDNS / DNS-SD (primary)
Instance: `<DisplayName> (<4-char suffix of rotating id>)._wooosh._tcp.local.`

TXT record (short keys, all values UTF-8):

| Key | Meaning |
|---|---|
| `v` | protocol version, `1` |
| `rid` | **rotating discovery ID**: 8 random bytes, regenerated per network join / every 24 h. NOT derived from the identity key |
| `dn` | display name |
| `dt` | device type — see the enum below |
| `p` | QUIC UDP port |
| `vis` | `e` (everyone) \| `p` (paired-only) |

**Device type enum (`dt`).** Platform-explicit rather than form-factor-only, because the receiving UI cannot pick a correct icon from form factor alone — an Android phone and an iPhone are both "phone", and showing an iPhone glyph for a Pixel is wrong:

| Wire value | Device |
|---|---|
| `iphone` | iPhone |
| `ipad` | iPad |
| `mac` | Mac (laptop or desktop) |
| `windows` | Windows PC |
| `android-phone` | Android phone |
| `android-tablet` | Android tablet |

Unrecognized or absent values MUST be treated as unknown and rendered with a neutral device glyph — never guessed at. A generic icon is always acceptable; a confidently wrong one is not. Implementations MUST tolerate future additions to this list.

The long-term public key is deliberately **absent** — identity is proven in the handshake, not the broadcast, and `rid` prevents passive cross-network tracking. `dt` does disclose the platform to passive listeners on the LAN, a deliberate and minor trade for correct iconography; it is coarse, carries no identifier, and is already inferable from traffic. TXT data is untrusted UI hint material only.

### 3.2 UDP fallback (multicast-hostile networks)
Every **2 s** while the app is frontmost (30 s otherwise), and immediately on join: broadcast CBOR datagram on UDP 44777 with the same fields as TXT + `goodbye: true` variant on clean exit. Listeners treat mDNS and fallback announces identically and de-duplicate by `rid`.

### 3.3 Peer registry rules (drives the UI contract)
- First sighting of a `rid` → `PeerAppeared` with `discovered_at = now` (monotonic clock). Ordering key forever (per session).
- Announce/scan expected every ≤ **2 s** foreground; peer marked stale after **10 s** silence → `PeerStale`. Any announce → `PeerReturned`. The stale threshold stays at 10 s deliberately: it is now ~5 missed announces rather than 2, so a single dropped multicast packet cannot grey out a device that is still present. Faster scanning is for finding devices sooner, not for dropping them sooner.
- A re-appearing peer with a *new* `rid` but that completes a handshake with a *known pinned key* is merged onto the existing row (paired devices keep their slot across `rid` rotation).

### 3.4 Bluetooth LE (nearby, any network or none)

> **Status: specified, not implemented.**

BLE finds devices that are physically close whether or not they share a network, and introduces them. **It never carries file data**: BLE throughput is orders of magnitude below the 4 GB requirement. After an introduction the devices connect over the LAN (§4.1) or a rendezvous room (§9.4), and everything from TLS onward is unchanged.

**Advertisement.** Service UUID `WOOOSH_BLE_SERVICE` (128-bit, fixed per protocol version) and nothing else. iOS cannot put service data or a custom payload in an advertisement, and a local name would broadcast the device name to everyone in radio range, so neither is used. Advertising runs while Wooosh is in the foreground with visibility not `Off`. iOS moves background advertisements into an overflow area that only iOS scanners filtering for the UUID can see, so background BLE discovery is not relied on anywhere.

**GATT.** One service, one read-only characteristic `INFO`, CBOR:

```
INFO { v: 1, rid: bytes8, vis: "e"|"p",
       dn?: tstr, dt?: tstr,                 // omitted when vis = "p"
       lan?: ["ip:port", ...],               // the §3.1 listener, routable addresses only
       room: bytes16, rk: bytes32 }          // a fresh `peer` room (§9.4) and its candidate-MAC key
```

`room` and `rk` are regenerated with `rid` and whenever an introduction completes. Reading `INFO` requires a BLE connection, so a scanner reads it once per `rid` and caches it.

**Introduction.** Tapping a BLE-discovered row:

1. If any `lan` hint is reachable, dial the hints concurrently as in §4.2 (race, not queue). This is the fast path when both devices are on one network but mDNS is blocked.
2. Otherwise, both sides join `room` as `peer` and hole punch (§9.5) with `k_room = rk`. The advertiser joins when its INFO is read and leaves after 120 s if nobody arrives.
3. TLS and HELLO as §4.1. A pinned key is trusted as usual. An unknown key is `untrusted`, and **pairing is the §4.3 SAS comparison**, which both screens display. No BLE-specific pairing ceremony exists, and BLE link-layer pairing (Just Works or passkey) is not used: neither binds to the Wooosh identity keys.

**Privacy.** Under `PairedOnly`, `INFO` omits `dn` and `dt`, and the scanner shows no row until a handshake proves a pinned key. A stranger in range learns only that a Wooosh device is nearby and its current `rid`, which rotates. Under `Everyone`, `INFO` discloses the name and type to anyone in radio range, the same disclosure mDNS makes to anyone on the LAN, and the visibility setting says so.

**Registry.** A BLE sighting feeds the same registry as mDNS (§3.3): rows keyed by `rid`, ordered by first sighting from either source, stale after 10 s without a sighting from **either** source. A device seen by both is one row.

## 4. Connection & the key-sharing (pairing) protocol

### 4.1 Channel establishment
- QUIC with TLS 1.3, **mutual certificates required**.
- Each side presents a self-signed X.509 cert whose SubjectPublicKeyInfo is its Ed25519 identity key (raw-public-key semantics; the cert is an envelope, CA chains are never evaluated).
- Verification callback = pinning logic:
  - **Known peer** (pubkey in trust store): accept, mark connection `trusted`.
  - **Unknown peer**: accept crypto-wise, mark `untrusted` — the channel is encrypted but unauthenticated pending §4.2/§4.3, and only `HELLO`, pairing messages, and (in visibility=Everyone) `OFFER` are honored. Visibility=PairedOnly → close with `PAIRING_REQUIRED` immediately after **receiving the peer's** HELLO and *without sending our own* (the close code is the entire answer, and withholding HELLO avoids disclosing our display name to a peer we are refusing).
- HELLO on the bidirectional control stream is **role-ordered**, not simultaneous: the client writes its HELLO first; the server reads and validates it (version, visibility) before writing its own. Simultaneous exchange makes rejection non-deterministic — see the close-code rule below.
  `HELLO { v: 1, device_id, dn, dt, caps: [..] }` — version negotiation: lowest common `v`; no common version → close `VERSION_MISMATCH`.

#### 4.1.1 HELLO identity binding (normative)
`HELLO.device_id` is a self-asserted label; the *authenticated* identity is the Ed25519 key in the peer's certificate. A receiver MUST therefore check `device_id == BLAKE3(peer cert key)[0..16]` and reject a mismatch — a peer announcing an identity its key does not back is either broken or impersonating. If the claimed DeviceID is one the receiver has **pinned**, the mismatch is precisely the §4.5 `KEY_CHANGED` signal and MUST be surfaced as such; otherwise it is a plain protocol error. Both cases close with `KEY_CHANGED`. This adds no wire fields (`dt`/`device_id` already ride in HELLO) — only a validation rule that was previously unstated.

#### 4.1.2 Close codes are the rejection contract (normative)
A QUIC `close()` **discards buffered stream data**. A control frame written immediately before closing is therefore usually destroyed in the send buffer and never arrives. Consequences, binding on all implementations:
- A rejecting side MUST NOT rely on a frame it sends just before closing (`PAIR_REJECT`, `UNTRUSTED_MSG` notices, etc.). The **application close code is the only reliable rejection signal**; any such frame is a courtesy, not a guarantee.
- A connecting side MUST consult the application close code and map it to a typed error the UI can act on — `PAIRING_REQUIRED`, `VERSION_MISMATCH`, `QR_KEY_MISMATCH`, `KEY_CHANGED`, `TOKEN_INVALID`. Reporting these as a generic transport failure is a conformance bug: "this device only accepts paired senders" and "the network dropped" demand different user responses.
- Waits for a pairing reply MUST race the reply against connection-closed, so a rejected pairing surfaces immediately rather than burning the full reply timeout.

### 4.2 Pairing method A — QR (out-of-band key exchange; preferred)
1. Receiver R shows QR: `wooosh-pair:1?pk=<R pubkey b64>&tok=<32B single-use token>&dn=<display name>&hints=<ip:port list>&exp=<unix+120s>` (`dn` is an unauthenticated hint so the scanner can label the peer before HELLO). Fields after the first are separated by `&`, standard query syntax — earlier drafts of this document used `?` throughout, which the implementation never emitted; parsers SHOULD accept both for safety.
2. Sender S scans → S now holds R's key **out-of-band** (MITM-proof), connects, verifies the presented cert key equals the QR key (else abort `QR_KEY_MISMATCH`).

**Address hints are a race, not a queue (normative).** `hints` is ordered best-first, but ordering is only a preference: a QR is normally scanned by a *different* device, and R cannot know which of its addresses S can actually route to. Therefore:
- R MUST list only addresses its listening socket can actually be reached on, routable ones first. Loopback is last and exists for the same-machine case only (a desktop app pairing with a CLI on one host); R MUST NOT emit a loopback hint when it is bound to a single non-loopback interface, nor a LAN hint when it is bound to loopback.
- S MUST dial the hints **concurrently** and take the first connection that completes, closing the losers. Dialling them in sequence charges a full connect timeout for every dead hint ahead of the live one, which is unbounded in the number of hints and presents to the user as a frozen screen with no event at all. S SHOULD use a connect deadline tighter than its general-purpose one here, since pairing is an interactive, same-room operation.
- Only the connect is raced. HELLO and `PAIR_REQUEST` run on the single winner, so the single-use token is never presented twice.
- Every terminal outcome of a scan — paired, rejected, expired/unparseable QR, nothing reachable — MUST be reported to the host layer as a pairing result, not merely returned to the caller. Hosts drive the pairing UI off these notifications, so a path that reports nothing is indistinguishable from a hang.
3. S sends `PAIR_REQUEST { token }` on the control stream. Token proves S actually scanned (proximity/authorization); single-use, 120 s expiry, constant-time compare.
4. R replies `PAIR_ACCEPT`; both persist `{peer pubkey, device_id, dn, paired_at}` in the trust store. S's key was delivered to R inside the TLS handshake, whose integrity S just verified against the QR — both directions are now authenticated.

**PairedOnly carve-out (normative).** While R has a QR token pending (120 s window, §4.2 step 1), R MUST accept untrusted connections even under visibility=PairedOnly, and MUST restrict them to `HELLO` + `PAIR_REQUEST` only. Without this, PairedOnly and QR pairing are mutually exclusive — R refuses the very scanner it is displaying a QR for, and can never be paired without first dropping to Everyone. The carve-out is safe because displaying the QR is an explicit, time-boxed user authorization, and a connection that fails to present the valid token is closed with `TOKEN_INVALID` exactly as in Everyone mode. The same applies to a pending SAS exchange the user initiated (§4.3).

### 4.3 Pairing method B — SAS numeric comparison (no camera path)
1. Either side sends `PAIR_REQUEST {}` (no token) over an `untrusted` channel.
2. Both compute `SAS = BE_u32(HKDF(exporter, "wooosh-sas-v1")[0..4]) mod 1_000_000`, where `exporter` = TLS 1.3 exporter secret (RFC 8446 §7.5, label `EXPORTER-wooosh-sas`, 32 bytes). The exporter binds both certificates and the full handshake transcript: an active MITM terminates two distinct TLS sessions and *cannot* force the two displayed codes to match (2⁻²⁰ guess).
3. Both UIs show the 6-digit code; each user confirms match → `PAIR_CONFIRM` both ways → trust store as above. Any mismatch/timeout (60 s) → abort, key NOT stored.

### 4.4 Unpaired transfers ("Everyone" mode)
`OFFER` from an `untrusted` channel is allowed but the consent sheet displays the sender's fingerprint phrase and offers *Accept once* / *Pair & accept*. *Accept once* never writes the trust store.

### 4.5 Trust lifecycle
- Trust store: `{pubkey → device_id, dn, dt?, paired_at, last_seen, last_addr?}`; user-visible list with revoke (removes pin; next contact is `untrusted` again). `dt` is the device type from the peer's HELLO at pairing time (icon hint). `last_addr` is defined below.
- A pinned peer presenting a different key = hard failure `KEY_CHANGED`, surfaced prominently (this is the MITM/reinstall signal); require re-pairing, never silent re-pin (no TOFU downgrade).
- **The trust store is the source of the pin, not the caller (normative).** An implementation MUST NOT make pinning conditional on a host/UI layer passing the expected key into its connect call: the SAS and QR-*displaying* pairing paths never hand the host a key, so a host-supplied pin is exactly the case that silently goes missing. On an outgoing connection with no caller-supplied key the implementation MUST consult its own trust store and, whenever it can resolve the identity behind the target, pin to that key and fail `KEY_CHANGED` on mismatch. Two resolution points, both required:
  1. **Before the handshake** — `last_addr`, the exact `ip:port` at which a pinned peer was last successfully authenticated (recorded on every authenticated connection, in either direction, and unique across entries). A pinned peer reached again at that address is pinned to its stored key, so an imposter that takes over the address is rejected during TLS, before any HELLO or file metadata is exchanged. The mapping is advisory in one direction only: it can add a pin, never grant trust.
  2. **At the handshake** — the certificate key itself, plus the §4.1.1 identity-binding check, which catches a peer that claims a pinned DeviceID while presenting another key. This is the fallback when the peer moved to an address the store has not seen (fresh install of the host, DHCP change, new ephemeral port).
  A caller-supplied key always wins over the store, and supplying it is still preferred — it pins the *first* reconnect, before any address association exists.
- Peer identity is exposed to hosts as the raw public key wherever a peer is named (events, trust list); DeviceID is one-way and cannot be turned back into a pin (DESIGN.md §4).
- Replay/downgrade: TLS 1.3 transcript binding covers the handshake; pairing tokens single-use; `v` echoed inside encrypted HELLO prevents version-downgrade via spoofed TXT.

## 5. Control protocol (bidirectional stream 0)

Messages (CBOR maps, `t` = type tag):

```
OFFER    { t:1, tid: bytes16, files: [FileMeta], total: u64, note?: tstr }
FileMeta { fid: u32, name: tstr, rel_path?: tstr, size: u64,
           mime: tstr, b3: bytes32, mtime: u64 }
DECISION { t:2, tid, accept: [fid...] }            // empty = decline all
RESUME_Q { t:3, tid }                              // sender asks after reconnect
RESUME_A { t:4, tid, have: [{fid, verified_off: u64}] }
DONE     { t:5, tid, fid, ok: bool, err?: tstr }   // receiver, per file
CANCEL   { t:6, tid, fid?: u32 }                   // either side; omit fid = whole transfer
PAIR_*   { t:16..19, ... }                         // §4
BYE      { t:255 }
```

Rules:
- `tid` is generated by the sender and **stable across resumes** — it keys the receiver's resume ledger.
- `name`/`rel_path` are sanitized by the receiver: strip path separators from `name`, reject `..`, control chars, reserved Windows names; `rel_path` only honored under a single transfer root in staging.
- Sender must not open file streams before `DECISION`; receiver resets any early stream.

## 6. File streams

One **unidirectional QUIC stream** per file *slot*, pool of ≤ 4 slots. Stream layout:

```
[ u32 header_len ][ CBOR { tid, fid, off: u64 } ][ raw bytes … ][ QUIC FIN ]
```

- Big file → its own stream, `off` from `RESUME_A` (0 initially). FIN marks end; receiver checks byte count == `size - off` and incremental BLAKE3 (restored from the ledger's hasher checkpoint, or recomputed over the `.part` prefix on cold resume) == `b3`.
- Small files (< 1 MiB) → **pipelined**: a slot stream carries file after file back-to-back (header, bytes, header, bytes…), FIN only when the slot's queue drains. No per-file round trips; `DONE` messages flow back asynchronously on the control stream.
- Receiver writes to `staging/<tid>/<fid>.part`, preallocated to `size`. Ledger `{tid, fid → verified_off, hasher_state}` is updated every 16 MiB of a large file, **once per completed file** (which dominates in pipelined small-file batches), and on shutdown.
- **Ledger concurrency (normative).** Up to 4 slot streams run concurrently, so the ledger has multiple would-be writers. It MUST be held as a single in-memory authority mutated under one lock and rewritten from that memory — never re-read from disk per update, which loses concurrent updates. Each write is atomic (write temp + rename) and the temp file MUST have a **per-write unique name**; a fixed temp path lets one task truncate a file another is mid-write on, and lets two tasks race the same rename. Both corruptions were observed in practice.
- **The ledger is a resume optimization, never the source of truth.** Authoritative completion is the `.part` byte count plus the BLAKE3 verification. A ledger read/write failure MUST therefore be logged and swallowed, never propagated into the transfer path: aborting a slot stream on a ledger error strands every file still queued behind it on that slot, and the transfer can never complete. Worst case, a lost ledger update costs re-hashing a `.part` prefix on the next resume.
- Verified file → shell `FileReady` → routed (DESIGN.md §6) → staging entry removed. Staging older than 7 days is garbage-collected.
- Backpressure is pure QUIC flow control; the app layer never ACKs bytes.

### Transport tuning (initial values, benchmark before changing)
`initial_max_stream_data` 8 MiB · `initial_max_data` 32 MiB · max concurrent uni streams 8 · UDP GSO/GRO enabled where available · MTU discovery on · keep-alive 15 s during active transfer.

## 7. Threat model

| Threat | Mitigation |
|---|---|
| Passive sniffing on open Wi-Fi | Everything after the QUIC Initial is TLS 1.3-encrypted; broadcasts carry no file data and no long-term identifiers |
| Active MITM at first contact | QR = out-of-band key delivery; SAS = exporter-bound code comparison (§4.3) |
| MITM against paired devices | Key pinning, resolved from the implementation's own trust store rather than from a host argument (§4.5); `KEY_CHANGED` hard-fails, no silent re-pin |
| Imposter taking over a paired device's address | `last_addr` pin resolution (§4.5) rejects it during TLS; HELLO identity binding (§4.1.1) rejects a peer that merely claims the pinned DeviceID |
| Device tracking across networks | Rotating `rid`; identity key never broadcast |
| Spoofed discovery (evil twin rows) | TXT is untrusted hints; a spoofer can't complete a handshake as a pinned peer. Unpaired spoofing (in Everyone mode) is fingerprint-verifiable + consent-gated |
| Malicious sender: path traversal | Filename/relpath sanitization (§5); staging jail; routing only after verify |
| Malicious sender: disk-fill / zip-bomb-alike | Consent sheet shows total size before any bytes; preallocation fails fast; per-transfer staging quota = min(free space − 1 GiB, offered total) |
| Offer spam (party mode abuse) | Rate limit unpaired OFFERs per rid+source (3/min), auto-mute option; PairedOnly mode rejects at handshake |
| Pairing token brute force | 32-byte token, single use, 120 s TTL, constant-time compare, connection closed on first failure |
| Malicious or compromised rendezvous (§9) | It never sees identity keys or ticket tokens, candidates are MAC'd with a key it lacks, and relayed bytes are QUIC ciphertext; it can drop traffic but not read, inject, redeem, or impersonate (§9.10) |
| Rendezvous quota abuse | Admission by device attestation with per-device daily quotas; unverified clients are rate-limited per network and never relayed (§9.2) |
| Strangers in Bluetooth range | BLE advertises only a service UUID; INFO omits name and type under PairedOnly; a BLE introduction is followed by a normal TLS handshake and, to pair, the §4.3 comparison (§3.4) |
| Compromised staging tampering | Hash is verified over the final staged bytes immediately before routing |

Out of scope v1: metadata privacy against the local network operator (they can see *that* two devices transfer and rough volume), post-compromise security of a stolen unlocked device, and malware scanning of received files (surfaced to the user as platform conventions: quarantine attribute on macOS `com.apple.quarantine`, MotW `Zone.Identifier` ADS on Windows — both **are** applied by the shells).

## 8. Version negotiation & extensibility
- `v` in TXT is advisory (pre-filter); real negotiation is in HELLO over the encrypted channel.
- Unknown CBOR map keys MUST be ignored (forward compat); unknown `t` on the control stream → `ERR_UNSUPPORTED {t}` response, connection stays up.
- `caps` in HELLO gates optional features (e.g. future `zstd` compression, `dir` structured folders) — both sides must advertise a capability to use it.

## 9. Internet path (off-LAN, Wooosh Rendezvous)

> **Status: implemented in the core, the Worker and both shells; not yet run across real networks** (STATUS.md). §9.9 (paired rendezvous) and NAT-PMP/PCP mapped candidates are not implemented. iroh, which this replaces, is specified in this document as of commit `5daa4d9`.

Everything above describes packets on a LAN. §9 defines the second path they can take: a direct QUIC session between two networks, hole punched with the help of **Wooosh Rendezvous**, a Cloudflare Worker operated by the project (DESIGN.md §9.1). **Only the path changes.** HELLO, OFFER/DECISION, the file streams, the resume ledger, the close codes and the trust model are the ones defined in §4–§6, byte for byte, and implementations MUST NOT fork them per transport.

### 9.1 Roles

| Party | Does | Never does |
|---|---|---|
| **Rendezvous** (Worker + one Durable Object per room) | Admits attested clients (§9.2); joins two clients in a room; forwards their address candidates; forwards relayed datagrams when no direct path forms (§9.6) | Terminate TLS, see keys or tokens, persist anything, log payloads |
| **STUN** (`stun.cloudflare.com:3478`, UDP) | Tells a client its public `ip:port` as seen from outside its NAT | Anything else. Workers cannot open UDP sockets, so the rendezvous cannot do this itself |
| **Clients** | Gather candidates, punch, run the §4–§6 protocol end to end | Trust the rendezvous with anything that grants access to files |

The rendezvous is a matchmaker. It is never a party to the Wooosh handshake, so a compromised or hostile rendezvous can deny service but cannot read, forge, or redirect a session to anyone other than the pinned key (§9.10).

**Storage rule (normative).** The rendezvous MUST NOT write to Durable Object storage, KV, D1, R2, or any log sink. All state (rooms, quota counters) lives in Durable Object memory and dies with the object. Workers Logs and Logpush are disabled. Cloudflare's own edge request metadata is outside the project's control and is disclosed as such in DESIGN.md §9.5.

### 9.2 Admission (device attestation)

The rendezvous runs on a free plan with a hard daily quota (DESIGN.md §9.4). Admission exists to stop that quota being spent by anything other than real Wooosh installs. **It is abuse control, not a security boundary.** Session security rests entirely on the §2 identity keys, and nothing in this section may be relied on for it.

Admission runs **on the room's own WebSocket** (§9.4), before the client may do anything else there. There are no separate HTTP endpoints: one connection per session costs one request, and verification runs under the Durable Object's CPU limit (30 s per message) rather than the free plan's 10 ms Worker limit.

1. On connect the room sends `{ t: "challenge", nonce }`: 32 random bytes, held in the socket's attachment for this connection only.
2. The client answers `{ t: "admit", … }` with exactly one of: a cached `token`; `apple` evidence; `android` evidence; `tpm` evidence (which adds a second round, below); or `anon: true`. Evidence is bound to `nonce` per platform (table below).
3. On success → `{ t: "admitted", tier, token?, key_credential? }`. `token` is an HMAC-SHA256-sealed JSON blob `{ tier, qk, day, exp }`, valid **24 h**, that the client caches and presents instead of evidence on later joins. Only this Worker ever verifies it, so a MAC suffices. Unverified admissions receive no token.

A refused admission answers `{ t: "error", code }` and closes. `bad_token` and `attestation_failed` make the client clear its cached token and key credential and retry once; if a fresh proof was refused, the retry is anonymous rather than a repeat of the same proof.

`qk` is the **quota key**: `HMAC(daily_secret, hardware_identity)` where `hardware_identity` is whatever stable handle the evidence proved: the App Attest key ID; for Android, a random per-install ID that the client binds into the Play Integrity `requestHash = hex(SHA-256(nonce ‖ install_id))` (Play Integrity deliberately exposes no device identifier); or the TPM EK public key. `daily_secret` rotates at 00:00 UTC and is never stored, so `qk` cannot be linked across days, and the hardware identity itself is never kept. The token carries **no Wooosh identity key**: the rendezvous must not learn who is talking to whom.

| Platform | Evidence | Tier |
|---|---|---|
| iOS / iPadOS | App Attest over `clientDataHash = SHA-256(nonce)`. First run: attestation object (verify the chain to the Apple App Attestation root, the nonce extension, the key ID, the App ID, counter 0, `aaguid` = `appattest`). The Worker returns a **key credential**, an HMAC-sealed `{ kid, pub, appId, exp }` blob the client keeps, so the Worker stays stateless. Later admissions: key credential + assertion over `nonce`. Assertion counters are not checked: the Worker keeps no per-key state, and the single-use nonce already makes each assertion fresh | **Verified app** |
| macOS (Developer ID, outside the Mac App Store) | App Attest, as iOS, with the entitlement set to `production` (outside TestFlight, the App Store, and Enterprise, the entitlement chooses the environment). Requires macOS 27, the Wooosh minimum. **Unverified** whether a Developer ID provisioning profile can carry the App Attest capability; until confirmed, Macs fall to Unverified | **Verified app** |
| Android | Play Integrity standard request, `requestHash = SHA-256(nonce)`. The Worker decodes via Google's server API and requires `PLAY_RECOGNIZED`, the release signing-cert digest, and `MEETS_DEVICE_INTEGRITY` | **Verified app** |
| Windows | TPM 2.0 credential activation (below) | **Verified hardware** |
| `wooosh-cli` on Linux / Windows | TPM 2.0 credential activation, where a TPM is present | **Verified hardware** |
| `wooosh-cli` on macOS | App Attest, **only** when the binary runs from inside the signed `Wooosh.app` bundle (§9.2.1) | **Verified app** |
| Any client whose attestation is unavailable or fails `isSupported` | None | **Unverified** |

**TPM 2.0 credential activation** (Windows and the CLI). RSA endorsement keys only, which is what Windows provisions by default; an ECC-only TPM falls to Unverified. Proves "this attestation key (AK) lives in the same genuine TPM as this endorsement key (EK)". It does *not* prove the binary is Wooosh; there is no Windows equivalent of App Attest, hence the separate tier.

1. Client creates (once, then reuses) a restricted signing AK under the EK and sends `tpm: { ek_cert, ek_chain, ak_public }`. `ek_chain` carries the intermediates, which the client fetches from the certificate's AIA URL if the OS does not have them; the Worker bundles roots only. On firmware TPMs the EK certificate itself is fetched by the OS from the vendor, not read from NV.
2. Worker verifies `ek_cert` chains to a bundled TPM manufacturer root (the RootCA set of Microsoft's published `TrustedTpm.cab`, regenerated by `Worker/scripts/update-roots.sh`), checks the AK attributes (`fixedTPM`, `fixedParent`, `sensitiveDataOrigin`, `restricted`, `sign`, not `decrypt`, name algorithm SHA-256), then runs `TPM2_MakeCredential` in software against the EK with a random secret `s`, and answers `{ t: "tpm-challenge", credential_blob, encrypted_secret }`. `SHA-256(s)` stays in the socket's attachment.
3. Client runs `TPM2_ActivateCredential` and answers `{ t: "tpm-activate", secret }`. A match admits the client. `hardware_identity = SHA-256(EK SubjectPublicKeyInfo)`.

#### 9.2.1 The CLI on macOS

A Mac has no TPM, and Secure Enclave keys carry no attestation a third party can verify, so App Attest is the only hardware-backed option. App Attest binds a key to a **Team ID + bundle ID** and needs the `appattest-environment` entitlement, which is restricted and therefore needs a provisioning profile. A loose command-line binary has neither. The CLI can attest only when it is shipped inside `Wooosh.app` (`Contents/MacOS/wooosh-cli`), signed with the app's identity and profile, and calls `DCAppAttestService` through Objective-C bindings. Apple says App Attest "may not be available for use through all app types", so whether a bundled helper launched from Terminal qualifies is **unverified**. Until a spike shows `isSupported` returning true there, the macOS CLI is Unverified. Development uses the `dev` Worker allowlist either way.

A **hardware ID string** (SMBIOS UUID, `MachineGuid`, MAC address) MUST NOT be used as evidence. It is self-reported and therefore forgeable by anyone who reads this document, and it is a stable cross-app tracking identifier. The TPM EK is just as stable, which is why it is folded into the daily-rotating `qk` and never retained.

**Tiers and quotas** (initial values, tune from the dashboard):

| Tier | Rooms per `qk` per day | Relay (§9.6) | Notes |
|---|---|---|---|
| Verified app | 50 | allowed | |
| Verified hardware | 50 | allowed | |
| Unverified | 5 per source `/24` (IPv4) or `/56` (IPv6) | **not allowed** | Direct paths only. Exists so a device whose attestation is unavailable (no TPM, `isSupported` false, a standalone macOS CLI) is not locked out of the internet path entirely |

Quota counters are in-memory and best effort: an evicted Durable Object forgets them, which costs at most one extra day's allowance. Exceeding a quota is `RENDEZVOUS_QUOTA` to the client, distinct from a network failure. When Cloudflare's own daily limit is exhausted the Worker returns errors until 00:00 UTC; clients surface that as `RENDEZVOUS_UNAVAILABLE`.

The Worker accepts only production App Attest keys (`aaguid` = `appattest`), so a Developer ID build left on the default development entitlement is refused rather than quietly admitted.

Simulators, emulators, debug builds, and sideloaded APKs cannot pass Verified-app checks and fall to Unverified. Development uses a separate Worker environment with a debug allowlist, never a bypass in the production Worker.

### 9.3 Ticket format

A **ticket** is the out-of-band capability that starts an internet transfer with a device that is not paired. One line, chat- and QR-safe:

```
wooosh-net:2?nid=<pubkey b64url>&tok=<32B token b64url>&room=<16B room id b64url>&dn=<display name>&exp=<unix>
```

| Field | Meaning |
|---|---|
| `nid` | publisher's Ed25519 identity key (**required**) |
| `tok` | 32-byte single-use pairing token, same rules as the QR token (§4.2 step 3) (**required**) |
| `room` | 16 random bytes naming the rendezvous room (**required**) |
| `dn` | display-name hint, unauthenticated, UI label only |
| `exp` | expiry, unix seconds (**required**) |

v1 tickets (iroh) are rejected with a message asking the other device to update; the two paths do not interoperate.

Rules carried over from v1, unchanged:

- The publisher is the **sender**; the redeemer is the **receiver**. The connector redeems and the *acceptor* offers.
- TTL **120 s**, redemption **single-use**, token compared in constant time, publisher can withdraw immediately.
- A token issued for one transport MUST NOT be redeemable on the other.
- `nid` carries the publisher's key **out of band**, which is what makes the path MITM-proof.

New in v2:

- **`tok` MUST NOT be sent to the rendezvous**, in any form. Only `room` is. If the token reached the Worker, the Worker (or anyone who compromised it) could join as the redeemer and be handed the files: `nid` stops it impersonating the sender, but nothing would stop it posing as the receiver. Keeping `tok` end to end confines the rendezvous to denial of service.
- **A ticket no longer carries addresses.** v1 put the publisher's LAN and public IPs in the ticket, disclosing them to anyone the ticket was forwarded to. v2 exchanges candidates only inside the room, after both sides are admitted, and only with the one other occupant.
- The rendezvous URL is not in the ticket. Both sides use their configured rendezvous (`Config.rendezvous_url`); two devices configured for different rendezvous services cannot meet, which is reported as a timeout naming the setting.

### 9.4 Rooms

`GET /v1/room/<room id>?role=<pub|redeem|peer|pair>` upgrades to a WebSocket; the room id is 16 bytes, base64url. Admission (§9.2) runs first. Control messages are JSON text frames, `t` = type:

```
peer   {}                                              // rendezvous → both, once both are admitted
cand   { c: [{ kind: "host"|"srflx"|"mapped", a: "ip:port" }], ufrag, mac }  // forwarded untouched
go     {}                                              // rendezvous → both, once both sent cand: start punching
path   { direct: true }                                // client → rendezvous, informational
bye    {}                                              // either side: close the room
error  { code, detail? }                               // rendezvous → client
```

Relay frames (§9.6) are **binary** frames, `0x05` followed by `[u16 len][datagram]` repeated. The rendezvous forwards them without parsing.

`role`: `pub` and `redeem` for tickets, `peer` for Bluetooth introductions (§3.4), `pair` for paired rendezvous (§9.9).

- A room holds **at most two** clients of compatible roles. A third connection, or an incompatible role, is refused with HTTP **409**.
- The publisher joins before its code is shown, so a `redeem` arriving at a room with no `pub` holds a withdrawn or expired code: it is refused at once with HTTP **410** rather than left waiting.
- A ticket or BLE room with one occupant expires after 120 s; a paired room (§9.9) stays open as long as its occupant stays connected, which costs nothing while hibernated.
- Clients keep the socket open for the life of the session, since either side may still need the relay. When either side sends `bye` or disconnects, the rendezvous closes the other with reason `peer_left`.
- Rooms use the Durable Object **WebSocket Hibernation API**, so a room waiting for its second occupant costs no duration.
- `role` lets the Worker refuse two publishers in one room. It is not trusted for anything else.
- The room id is a capability only in the weak sense that it locates a room. Guessing one (2¹²⁸) is infeasible; a client that does land in a room still fails the TLS pin (§9.7) or the token check.

### 9.5 Candidates and hole punching

Each client runs **one UDP socket** for the session. QUIC, STUN, and punch probes share it, demultiplexed by first byte (RFC 9443: `0x00–0x03` STUN, `0x40–0x7F`/`0xC0–0xFF` QUIC; probes use STUN Binding with a Wooosh attribute).

1. **Gather.** `host`: every interface address, with link-local omitted and loopback last (it is how two sessions on one machine meet; a peer elsewhere can never validate it). `srflx`: STUN Binding to the STUN server (`stun.cloudflare.com:3478` by default) from the session socket, over each address family it resolves to. `mapped`: a UPnP-IGD / NAT-PMP / PCP mapping where the router allows one; specified but **not implemented** (n0's standalone `portmapper` crate is the candidate). Gathering is capped at 2 s.
2. **Exchange.** `cand` to the room, with `ufrag` 8 random bytes (hex) and `mac = BLAKE3-keyed(k_room, "wooosh cand\0" ‖ ufrag ‖ ("\n" ‖ kind ‖ "|" ‖ addr)*)[0..16]`, hex. `k_room = BLAKE3-derive-key("wooosh room v2", tok)` for tickets, `rk` from BLE INFO for `peer` rooms. The rendezvous cannot compute `k_room`, so it cannot inject candidates the other client will accept. A bad `mac` is dropped.
3. **Punch.** On `go`, both sides send probes to every candidate of the other, every 100 ms, for the 5 s punch window and then the 15 s upgrade window, and answer probes they receive. A probe is a STUN Binding request whose `0xC057` attribute is `BLAKE3-keyed(k_room, "wooosh probe req" ‖ txid)[0..16]`; the answer is a Binding success carrying `XOR-MAPPED-ADDRESS` and `BLAKE3-keyed(k_room, "wooosh probe resp" ‖ txid)[0..16]`. The first candidate whose probe is answered correctly is the **direct path**.
4. **Connect.** After the punch window the redeemer (for `peer` and `pair` rooms, the side with the lower `ufrag`) dials QUIC over the **path-selecting socket** (below), relayed if no direct path was found.

**Path-selecting socket (normative shape).** quinn is given a custom `AsyncUdpSocket` that presents a single synthetic peer address and routes each datagram over the best live path: direct if one is validated, otherwise relay (§9.6). quinn therefore sees one stable peer for the life of the connection, and a session that starts relayed moves onto a direct path the moment punching succeeds, with no QUIC migration involved. Probing continues for 15 s after connect whenever the current path is relay.

Hole punching does not always succeed. Endpoint-dependent mapping on both sides (common with mobile carrier-grade NAT) defeats it. That is the case §9.6 exists for, and it is an expected outcome to report plainly, not a bug.

### 9.6 Relayed transfers are size-capped

When no direct path forms, datagrams travel inside `RELAY` frames over the room's WebSocket. The Durable Object forwards each frame to the other occupant and does nothing else with it.

**On a relayed connection no single file may exceed `RELAY_MAX_FILE_BYTES` (1 GiB).** A direct path has no limit.

- An implementation MUST wait for a direct path before applying the cap: **15 s** from the send request.
- The sender MUST check **before the OFFER** (§5), on metadata only, so an oversized file fails without a wasted hash pass.
- The receiver MUST enforce it too, declining the whole offer.
- Failure is reported as `RELAY_FILE_TOO_LARGE`, distinct from a generic transfer error.
- The cap is per file, not per transfer.
- Control traffic (HELLO, pairing, OFFER/DECISION) is always relayable.
- Clients admitted at the **Unverified** tier MUST NOT be relayed. The Worker refuses `RELAY` frames from their room, and the client reports `RELAY_UNAVAILABLE` rather than a generic failure.

**Batching (normative).** Every incoming WebSocket message is a billable Durable Object request (at the 20:1 WebSocket rate). One QUIC datagram per frame would cost roughly 45 000 requests per GiB and exhaust the free daily quota in about two transfers. A client MUST therefore coalesce outgoing datagrams into one `RELAY` frame, flushing at **256 KiB or 5 ms**, whichever comes first. A relayed GiB then costs roughly 200–600 billable requests in each direction, depending on throughput (DESIGN.md §9.4).

**No second reliability layer.** The relay is a datagram forwarder from QUIC's point of view, even though WebSocket runs over TCP. QUIC's own congestion control bounds what is in flight, so the Durable Object's queue toward the slower side stays at roughly one congestion window. Clients MUST NOT add acknowledgements or retransmission on top.

The cap exists for three reasons: the daily request quota is shared by every user, a single Durable Object's throughput is far below a direct path, and Cloudflare's terms are written for web traffic, not bulk file hosting. 1 GiB keeps the everyday case working from anywhere while keeping the 4 GB case on the direct path it should have been on.

Both sides always accept relayed datagrams, even after finding a direct path: punching can succeed in one direction only, and the other side may still be relaying.

### 9.7 Connection establishment and trust

1. Sender publishes a ticket (§9.3), is admitted (§9.2), and joins the room as `pub`. Visibility applies unchanged, except that a pending ticket is an invitation `PairedOnly` MUST honour, at HELLO *and* at OFFER (§9.8).
2. Receiver parses the ticket, rejects it if expired, is admitted, and joins as `redeem`.
3. Candidates are exchanged and punched (§9.5), then the receiver dials QUIC with ALPN `wooosh/1`.
4. TLS runs exactly as §4.1: mutual certificates over the identity keys. The receiver MUST assert the authenticated server key equals `nid` and hard-fail `KEY_CHANGED` otherwise.
5. HELLO runs exactly as in §4.1, client-first, including §4.1.1 identity binding.
6. The receiver presents `PAIR_REQUEST { token }`; the sender redeems it single-use. Both mark the connection `ticket_authorized` and emit `TicketRedeemed`. **Neither side pins** (§9.8).
7. Transfers proceed under §5 and §6, the publisher originating the `OFFER`.

Deliberate differences from the LAN path:

- **No `last_addr`.** A rendezvous session may be relayed or NAT-mapped, so there is no stable `ip:port` to record as the §4.5 address pin.
- **Rejections still travel as close codes** (§4.1.2).

### 9.8 The internet path never pairs

Unchanged from v1. A redeemed ticket authorises **exactly one transfer session on one connection**. Implementations MUST NOT write the trust store on this path, in either direction.

- The publisher emits `TicketRedeemed`, never `PairingResult`. The redeemer does the same on `PAIR_ACCEPT`.
- The authorisation dies with the connection and is never persisted.
- **A device MUST NOT send file data to an unpinned internet peer that has not redeemed a ticket.**
- `PairedOnly` MUST honour a pending ticket. With no ticket pending, `PairedOnly` rejects as usual.
- Implementations SHOULD NOT display a fingerprint for a ticket peer: there is nothing to compare it against.

A user who wants a durable pairing off-LAN can run the §4.3 SAS ceremony over a rendezvous session. That is explicitly requested (`request_sas_pairing`) and pins exactly as on the LAN. The derivation is used verbatim: both ends of one QUIC session derive the same digits, and a relay, which forwards opaque ciphertext and terminates nothing, cannot make two sessions agree.

### 9.9 Paired rendezvous

> **Status: not implemented.** Specified here so §9.4's `pair` role is defined; the Worker already accepts it.

Two devices that are already paired can meet over the internet without a ticket, provided both have Wooosh open. Each derives the same room id from the pair alone:

`room = BLAKE3-keyed(key = BLAKE3("wooosh pair room v2" ‖ min(pkA,pkB) ‖ max(pkA,pkB)), "YYYY-MM-DD" UTC)[0..16]`

The room is opaque to the rendezvous, which cannot recover either key from it. It does rotate only daily, so within one UTC day the rendezvous can tell that the same pair met more than once. That linkability is the price of not needing a ticket, and is why this path is opt-in per paired device (DESIGN.md §9.1). A client joins the room of every opted-in paired device while Wooosh is in the foreground, and leaves when it is backgrounded; when the other device joins too, the two are introduced as in §9.5. `k_room` for candidate MACs is derived the same way with a different label. Trust is the pin: no token is involved, and an unpinned key closes `KEY_CHANGED` as on the LAN.

### 9.10 Threat notes specific to the rendezvous

- The rendezvous sees: client IP addresses, room ids, message timing and sizes, admission tier, and (in the room) the address candidates of both clients. It does not see identity keys, tokens, display names, file names, or file data.
- A hostile rendezvous can **drop** or **delay**. It cannot inject candidates (MAC'd with a key it lacks), redeem a ticket (it never sees `tok`), or impersonate a peer (TLS pins `nid`). The worst case is denial of service, never a downgrade.
- Relayed bytes are QUIC ciphertext. The §9.6 cap bounds what a relay ever carries; anything larger only crosses a direct path.
- Attestation is not anonymity-neutral: App Attest, Play Integrity, and TPM evidence identify a device to Apple, Google, or a TPM vendor's certificate chain respectively during admission. The Worker reduces that to a daily-rotating quota key and keeps nothing.
- Publishing a ticket reveals the publisher's public key to whoever receives the ticket, as a pairing QR does. It no longer reveals the publisher's IP addresses (§9.3).
