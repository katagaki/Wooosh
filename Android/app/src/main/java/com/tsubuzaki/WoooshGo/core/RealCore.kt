package com.tsubuzaki.WoooshGo.core

import android.content.Context
import android.net.Uri
import android.os.SystemClock
import android.provider.OpenableColumns
import android.util.Log
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.identity.IdentityManager
import com.tsubuzaki.WoooshGo.peers.DeviceType
import java.io.File
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.wooosh_core.WoooshException
import uniffi.wooosh_core.deviceIdFor as ffiDeviceIdFor
import uniffi.wooosh_core.fingerprintPhraseFor as ffiFingerprintPhraseFor
import uniffi.wooosh_core.parseInternetTicket
import uniffi.wooosh_core.parsePairingQr
import uniffi.wooosh_core.CoreEvent as FfiEvent
import uniffi.wooosh_core.CoreEventListener as FfiListener
import uniffi.wooosh_core.Config as FfiConfig
import uniffi.wooosh_core.DeviceType as FfiDeviceType
import uniffi.wooosh_core.FileKind as FfiFileKind
import uniffi.wooosh_core.TransferDirection as FfiDirection
import uniffi.wooosh_core.TrustedPeer as FfiTrustedPeer
import uniffi.wooosh_core.Visibility as FfiVisibility
import uniffi.wooosh_core.WoooshCore as FfiCore

/**
 * The core delivers events on its own thread, so [FfiListener.onEvent] only does a
 * non-blocking `trySend`; the `suspend` members `block_on` a tokio future and always run
 * on [Dispatchers.IO]. Identity belongs to the core (PROTOCOL.md §2).
 */
