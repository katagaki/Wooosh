# Wooosh Protocol Specification (v1)

Wire-level companion to [DESIGN.md](DESIGN.md). Normative for `wooosh-core`.

- Transport: QUIC (RFC 9000) + TLS 1.3, single UDP socket.
- Serialization: CBOR (RFC 8949) for all control messages, length-prefixed (`u32` BE) on streams.
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
- The identity key signs the TLS certificate (below) and doubles as the iroh node key for the internet path (DESIGN.md §9.1), so a peer paired locally is automatically authenticated over the internet path too.

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
| Malicious relay (internet path, iroh) | Relays forward opaque QUIC ciphertext; peer auth is the same Ed25519 pinning + SAS — a relay can drop traffic but not read or impersonate |
| Compromised staging tampering | Hash is verified over the final staged bytes immediately before routing |

Out of scope v1: metadata privacy against the local network operator (they can see *that* two devices transfer and rough volume), post-compromise security of a stolen unlocked device, and malware scanning of received files (surfaced to the user as platform conventions: quarantine attribute on macOS `com.apple.quarantine`, MotW `Zone.Identifier` ADS on Windows — both **are** applied by the shells).

## 8. Version negotiation & extensibility
- `v` in TXT is advisory (pre-filter); real negotiation is in HELLO over the encrypted channel.
- Unknown CBOR map keys MUST be ignored (forward compat); unknown `t` on the control stream → `ERR_UNSUPPORTED {t}` response, connection stays up.
- `caps` in HELLO gates optional features (e.g. future `zstd` compression, `dir` structured folders) — both sides must advertise a capability to use it.
