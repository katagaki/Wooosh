package com.tsubuzaki.WoooshGo.core

import android.net.Uri
import com.tsubuzaki.WoooshGo.peers.DeviceType
import kotlinx.coroutines.flow.Flow

typealias TransferId = String

/** File id inside a transfer — `fid` on the wire (PROTOCOL.md §5), `u32` in the core. */
typealias FileId = UInt

enum class CoreVisibility { EVERYONE, PAIRED_ONLY, OFF }

enum class FileKind { PHOTO, VIDEO, DOCUMENT }

enum class TransferDirection { SEND, RECEIVE }

data class CoreConfig(
    val displayName: String,
    val deviceType: DeviceType,
    val visibility: CoreVisibility,
    /** App-private directory where the core stages incoming files until FileReady. */
    val stagingDir: String,
    /** App-private JSON file holding the core's pinned peer keys (PROTOCOL.md §4.5). */
    val trustStorePath: String,
    /** Bind address; null = 0.0.0.0 on an ephemeral UDP port. */
    val listenAddr: String? = null,
)

/**
 * A peer as the core sees it — keyed by long-term identity (DeviceID), NOT by the
 * rotating discovery id used by the NSD browser.
 *
 * [publicKey] is the peer's raw 32-byte Ed25519 identity key, proven in the TLS
 * handshake. Non-null for anything that came off the wire; null only for a peer known by
 * DeviceID alone (a cache miss). Pass it back to `connectPeer` / `revokePeer`.
 */
data class PeerRef(
    val id: String,
    val displayName: String,
    /** Human-checkable 6-word phrase (PROTOCOL.md §2), derived by the core from [publicKey]. */
    val fingerprint: String,
    val paired: Boolean,
    val publicKey: ByteArray? = null,
    /** The peer's HELLO `dt`; null when it announced a type this build doesn't know. */
    val deviceType: DeviceType? = null,
) {
    // ByteArray needs structural equality, or every re-emission of the same peer looks
    // like a change to the StateFlows that hold PeerRefs.
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is PeerRef) return false
        return id == other.id &&
            displayName == other.displayName &&
            fingerprint == other.fingerprint &&
            paired == other.paired &&
            deviceType == other.deviceType &&
            publicKey.contentEquals(other.publicKey)
    }

    override fun hashCode(): Int {
        var result = id.hashCode()
        result = 31 * result + displayName.hashCode()
        result = 31 * result + fingerprint.hashCode()
        result = 31 * result + paired.hashCode()
        result = 31 * result + (deviceType?.hashCode() ?: 0)
        result = 31 * result + (publicKey?.contentHashCode() ?: 0)
        return result
    }
}

/**
 * One pinned peer from the core's own trust store (`trustedPeers()`) — the shell's only
 * trust list. Re-read it at launch, after every successful pairing and after a revoke.
 */
data class TrustedPeerInfo(
    /** `Q7KM-3PXA-…` — identical to the `peerId` carried by every core event. */
    val deviceId: String,
    /** Raw 32 bytes; what `connectPeer` pins against and `revokePeer` takes. */
    val publicKey: ByteArray,
    val displayName: String,
    val deviceType: DeviceType?,
    val fingerprint: String,
    /** Unix milliseconds (the core reports seconds). */
    val pairedAtMillis: Long,
    val lastSeenMillis: Long,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is TrustedPeerInfo) return false
        return deviceId == other.deviceId &&
            displayName == other.displayName &&
            deviceType == other.deviceType &&
            fingerprint == other.fingerprint &&
            pairedAtMillis == other.pairedAtMillis &&
            lastSeenMillis == other.lastSeenMillis &&
            publicKey.contentEquals(other.publicKey)
    }

    override fun hashCode(): Int {
        var result = deviceId.hashCode()
        result = 31 * result + displayName.hashCode()
        result = 31 * result + (deviceType?.hashCode() ?: 0)
        result = 31 * result + fingerprint.hashCode()
        result = 31 * result + pairedAtMillis.hashCode()
        result = 31 * result + lastSeenMillis.hashCode()
        result = 31 * result + publicKey.contentHashCode()
        return result
    }
}

data class FileMeta(
    val id: FileId,
    val name: String,
    val size: Long,
    val mime: String,
)

/**
 * A parsed `wooosh-pair:1?…` payload (PROTOCOL.md §4.2), read *without* dialling anything.
 *
 * Everything in here is unauthenticated hint material — [deviceName] is the QR's `dn`
 * field and is only ever a label. It exists so the pairing UI can say who it is
 * connecting to while the blocking handshake runs, and can reject an expired or
 * malformed code instantly instead of after a network timeout.
 */
data class PairingCodeInfo(
    val deviceId: String,
    val deviceName: String?,
    val hints: List<String>,
    val expired: Boolean,
)

/**
 * Event stream (core -> shell), the single source of UI truth (DESIGN.md §4).
 *
 * This mirrors the shape of `uniffi.wooosh_core.CoreEvent` one-for-one; [RealCore]
 * only folds the per-event peer fields (`peer_id` / `peer_pubkey` / `device_name` /
 * `device_type` / `fingerprint` / `trusted`) into a [PeerRef].
 */
sealed interface CoreEvent {
    data class PeerConnected(val peer: PeerRef) : CoreEvent
    data class PeerDisconnected(val peerId: String) : CoreEvent

    data class PairingSas(val peer: PeerRef, val sixDigits: String) : CoreEvent

    /**
     * Pairing concluded — QR or SAS, success or failure/timeout (DESIGN.md §4).
     * On success [message] carries the peer's device name, on failure the reason.
     * [peer] carries the key that was (or would have been) pinned.
     */
    data class PairingResult(
        val peerId: String,
        val peer: PeerRef?,
        val success: Boolean,
        val message: String?,
    ) : CoreEvent

