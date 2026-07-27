package com.tsubuzaki.WoooshGo.core

import android.net.Uri
import com.tsubuzaki.WoooshGo.peers.DeviceType
import kotlinx.coroutines.flow.Flow

typealias TransferId = String

/** `fid` on the wire (PROTOCOL.md §5), `u32` in the core. */
typealias FileId = UInt

enum class CoreVisibility { EVERYONE, PAIRED_ONLY, OFF }

enum class FileKind { PHOTO, VIDEO, DOCUMENT }

enum class TransferDirection { SEND, RECEIVE }

data class CoreConfig(
    val displayName: String,
    val deviceType: DeviceType,
    val visibility: CoreVisibility,
    val stagingDir: String,
    /** App-private; the core's pinned peer keys (PROTOCOL.md §4.5). */
    val trustStorePath: String,
    /** Bind address; null = 0.0.0.0 on an ephemeral UDP port. */
    val listenAddr: String? = null,
)

/** Keyed by DeviceID; [publicKey] is the key proven in the TLS handshake, or null. */
data class PeerRef(
    val id: String,
    val displayName: String,
    /** 6-word phrase, derived by the core (PROTOCOL.md §2). */
    val fingerprint: String,
    val paired: Boolean,
    val publicKey: ByteArray? = null,
    /** The peer's HELLO `dt`; null when it announced a type this build doesn't know. */
    val deviceType: DeviceType? = null,
) {
    // ByteArray needs structural equality, or re-emitting the same peer looks like a change.
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

/** The shell's only trust list. Re-read at launch, after each pairing and after a revoke. */
data class TrustedPeerInfo(
    /** `Q7KM-3PXA-…` — identical to the `peerId` carried by every core event. */
    val deviceId: String,
    /** Raw 32 bytes; what `connectPeer` pins against. */
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

/** Unauthenticated hint material (PROTOCOL.md §4.2); [deviceName] is only ever a label. */
data class PairingCodeInfo(
    val deviceId: String,
    val deviceName: String?,
    val hints: List<String>,
    val expired: Boolean,
)

/** [publicKey] arriving out of band is what makes the internet path MITM-proof. */
data class TicketInfo(
    val deviceId: String,
    val publicKey: ByteArray,
    val deviceName: String?,
    /** Home relay carried by the ticket; null when it has direct addresses only. */
    val relay: String?,
    val expired: Boolean,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is TicketInfo) return false
        return deviceId == other.deviceId &&
            deviceName == other.deviceName &&
            relay == other.relay &&
            expired == other.expired &&
            publicKey.contentEquals(other.publicKey)
    }

    override fun hashCode(): Int {
        var result = deviceId.hashCode()
        result = 31 * result + (deviceName?.hashCode() ?: 0)
        result = 31 * result + (relay?.hashCode() ?: 0)
        result = 31 * result + expired.hashCode()
        result = 31 * result + publicKey.contentHashCode()
        return result
    }
}

/** Core -> shell, the single source of UI truth (DESIGN.md §4). */
sealed interface CoreEvent {
    data class PeerConnected(val peer: PeerRef) : CoreEvent
    data class PeerDisconnected(val peerId: String) : CoreEvent

    data class PairingSas(val peer: PeerRef, val sixDigits: String) : CoreEvent

    /** QR or SAS, success or failure/timeout. [message] is the device name, or the reason. */
    data class PairingResult(
        val peerId: String,
        val peer: PeerRef?,
        val success: Boolean,
        val message: String?,
    ) : CoreEvent

    /** Not a pairing: the authorisation dies with the connection (PROTOCOL.md §9.4). */
    data class TicketRedeemed(val peer: PeerRef) : CoreEvent

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

    /** [durationMs] is the core's measurement; the shell never times transfers. */
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

    /** [peer] is the expected identity (PROTOCOL.md §4.5); the presented one may be null. */
    data class KeyChanged(
        val peer: PeerRef,
        val presentedFingerprint: String?,
    ) : CoreEvent
}

/** The `suspend` members do network and disk work and never run on the main thread. */
interface WoooshCore {

    /** Hot; events fired with no subscriber are dropped. */
    val events: Flow<CoreEvent>

    /** Blocking; call off the main thread. */
    fun start(config: CoreConfig)

    fun stop()

    fun setVisibility(mode: CoreVisibility)

    /** `Q7KM-3PXA-…` DeviceID = BLAKE3(pubkey)[0..16]; null before [start]. */
    fun deviceId(): String?

    fun fingerprintPhrase(): String?

    /** Bound "ip:port" of the QUIC listener to publish in the mDNS TXT `p` field. */
    fun listenAddr(): String?

    /** The core's own derivation — never re-implemented in the shell. */
    fun fingerprintPhraseFor(publicKey: ByteArray): String?

    /** Equals the `peerId` in every event. */
    fun deviceIdFor(publicKey: ByteArray): String?

    /** Re-read after every successful pairing and after a revoke. */
    suspend fun trustedPeers(): List<TrustedPeerInfo>

    /** False when [publicKey] was not pinned. */
    suspend fun revokePeer(publicKey: ByteArray): Boolean

    fun beginPairingQr(): String

    /** Local parse, no network. */
    fun parsePairingCode(payload: String): PairingCodeInfo?

    /** Hints are dialled in turn, so callers must show progress for tens of seconds. */
    fun pairWithQr(payload: String)

    fun requestSasPairing(peerId: String)

    fun confirmSas(peerId: String, accepted: Boolean)

    /** First call binds the iroh endpoint and waits up to ~15 s for a home relay. */
    suspend fun beginInternetTicket(): String

    /** A ticket is a capability, so it is invalidated on leaving, not at the 120 s expiry. */
    fun endInternetTicket()

    /** Budget a minute: hole punching runs before the wait for PAIR_ACCEPT starts. */
    fun redeemTicket(ticket: String)

    /** Local parse, no network. */
    fun parseTicket(ticket: String): TicketInfo?

    /** `null` = n0's public relays, `emptyList()` = no relay or address lookup at all. */
    suspend fun setRelayUrls(urls: List<String>?)

    /** Null [expectedPublicKey] leaves the first reconnect to a new address unpinned. */
    suspend fun connectPeer(addr: String, expectedPublicKey: ByteArray? = null): String

    suspend fun send(peerId: String, uris: List<Uri>): TransferId

    /** Empty [acceptedFileIds] declines the whole offer. */
    fun respondToOffer(transferId: TransferId, acceptedFileIds: List<FileId>)

    fun cancel(transferId: TransferId)
}

class CoreException(message: String, cause: Throwable? = null) : Exception(message, cause)
