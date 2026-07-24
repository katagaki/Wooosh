package com.tsubuzaki.WoooshGo.core

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import android.util.Log
import android.webkit.MimeTypeMap
import com.tsubuzaki.WoooshGo.peers.DeviceType
import java.io.File
import java.net.Inet4Address
import java.net.NetworkInterface
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlin.math.max
import kotlin.random.Random
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.wooosh_core.deviceIdFor as ffiDeviceIdFor
import uniffi.wooosh_core.fingerprintPhraseFor as ffiFingerprintPhraseFor
import uniffi.wooosh_core.parsePairingQr as ffiParsePairingQr

// Scripted stand-in for the Rust core, kept reachable behind the debug section so the
// UI flows (incoming offer, SAS pairing, KEY_CHANGED) can be exercised without a second
// device. [RealCore] is the default — see WoooshApplication.
//
// Progress ticks at a plausible Wi-Fi rate, completions are delayed, and staged files are
// really written to disk so the storage-routing path (MediaStore.Downloads) is exercised
// end to end.
//
// Identity values are NOT invented here: mock peers get a deterministic 32-byte key and
// their DeviceID / fingerprint come from the core's own exported `device_id_for` and
// `fingerprint_phrase_for`, so a mocked peer looks exactly like a real one to the UI.
class MockCore(
    context: Context,
    private val scope: CoroutineScope,
) : WoooshCore {

    private val appContext = context.applicationContext

    private val _events = MutableSharedFlow<CoreEvent>(replay = 0, extraBufferCapacity = 256)
    override val events: Flow<CoreEvent> = _events.asSharedFlow()

    @Volatile
    private var config: CoreConfig? = null

    @Volatile
    private var visibility: CoreVisibility = CoreVisibility.EVERYONE

    private val mockPublicKey = ByteArray(32).also { SecureRandom().nextBytes(it) }

    /** Stands in for the core's trust store so the Settings list has something to show. */
    private val pinned = ConcurrentHashMap<String, TrustedPeerInfo>()

    private val transferJobs = ConcurrentHashMap<TransferId, Job>()
    private val pendingOffers = ConcurrentHashMap<TransferId, Pair<PeerRef, List<FileMeta>>>()
    private val pendingSas = ConcurrentHashMap<String, PeerRef>()
    private val transferSeq = AtomicInteger(0)
    private val transferStarts = ConcurrentHashMap<TransferId, Long>()

    override fun start(config: CoreConfig) {
        this.config = config
        File(config.stagingDir).mkdirs()
    }

    override fun stop() {
        transferJobs.values.forEach { it.cancel() }
        transferJobs.clear()
    }

    override fun setVisibility(mode: CoreVisibility) {
        visibility = mode
    }

    override fun deviceId(): String? = deviceIdFor(mockPublicKey)

    override fun fingerprintPhrase(): String? = fingerprintPhraseFor(mockPublicKey)

    override fun listenAddr(): String = localAddressHint()

    override fun fingerprintPhraseFor(publicKey: ByteArray): String? =
        runCatching { ffiFingerprintPhraseFor(publicKey) }
            .onFailure { Log.w(TAG, "fingerprintPhraseFor failed", it) }
            .getOrNull()

    override fun deviceIdFor(publicKey: ByteArray): String? =
        runCatching { ffiDeviceIdFor(publicKey) }
            .onFailure { Log.w(TAG, "deviceIdFor failed", it) }
            .getOrNull()

    override suspend fun trustedPeers(): List<TrustedPeerInfo> =
        pinned.values.sortedWith(compareBy({ it.pairedAtMillis }, { it.deviceId }))

    override suspend fun revokePeer(publicKey: ByteArray): Boolean =
        pinned.remove(deviceIdFor(publicKey)) != null

    override fun beginPairingQr(): String {
        val token = ByteArray(32).also { SecureRandom().nextBytes(it) }
        val exp = System.currentTimeMillis() / 1000 + 120
        // Payload layout from PROTOCOL.md §4.2.
        return "wooosh-pair:1" +
            "?pk=${b64url(mockPublicKey)}" +
            "?tok=${b64url(token)}" +
            "?hints=${localAddressHint()}" +
            "?exp=$exp"
    }

    /**
     * The real parser is a pure core function with no engine behind it, so the mock uses
     * the same one — a mocked pairing code parses exactly like a real one. Falls back to
     * the mock's own `?`-separated layout, which the core parser does not accept.
     */
    override fun parsePairingCode(payload: String): PairingCodeInfo? {
        val trimmed = payload.trim()
        runCatching { ffiParsePairingQr(trimmed) }.getOrNull()?.let {
            return PairingCodeInfo(it.deviceId, it.deviceName, it.hints, it.expired)
        }
        val pk = extractField(trimmed, "pk")
        if (!trimmed.startsWith("wooosh-pair:1?") || pk.isNullOrBlank()) return null
        val expiry = extractField(trimmed, "exp")?.toLongOrNull()
        return PairingCodeInfo(
            deviceId = pk,
            deviceName = null,
            hints = listOfNotNull(extractField(trimmed, "hints")),
            expired = expiry != null && expiry < System.currentTimeMillis() / 1000,
        )
    }

    override fun pairWithQr(payload: String) {
        scope.launch {
            delay(900) // simulated connect + PAIR_REQUEST/PAIR_ACCEPT round trip
            val trimmed = payload.trim()
            val pk = extractField(trimmed, "pk")
            if (!trimmed.startsWith("wooosh-pair:1?") || pk.isNullOrBlank()) {
                _events.emit(
                    CoreEvent.PairingResult("", null, false, "Not a valid Wooosh pairing code")
                )
                return@launch
            }
            val peer = mockPeer(seed = pk, name = null, paired = true)
            pin(peer)
            _events.emit(CoreEvent.PairingResult(peer.id, peer, true, peer.displayName))
        }
    }

    override fun requestSasPairing(peerId: String) {
        scope.launch {
            val peer = pendingSas[peerId] ?: return@launch
            _events.emit(CoreEvent.PairingSas(peer, "%06d".format(Random.nextInt(1_000_000))))
        }
    }

    override fun confirmSas(peerId: String, accepted: Boolean) {
        val peer = pendingSas.remove(peerId)
        scope.launch {
            delay(400) // simulated PAIR_CONFIRM from the other side
            if (peer == null) return@launch
            if (accepted) {
                val paired = peer.copy(paired = true)
                pin(paired)
                _events.emit(
                    CoreEvent.PairingResult(paired.id, paired, true, paired.displayName)
                )
            } else {
                _events.emit(
                    CoreEvent.PairingResult(peer.id, peer, false, "Pairing cancelled")
                )
            }
        }
    }

    override suspend fun connectPeer(addr: String, expectedPublicKey: ByteArray?): String {
        // No transport in the mock: pretend the address resolves to a stable peer id.
        delay(150)
        return mockPeer(seed = addr, name = null, paired = false).id
    }

    override suspend fun send(peerId: String, uris: List<Uri>): TransferId {
        val transferId = newTransferId()
        val peer = peerFor(peerId)
        val manifest = withContext(Dispatchers.IO) {
            uris.mapIndexed { index, uri -> resolveMeta(uri, index) }
        }
        if (manifest.isEmpty()) throw CoreException("Nothing to send")
        val job = scope.launch(Dispatchers.IO) {
            _events.emit(CoreEvent.TransferStarted(transferId, peer, TransferDirection.SEND, manifest))
            transferStarts[transferId] = System.currentTimeMillis()
            delay(350) // simulated OFFER -> DECISION round trip
            runTransfer(transferId, manifest, receive = false)
        }
        transferJobs[transferId] = job
        return transferId
    }

    override fun respondToOffer(transferId: TransferId, acceptedFileIds: List<FileId>) {
        val (from, manifest) = pendingOffers.remove(transferId) ?: return
        if (acceptedFileIds.isEmpty()) return // declined
        val accepted = manifest.filter { it.id in acceptedFileIds.toSet() }
        val job = scope.launch(Dispatchers.IO) {
            _events.emit(CoreEvent.TransferStarted(transferId, from, TransferDirection.RECEIVE, accepted))
            transferStarts[transferId] = System.currentTimeMillis()
            delay(250)
            runTransfer(transferId, accepted, receive = true)
        }
        transferJobs[transferId] = job
    }

    override fun cancel(transferId: TransferId) {
        transferJobs.remove(transferId)?.cancel()
        pendingOffers.remove(transferId)
        scope.launch {
            _events.emit(CoreEvent.TransferError(transferId, "Transfer cancelled", resumable = false))
        }
    }

    // ---------------------------------------------------------------- debug hooks

    /** Debug: simulate a nearby device offering [fileCount] files ("Everyone" mode consent path). */
    fun debugSimulateIncomingOffer(fileCount: Int) {
        val transferId = newTransferId()
        val from = mockPeer(
            seed = "mock-kana-mbp",
            name = "Kana's MacBook Pro",
            paired = false,
            deviceType = DeviceType.MAC,
        )
        val manifest = if (fileCount <= 3) {
            listOf(
                FileMeta(0u, "IMG_4021.jpeg", 2_437_921, "image/jpeg"),
                FileMeta(1u, "Clip 2026-07-12.mp4", 3_882_004, "video/mp4"),
                FileMeta(2u, "Itinerary.pdf", 348_113, "application/pdf"),
            ).take(fileCount.coerceAtLeast(1))
        } else {
            (0 until fileCount).map { i ->
                FileMeta(
                    i.toUInt(),
                    "Scan %03d.png".format(i + 1),
                    180_000L + Random.nextInt(220_000),
                    "image/png",
                )
            }
        }
        pendingOffers[transferId] = from to manifest
        scope.launch { _events.emit(CoreEvent.IncomingOffer(transferId, from, manifest)) }
    }

    /** Debug: simulate a peer starting SAS numeric-comparison pairing (PROTOCOL.md §4.3). */
    fun debugSimulateIncomingSas() {
        val peer = mockPeer(
            seed = "mock-shiro-pc",
            name = "Shiro's PC",
            paired = false,
            deviceType = DeviceType.WINDOWS,
        )
        pendingSas[peer.id] = peer
        val code = "%06d".format(Random.nextInt(1_000_000))
        scope.launch { _events.emit(CoreEvent.PairingSas(peer, code)) }
    }

    /** Debug: simulate a pinned peer presenting a different key (PROTOCOL.md §4.5). */
    fun debugSimulateKeyChanged() {
        val expected = mockPeer(
            seed = "mock-kana-mbp",
            name = "Kana's MacBook Pro",
            paired = true,
            deviceType = DeviceType.MAC,
        )
        val presented = mockKey("mock-kana-mbp-newkey")
        scope.launch {
            _events.emit(
                CoreEvent.KeyChanged(expected, fingerprintPhraseFor(presented))
            )
        }
    }

    // ---------------------------------------------------------------- internals

    private suspend fun runTransfer(transferId: TransferId, files: List<FileMeta>, receive: Boolean) {
        try {
            val total = files.sumOf { it.size }
            var completedBytes = 0L
            for (file in files) {
                // Effective rate keeps every file under ~4 s of simulation, floor ~2.5 MB/s.
                val baseRate = max(2_500_000L, file.size / 4)
                var done = 0L
                while (done < file.size) {
                    delay(TICK_MS)
                    val rate = baseRate + Random.nextLong(-baseRate / 4, baseRate / 4)
                    done = (done + rate * TICK_MS / 1000).coerceAtMost(file.size)
                    val remaining = (total - completedBytes - done).coerceAtLeast(0)
                    _events.emit(
                        CoreEvent.Progress(
                            transferId = transferId,
                            fileId = file.id,
                            bytes = done,
                            totalBytes = file.size,
                            rate = rate,
                            etaSeconds = if (rate > 0) remaining / rate else -1,
                        )
                    )
                }
                completedBytes += file.size
                if (receive) {
                    val staged = writeStagedFile(transferId, file)
                    _events.emit(
                        CoreEvent.FileReady(transferId, file.id, staged.absolutePath, kindFor(file.mime))
                    )
                }
            }
            delay(300) // simulated final DONE round trip
            val startedAt = transferStarts.remove(transferId) ?: System.currentTimeMillis()
            _events.emit(
                CoreEvent.TransferDone(
                    transferId = transferId,
                    okFiles = files.size,
                    failedFiles = 0,
                    bytesTransferred = total,
                    durationMs = System.currentTimeMillis() - startedAt,
                )
            )
        } finally {
            transferJobs.remove(transferId)
        }
    }

    private fun writeStagedFile(transferId: TransferId, meta: FileMeta): File {
        val stagingDir = File(config?.stagingDir ?: appContext.filesDir.resolve("staging").absolutePath)
        val dir = File(stagingDir, transferId).apply { mkdirs() }
        val file = File(dir, meta.id.toString())
        // Cap actual bytes so huge simulated manifests never thrash disk.
        val bytesToWrite = meta.size.coerceAtMost(4L * 1024 * 1024)
        file.outputStream().buffered().use { out ->
            val chunk = ByteArray(64 * 1024).also { Random.nextBytes(it) }
            var written = 0L
            while (written < bytesToWrite) {
                val n = minOf(chunk.size.toLong(), bytesToWrite - written).toInt()
                out.write(chunk, 0, n)
                written += n
            }
        }
        return file
    }

    private fun resolveMeta(uri: Uri, index: Int): FileMeta {
        var name: String? = null
        var size = -1L
        when (uri.scheme) {
            "content" -> runCatching {
                appContext.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        if (nameIndex >= 0) name = cursor.getString(nameIndex)
                        val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                        if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) size = cursor.getLong(sizeIndex)
                    }
                }
            }

            "file" -> uri.path?.let { path ->
                File(path).let {
                    name = it.name
                    size = it.length()
                }
            }
        }
        val finalName = name?.takeIf { it.isNotBlank() } ?: uri.lastPathSegment ?: "file-$index"
        val mime = (if (uri.scheme == "content") appContext.contentResolver.getType(uri) else null)
            ?: guessMime(finalName)
        return FileMeta(
            id = index.toUInt(),
            name = finalName,
            size = if (size > 0) size else 1_500_000L,
            mime = mime,
        )
    }

    private fun guessMime(name: String): String {
        val ext = name.substringAfterLast('.', "").lowercase()
        return MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
            ?: "application/octet-stream"
    }

    private fun kindFor(mime: String): FileKind = when {
        mime.startsWith("image/") -> FileKind.PHOTO
        mime.startsWith("video/") -> FileKind.VIDEO
        else -> FileKind.DOCUMENT
    }

    private fun newTransferId(): TransferId =
        "t${transferSeq.incrementAndGet()}-${System.currentTimeMillis()}"

    private fun extractField(payload: String, key: String): String? =
        payload.substringAfter('?', "")
            .split('?', '&')
            .firstOrNull { it.startsWith("$key=") }
            ?.substringAfter('=')
            ?.takeIf { it.isNotBlank() }

    /** Deterministic 32-byte stand-in key for a mock peer — never a real identity. */
    private fun mockKey(seedMaterial: String): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(seedMaterial.toByteArray())

    /**
     * Builds a peer the way the core would: the key is invented, but the DeviceID and the
     * fingerprint phrase are derived by the core's own exported functions.
     */
    private fun mockPeer(
        seed: String,
        name: String?,
        paired: Boolean,
        deviceType: DeviceType? = DeviceType.ANDROID_PHONE,
    ): PeerRef {
        val key = mockKey(seed)
        val id = deviceIdFor(key) ?: seed
        return PeerRef(
            id = id,
            displayName = name ?: "Device ${id.take(4)}",
            fingerprint = fingerprintPhraseFor(key) ?: id,
            paired = paired,
            publicKey = key,
            deviceType = deviceType,
        )
    }

    private fun peerFor(peerId: String): PeerRef =
        pinned[peerId]?.let { trusted ->
            PeerRef(
                id = trusted.deviceId,
                displayName = trusted.displayName,
                fingerprint = trusted.fingerprint,
                paired = true,
                publicKey = trusted.publicKey,
                deviceType = trusted.deviceType,
            )
        } ?: PeerRef(
            id = peerId,
            displayName = "Device ${peerId.take(4)}",
            fingerprint = peerId,
            paired = false,
        )

    private fun pin(peer: PeerRef) {
        val key = peer.publicKey ?: return
        val now = System.currentTimeMillis()
        pinned[peer.id] = TrustedPeerInfo(
            deviceId = peer.id,
            publicKey = key,
            displayName = peer.displayName,
            deviceType = peer.deviceType,
            fingerprint = peer.fingerprint,
            pairedAtMillis = pinned[peer.id]?.pairedAtMillis ?: now,
            lastSeenMillis = now,
        )
    }

    private fun localAddressHint(): String = runCatching {
        NetworkInterface.getNetworkInterfaces().asSequence()
            .filter { it.isUp && !it.isLoopback }
            .flatMap { it.inetAddresses.asSequence() }
            .filterIsInstance<Inet4Address>()
            .firstOrNull()
            ?.hostAddress
    }.getOrNull()?.let { "$it:42000" } ?: "0.0.0.0:0"

    private fun b64url(bytes: ByteArray): String =
        Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    private companion object {
        const val TAG = "WoooshMockCore"
        const val TICK_MS = 140L
    }
}