class RealCore(
    context: Context,
    private val scope: CoroutineScope,
    private val keyStore: IdentityManager,
) : WoooshCore {

    private val appContext = context.applicationContext
    private val ffi = FfiCore()

    private val _events = MutableSharedFlow<CoreEvent>(replay = 0, extraBufferCapacity = 256)
    override val events: Flow<CoreEvent> = _events.asSharedFlow()

    private val peerCache = ConcurrentHashMap<String, PeerRef>()

    @Volatile
    private var started = false

    override fun start(config: CoreConfig) {
        if (started) return
        File(config.stagingDir).mkdirs()
        File(config.trustStorePath).parentFile?.mkdirs()

        // trySend never blocks, so a slow collector cannot stall the core.
        val ready = CountDownLatch(1)
        var failure: Throwable? = null

        callbackFlow {
            val listener = object : FfiListener {
                override fun onEvent(event: FfiEvent) {
                    val mapped = runCatching { adapt(event) }.getOrElse { error ->
                        Log.e(TAG, "failed to adapt core event $event", error)
                        null
                    } ?: return
                    if (mapped !is CoreEvent.Progress) Log.i(TAG, "event: $mapped")
                    if (trySend(mapped).isFailure) {
                        Log.w(TAG, "dropped core event (buffer full): $mapped")
                    }
                }
            }
            try {
                ffi.start(
                    FfiConfig(
                        deviceName = config.displayName,
                        deviceType = config.deviceType.toFfi(),
                        visibility = config.visibility.toFfi(),
                        stagingDir = config.stagingDir,
                        trustStorePath = config.trustStorePath,
                        listenAddr = config.listenAddr,
                    ),
                    keyStore,
                    listener,
                )
                started = true
                Log.i(
                    TAG,
                    "core started: deviceId=${ffi.deviceId()} listenAddr=${ffi.listenAddr()} " +
                        "fingerprint=\"${ffi.fingerprintPhrase()}\" staging=${config.stagingDir}",
                )
            } catch (t: Throwable) {
                Log.e(TAG, "core start failed", t)
                failure = t
            } finally {
                ready.countDown()
            }
            if (failure != null) {
                close()
                return@callbackFlow
            }
            awaitClose { runCatching { ffi.stop() } }
        }
            .buffer(EVENT_BUFFER)
            .onEach { _events.emit(it) }
            .launchIn(scope)

        // Must block: the shell needs listen_addr() and the identity the moment it returns.
        if (!ready.await(START_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            throw CoreException("Timed out starting the Wooosh core")
        }
        failure?.let { throw CoreException(userMessage(it), it) }
    }

    override fun stop() {
        if (!started) return
        started = false
        runCatching { ffi.stop() }.onFailure { Log.w(TAG, "core stop failed", it) }
    }

    override fun setVisibility(mode: CoreVisibility) {
        if (!started) return
        runCatching { ffi.setVisibility(mode.toFfi()) }
            .onFailure { Log.w(TAG, "setVisibility failed", it) }
    }

    override fun deviceId(): String? =
        if (!started) null else runCatching { ffi.deviceId() }.getOrNull()

    override fun fingerprintPhrase(): String? =
        if (!started) null else runCatching { ffi.fingerprintPhrase() }.getOrNull()

    override fun listenAddr(): String? =
        if (!started) null else runCatching { ffi.listenAddr() }.getOrNull()

    /** Pure core function — available before `start()`. */
    override fun fingerprintPhraseFor(publicKey: ByteArray): String? =
        runCatching { ffiFingerprintPhraseFor(publicKey) }
            .onFailure { Log.w(TAG, "fingerprintPhraseFor failed", it) }
            .getOrNull()

    override fun deviceIdFor(publicKey: ByteArray): String? =
        runCatching { ffiDeviceIdFor(publicKey) }
            .onFailure { Log.w(TAG, "deviceIdFor failed", it) }
            .getOrNull()

    override suspend fun trustedPeers(): List<TrustedPeerInfo> = withContext(Dispatchers.IO) {
        if (!started) return@withContext emptyList()
        runCatching { ffi.trustedPeers().map(::adaptTrustedPeer) }
            .onFailure { Log.w(TAG, "trustedPeers failed", it) }
            .getOrDefault(emptyList())
            .also { peers ->
                peers.forEach { peer ->
                    peerCache[peer.deviceId] = PeerRef(
                        id = peer.deviceId,
                        displayName = peer.displayName,
                        fingerprint = peer.fingerprint,
                        paired = true,
                        publicKey = peer.publicKey,
                        deviceType = peer.deviceType,
                    )
                }
            }
    }

    override suspend fun revokePeer(publicKey: ByteArray): Boolean = withContext(Dispatchers.IO) {
        if (publicKey.size != PUBKEY_SIZE_BYTES) {
            Log.w(TAG, "revokePeer: not a 32-byte key, skipping core revoke")
            return@withContext false
        }
        runCatching { ffi.revokePeer(publicKey) }
            .onFailure { Log.w(TAG, "revokePeer failed", it) }
            .getOrDefault(false)
            .also { removed ->
                if (removed) peerCache.remove(deviceIdFor(publicKey))
                Log.i(TAG, "revokePeer(${deviceIdFor(publicKey)}) -> $removed")
            }
    }

    override fun beginPairingQr(): String = ffi.beginPairingQr()

    /** Pure core function: no engine, no I/O. */
    override fun parsePairingCode(payload: String): PairingCodeInfo? =
        runCatching { parsePairingQr(payload.trim()) }
            .getOrNull()
            ?.let {
                PairingCodeInfo(
                    deviceId = it.deviceId,
                    deviceName = it.deviceName,
                    hints = it.hints,
                    expired = it.expired,
                )
            }

    override fun pairWithQr(payload: String) {
        scope.launch(Dispatchers.IO) {
            // Parsed only for the failure path.
            val info = runCatching { parsePairingQr(payload.trim()) }.getOrNull()
            Log.i(TAG, "pairWithQr: parsed=${info != null} peer=${info?.deviceId} hints=${info?.hints}")
            val startedAt = SystemClock.elapsedRealtime()
            try {
                val peerId = ffi.pairWithQr(payload.trim())
                Log.i(TAG, "pairWithQr: paired with $peerId after ${elapsed(startedAt)}")
            } catch (e: Throwable) {
                // The core emits no PairingResult when the blocking call throws.
                Log.w(TAG, "pairWithQr failed after ${elapsed(startedAt)}", e)
                _events.emit(
                    CoreEvent.PairingResult(
                        peerId = info?.deviceId.orEmpty(),
                        peer = info?.let {
                            PeerRef(
                                id = it.deviceId,
                                displayName = it.deviceName ?: it.deviceId,
                                fingerprint = fingerprintPhraseFor(it.pubkey) ?: it.deviceId,
                                paired = false,
                                publicKey = it.pubkey,
                            )
                        },
                        success = false,
                        message = pairingMessage(e),
                    )
                )
            }
        }
    }

    override fun requestSasPairing(peerId: String) {
        scope.launch(Dispatchers.IO) {
            runCatching { ffi.requestSasPairing(peerId) }.onFailure { error ->
                Log.w(TAG, "requestSasPairing($peerId) failed", error)
                _events.emit(
                    CoreEvent.PairingResult(peerId, peerRef(peerId), false, pairingMessage(error))
                )
            }
        }
    }

    override fun confirmSas(peerId: String, accepted: Boolean) {
        scope.launch(Dispatchers.IO) {
            runCatching { ffi.confirmSas(peerId, accepted) }.onFailure { error ->
                // The UI waits on a PairingResult the core will never emit if this threw.
                Log.w(TAG, "confirmSas($peerId, $accepted) failed", error)
                if (accepted) {
                    _events.emit(
                        CoreEvent.PairingResult(peerId, peerRef(peerId), false, pairingMessage(error))
                    )
                }
            }
        }
    }

    override suspend fun beginInternetTicket(): String = withContext(Dispatchers.IO) {
        try {
            ffi.beginInternetTicket()
        } catch (e: Throwable) {
            Log.w(TAG, "beginInternetTicket failed", e)
            throw CoreException(userMessage(e), e)
        }
    }

    override suspend fun setRelayUrls(urls: List<String>?) = withContext(Dispatchers.IO) {
        // Closing the bound iroh endpoint is a block_on shutdown; never the main thread.
        try {
            ffi.setRelayUrls(urls)
        } catch (e: Throwable) {
            Log.w(TAG, "setRelayUrls($urls) failed", e)
            throw CoreException(userMessage(e), e)
        }
    }

    override fun endInternetTicket() {
        scope.launch(Dispatchers.IO) {
            runCatching { ffi.endInternetTicket() }
                .onFailure { Log.w(TAG, "endInternetTicket failed", it) }
        }
    }

    /** Pure core function: no engine, no I/O. */
    override fun parseTicket(ticket: String): TicketInfo? =
        runCatching { parseInternetTicket(ticket.trim()) }
            .getOrNull()
            ?.let {
                TicketInfo(
                    deviceId = it.deviceId,
                    publicKey = it.nodeId,
                    deviceName = it.deviceName,
                    relay = it.relay,
                    expired = it.expired,
                )
            }

    override fun redeemTicket(ticket: String) {
        scope.launch(Dispatchers.IO) {
            // Parsed only for the failure path.
            val info = parseTicket(ticket)
            Log.i(TAG, "redeemTicket: parsed=${info != null} peer=${info?.deviceId} relay=${info?.relay}")
            val startedAt = SystemClock.elapsedRealtime()
            try {
                val peerId = ffi.redeemTicket(ticket.trim())
                Log.i(TAG, "redeemTicket: paired with $peerId after ${elapsed(startedAt)}")
            } catch (e: Throwable) {
                // The core emits no PairingResult when the blocking call throws.
                Log.w(TAG, "redeemTicket failed after ${elapsed(startedAt)}", e)
                _events.emit(
                    CoreEvent.PairingResult(
                        peerId = info?.deviceId.orEmpty(),
                        peer = info?.let {
                            PeerRef(
                                id = it.deviceId,
                                displayName = it.deviceName ?: it.deviceId,
                                fingerprint = fingerprintPhraseFor(it.publicKey) ?: it.deviceId,
                                paired = false,
                                publicKey = it.publicKey,
                            )
                        },
                        success = false,
                        message = pairingMessage(e),
                    )
                )
            }
        }
    }

    override suspend fun connectPeer(addr: String, expectedPublicKey: ByteArray?): String =
        withContext(Dispatchers.IO) {
            val expected = expectedPublicKey?.takeIf { it.size == PUBKEY_SIZE_BYTES }
            Log.i(TAG, "connectPeer($addr) pinned=${expected != null}")
            try {
                ffi.connectPeer(addr, expected)
            } catch (e: Throwable) {
                throw CoreException(userMessage(e), e)
            }
        }

    override suspend fun send(peerId: String, uris: List<Uri>): TransferId =
        withContext(Dispatchers.IO) {
            // The core cannot read a ContentResolver, so content:// URIs are copied first.
            val paths = materialise(uris)
            if (paths.isEmpty()) throw CoreException("Nothing to send")
            try {
                ffi.send(peerId, paths)
            } catch (e: Throwable) {
                throw CoreException(userMessage(e), e)
            }
        }

    override fun respondToOffer(transferId: TransferId, acceptedFileIds: List<FileId>) {
        scope.launch(Dispatchers.IO) {
            runCatching { ffi.respondToOffer(transferId, acceptedFileIds) }
                .onFailure { error ->
                    Log.w(TAG, "respondToOffer failed", error)
                    _events.emit(CoreEvent.TransferError(transferId, userMessage(error), false))
                }
        }
    }

    override fun cancel(transferId: TransferId) {
        scope.launch(Dispatchers.IO) {
            runCatching { ffi.cancel(transferId, null) }
                .onFailure { Log.w(TAG, "cancel failed", it) }
        }
    }

    /** file:// items are used in place; content:// is copied. */
    private fun materialise(uris: List<Uri>): List<String> {
        if (uris.isEmpty()) return emptyList()
        val outbox = File(appContext.filesDir, "outbox/${UUID.randomUUID()}")
        return uris.mapNotNull { uri ->
            when (uri.scheme) {
                "file", null -> uri.path?.takeIf { File(it).isFile }
                else -> runCatching {
                    if (!outbox.isDirectory) outbox.mkdirs()
                    val target = uniqueIn(outbox, displayNameOf(uri))
                    appContext.contentResolver.openInputStream(uri)?.use { input ->
                        target.outputStream().use { input.copyTo(it) }
                    } ?: return@runCatching null
                    target.absolutePath
                }.getOrElse {
                    Log.w(TAG, "could not stage $uri for sending", it)
                    null
                }
            }
        }
    }

    private fun displayNameOf(uri: Uri): String {
        runCatching {
            appContext.contentResolver
                .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                ?.use { cursor ->
                    if (cursor.moveToFirst() && !cursor.isNull(0)) return cursor.getString(0)
                }
        }
        return uri.lastPathSegment?.substringAfterLast('/')?.takeIf { it.isNotBlank() }
            ?: "shared-file"
    }

    private fun uniqueIn(dir: File, name: String): File {
        var candidate = File(dir, name)
        var counter = 2
        while (candidate.exists()) {
            val dot = name.lastIndexOf('.')
            val base = if (dot > 0) name.substring(0, dot) else name
            val ext = if (dot > 0) name.substring(dot) else ""
            candidate = File(dir, "$base ($counter)$ext")
            counter++
        }
        return candidate
    }

    private fun adapt(event: FfiEvent): CoreEvent? = when (event) {
        is FfiEvent.PeerConnected -> {
            val peer = PeerRef(
                id = event.peerId,
                displayName = event.deviceName,
                fingerprint = event.fingerprint,
                paired = event.trusted,
                publicKey = event.peerPubkey,
                deviceType = event.deviceType?.toApp(),
            )
            peerCache[event.peerId] = peer
            CoreEvent.PeerConnected(peer)
        }

        is FfiEvent.PeerDisconnected -> CoreEvent.PeerDisconnected(event.peerId)

        is FfiEvent.PairingSas -> CoreEvent.PairingSas(peerRef(event.peerId), event.code)

        is FfiEvent.TicketRedeemed -> CoreEvent.TicketRedeemed(
            PeerRef(
                id = event.peerId,
                displayName = event.deviceName.ifBlank { event.peerId },
                fingerprint = fingerprintPhraseFor(event.peerPubkey).orEmpty(),
                // The internet path does not pair (PROTOCOL.md §9.4).
                paired = false,
                publicKey = event.peerPubkey,
                deviceType = peerCache[event.peerId]?.deviceType,
            )
        )

        is FfiEvent.PairingResult -> {
            val cached = peerCache[event.peerId]
            val peer = PeerRef(
                id = event.peerId,
                // On success `message` is the peer's device name (DESIGN.md §4).
                displayName = cached?.displayName
                    ?: event.message?.takeIf { event.success }
                    ?: event.peerId,
                fingerprint = event.fingerprint,
                paired = event.success,
                publicKey = event.peerPubkey,
                deviceType = cached?.deviceType,
            )
            if (event.success) peerCache[event.peerId] = peer
            CoreEvent.PairingResult(
                peerId = event.peerId,
                peer = peer,
                success = event.success,
                message = event.message,
            )
        }

        is FfiEvent.IncomingOffer -> {
            val from = PeerRef(
                id = event.peerId,
                displayName = event.fromName,
                fingerprint = event.fingerprint,
                paired = event.trusted,
                publicKey = event.peerPubkey,
                deviceType = event.deviceType?.toApp(),
            )
            peerCache[event.peerId] = from
            CoreEvent.IncomingOffer(event.transferId, from, event.files.map(::adaptFile))
        }

        is FfiEvent.TransferStarted -> CoreEvent.TransferStarted(
            transferId = event.transferId,
            peer = peerRef(event.peerId),
            direction = when (event.direction) {
                FfiDirection.SEND -> TransferDirection.SEND
                FfiDirection.RECEIVE -> TransferDirection.RECEIVE
            },
            manifest = event.files.map(::adaptFile),
        )

        is FfiEvent.Progress -> CoreEvent.Progress(
            transferId = event.transferId,
            fileId = event.fileId,
            bytes = event.bytesDone.toLong(),
            totalBytes = event.totalBytes.toLong(),
            rate = event.rateBps.toLong(),
            etaSeconds = event.etaSecs.toLong(),
        )

        is FfiEvent.FileReady -> CoreEvent.FileReady(
            transferId = event.transferId,
            fileId = event.fileId,
            stagedPath = event.stagedPath,
            kind = when (event.kind) {
                FfiFileKind.PHOTO -> FileKind.PHOTO
                FfiFileKind.VIDEO -> FileKind.VIDEO
                FfiFileKind.DOCUMENT -> FileKind.DOCUMENT
            },
        )

        is FfiEvent.TransferDone -> CoreEvent.TransferDone(
            transferId = event.transferId,
            okFiles = event.okFiles.toInt(),
            failedFiles = event.failedFiles.toInt(),
            bytesTransferred = event.bytesTransferred.toLong(),
            durationMs = event.durationMs.toLong(),
        )

        is FfiEvent.TransferError -> CoreEvent.TransferError(
            transferId = event.transferId,
            message = event.error,
            resumable = event.resumable,
        )

        is FfiEvent.KeyChanged -> {
            val cached = peerCache[event.peerId]
            CoreEvent.KeyChanged(
                peer = PeerRef(
                    id = event.peerId,
                    displayName = cached?.displayName ?: event.peerId,
                    fingerprint = fingerprintPhraseFor(event.expectedPubkey) ?: event.peerId,
                    paired = true,
                    publicKey = event.expectedPubkey,
                    deviceType = cached?.deviceType,
                ),
                presentedFingerprint = event.presentedPubkey?.let(::fingerprintPhraseFor),
            )
        }
    }

    private fun adaptFile(file: uniffi.wooosh_core.OfferedFile) = FileMeta(
        id = file.fid,
        name = file.name,
        size = file.size.toLong(),
        mime = file.mime,
    )

    private fun adaptTrustedPeer(peer: FfiTrustedPeer) = TrustedPeerInfo(
        deviceId = peer.deviceId,
        publicKey = peer.pubkey,
        displayName = peer.deviceName,
        deviceType = peer.deviceType?.toApp(),
        fingerprint = peer.fingerprint,
        pairedAtMillis = peer.pairedAt.toLong() * 1000,
        lastSeenMillis = peer.lastSeen.toLong() * 1000,
    )

    private fun peerRef(peerId: String, paired: Boolean? = null): PeerRef {
        val cached = peerCache[peerId]
        return PeerRef(
            id = peerId,
            displayName = cached?.displayName ?: peerId,
            fingerprint = cached?.fingerprint ?: peerId,
            paired = paired ?: cached?.paired ?: false,
            publicKey = cached?.publicKey,
            deviceType = cached?.deviceType,
        )
    }

    private fun elapsed(startedAt: Long) = "${SystemClock.elapsedRealtime() - startedAt} ms"

    /** Every pairing sub-case is one `Pairing` variant, distinguished only by message text. */
    private fun pairingMessage(error: Throwable): String = appContext.getString(
        when {
            error is WoooshException.Connect || error is WoooshException.Io ->
                R.string.error_pairing_timeout

            error is WoooshException.Pairing -> {
                val detail = error.message.orEmpty()
                when {
                    detail.contains("expired", ignoreCase = true) ->
                        R.string.error_pairing_code_expired_detail

                    detail.contains("TOKEN_INVALID", ignoreCase = true) ->
                        R.string.error_pairing_code_rejected

                    detail.contains("timed out", ignoreCase = true) ->
                        R.string.error_pairing_timeout

                    detail.contains("rejected", ignoreCase = true) ->
                        R.string.error_pairing_declined

                    detail.contains("closed", ignoreCase = true) ->
                        R.string.error_pairing_dropped

                    else -> R.string.error_pairing_failed
                }
            }

            else -> messageRes(error)
        }
    )

    private fun userMessage(error: Throwable): String =
        appContext.getString(messageRes(error))

    /** The core's exception detail strings are untranslatable and log-shaped, never copy. */
    private fun messageRes(error: Throwable): Int = when (error) {
        is WoooshException.RelayFileTooLarge -> R.string.error_relay_file_too_large
        is WoooshException.PairingRequired -> R.string.error_pairing_required
        is WoooshException.VersionMismatch -> R.string.error_version_mismatch
        is WoooshException.KeyChanged -> R.string.error_key_changed
        is WoooshException.QrKeyMismatch -> R.string.error_qr_key_mismatch
        is WoooshException.InvalidQrPayload -> R.string.error_invalid_qr
        is WoooshException.Pairing -> R.string.error_pairing_failed
        is WoooshException.UnknownPeer -> R.string.error_unknown_peer
        is WoooshException.Connect -> R.string.error_connect
        is WoooshException.NotStarted -> R.string.error_not_started
        is WoooshException.AlreadyStarted -> R.string.error_already_started
        is WoooshException.Transfer -> R.string.error_transfer_failed
        is WoooshException.UnknownTransfer -> R.string.error_unknown_transfer
        is WoooshException.Protocol -> R.string.error_protocol
        is WoooshException.Crypto -> R.string.error_crypto
        is WoooshException.Io -> R.string.error_io
        is WoooshException.InvalidArgument -> R.string.error_transfer_failed
        else -> R.string.error_transfer_failed
    }

    private fun CoreVisibility.toFfi() = when (this) {
        CoreVisibility.EVERYONE -> FfiVisibility.EVERYONE
        CoreVisibility.PAIRED_ONLY -> FfiVisibility.PAIRED_ONLY
        CoreVisibility.OFF -> FfiVisibility.OFF
    }

    /** The FFI type is form-factor only, so outbound collapses to the nearest one. */
    private fun DeviceType.toFfi() = when (this) {
        DeviceType.IPHONE, DeviceType.ANDROID_PHONE -> FfiDeviceType.PHONE
        DeviceType.IPAD, DeviceType.ANDROID_TABLET -> FfiDeviceType.TABLET
        DeviceType.MAC -> FfiDeviceType.LAPTOP
        DeviceType.WINDOWS -> FfiDeviceType.DESKTOP
        // The FFI enum has no unknown member, so pick the likeliest rather than throw.
        DeviceType.UNKNOWN -> FfiDeviceType.PHONE
    }

    /** Never a guess: a wrong glyph is worse than a generic one, so the TXT `dt` wins. */
    private fun FfiDeviceType.toApp() = DeviceType.UNKNOWN

    private companion object {
        const val TAG = "WoooshRealCore"
        const val EVENT_BUFFER = 2048
        const val START_TIMEOUT_SECONDS = 15L
        const val PUBKEY_SIZE_BYTES = 32
    }
}