    data class TransferStarted(
        val transferId: TransferId,
        val peer: PeerRef,
        val direction: TransferDirection,
        val manifest: List<FileMeta>,
    ) : CoreEvent

    data class IncomingOffer(
        val transferId: TransferId,
        val from: PeerRef,
        val manifest: List<FileMeta>,
    ) : CoreEvent

    data class Progress(
        val transferId: TransferId,
        val fileId: FileId,
        val bytes: Long,
        val totalBytes: Long,
        /** Bytes/second for this transfer attempt. */
        val rate: Long,
        /** Seconds remaining; -1 when unknown. */
        val etaSeconds: Long,
    ) : CoreEvent

    data class FileReady(
        val transferId: TransferId,
        val fileId: FileId,
        val stagedPath: String,
        val kind: FileKind,
    ) : CoreEvent

    /** [durationMs] is the core's own wall-clock measurement; the shell never times transfers. */
    data class TransferDone(
        val transferId: TransferId,
        val okFiles: Int,
        val failedFiles: Int,
        val bytesTransferred: Long,
        val durationMs: Long,
    ) : CoreEvent

    data class TransferError(
        val transferId: TransferId,
        val message: String,
        val resumable: Boolean,
    ) : CoreEvent

    /**
     * A pinned peer presented a different key (PROTOCOL.md §4.5). [peer] is the pinned
     * identity we expected (its [PeerRef.fingerprint] is the expected phrase);
     * [presentedFingerprint] is the phrase for the key actually offered, when the
     * handshake got far enough to observe one.
     */
    data class KeyChanged(
        val peer: PeerRef,
        val presentedFingerprint: String?,
    ) : CoreEvent
}

/**
 * Shell-side seam over the wooosh-core FFI surface (DESIGN.md §4), implemented by [RealCore].
 *
 * Blocking calls (`connectPeer`, `send`, `revokePeer`, `trustedPeers`) are `suspend`:
 * the core does real network and disk work on them and they must never run on the main
 * thread.
 */
interface WoooshCore {

    /** Core -> shell event stream. Hot; events fired with no subscriber are dropped. */
    val events: Flow<CoreEvent>

    /** Blocking; call off the main thread. Boots the engine and binds the QUIC socket. */
    fun start(config: CoreConfig)

    fun stop()

    fun setVisibility(mode: CoreVisibility)

    // ---- identity: the CORE is the single source of truth (PROTOCOL.md §2) ----

    /** `Q7KM-3PXA-…` DeviceID = BLAKE3(pubkey)[0..16]; null before [start]. */
    fun deviceId(): String?

    /** 6-word verification phrase; null before [start]. */
    fun fingerprintPhrase(): String?

    /** Bound "ip:port" of the QUIC listener to publish in the mDNS TXT `p` field. */
    fun listenAddr(): String?

    /** The core's own derivation for any peer key — never re-implemented in the shell. */
    fun fingerprintPhraseFor(publicKey: ByteArray): String?

    /** The core's own DeviceID derivation; equals the `peerId` in every event. */
    fun deviceIdFor(publicKey: ByteArray): String?

    // ---- trust (PROTOCOL.md §4.5) ----

    /**
     * The core's pinned peer set, read straight from its trust store. This is the
     * shell's trust list — re-read after every successful pairing and after a revoke.
     */
    suspend fun trustedPeers(): List<TrustedPeerInfo>

    /** Drops the core's pin for [publicKey]; false when it was not pinned. */
    suspend fun revokePeer(publicKey: ByteArray): Boolean

    // ---- pairing (PROTOCOL.md §4) ----

    /** Returns the `wooosh-pair:1?...` payload to render as a QR code. */
    fun beginPairingQr(): String

    /**
     * Parses a scanned or pasted payload locally — no network, no blocking. Null when it
     * is not a Wooosh pairing code at all. Lets the UI name the peer while it waits.
     */
    fun parsePairingCode(payload: String): PairingCodeInfo?

    /**
     * Sender-side QR path. Returns immediately; the outcome arrives as
     * [CoreEvent.PairingResult] — success from the core, failure synthesised from the
     * thrown error. It can take tens of seconds (address hints are dialled in turn), so
     * callers must show progress rather than assume it is quick.
     */
    fun pairWithQr(payload: String)

    /** Camera-less path: start SAS numeric comparison with a connected peer. */
    fun requestSasPairing(peerId: String)

    fun confirmSas(peerId: String, accepted: Boolean)

    // ---- connections & transfers ----

    /**
     * Connects to an address the native mDNS browser resolved (DESIGN.md §4).
     * [expectedPublicKey] pins the TLS handshake to that exact key; always pass the
     * pinned key from [trustedPeers] when the shell holds one. Passing null does not
     * opt out of pinning (the core re-applies its own pin when it can resolve the
     * identity behind the address) but it does leave the very first reconnect to a new
     * address unpinned.
     *
     * Returns the core's peer id (= the peer's DeviceID).
     */
    suspend fun connectPeer(addr: String, expectedPublicKey: ByteArray? = null): String

    /** Begins an outgoing transfer; manifest arrives via [CoreEvent.TransferStarted]. */
    suspend fun send(peerId: String, uris: List<Uri>): TransferId

    /** Empty [acceptedFileIds] declines the whole offer. */
    fun respondToOffer(transferId: TransferId, acceptedFileIds: List<FileId>)

    fun cancel(transferId: TransferId)
}

/** Thrown by [WoooshCore.connectPeer] / [WoooshCore.send] with a user-presentable message. */
class CoreException(message: String, cause: Throwable? = null) : Exception(message, cause)
