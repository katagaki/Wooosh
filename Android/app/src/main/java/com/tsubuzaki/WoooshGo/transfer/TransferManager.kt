package com.tsubuzaki.WoooshGo.transfer

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.core.content.ContextCompat
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.core.CoreEvent
import com.tsubuzaki.WoooshGo.core.FileId
import com.tsubuzaki.WoooshGo.core.FileMeta
import com.tsubuzaki.WoooshGo.core.TransferDirection
import com.tsubuzaki.WoooshGo.core.TransferId
import com.tsubuzaki.WoooshGo.core.WoooshCore
import com.tsubuzaki.WoooshGo.core.transferErrorMessage
import com.tsubuzaki.WoooshGo.peers.Peer
import com.tsubuzaki.WoooshGo.peers.PeerRegistry
import com.tsubuzaki.WoooshGo.trust.TrustStore
import com.tsubuzaki.WoooshGo.ui.formatDuration
import java.io.File
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class TransferStatus { RUNNING, DONE, FAILED }

data class FileState(
    val meta: FileMeta,
    val bytes: Long = 0,
    /** True only once the file is routed to its final location (DESIGN.md §6). */
    val routed: Boolean = false,
    val error: String? = null,
)

data class TransferUi(
    val id: TransferId,
    val peerName: String,
    val direction: TransferDirection,
    val files: List<FileState>,
    val rate: Long = 0,
    val etaSeconds: Long = -1,
    val status: TransferStatus = TransferStatus.RUNNING,
    val message: String? = null,
    /** Wall-clock duration reported by the core on TransferDone; 0 while running. */
    val durationMs: Long = 0,
) {
    val totalBytes: Long get() = files.sumOf { it.meta.size }
    val transferredBytes: Long get() = files.sumOf { it.bytes }
}

/**
 * An outgoing OFFER on the wire, waiting for the receiver's DECISION (PROTOCOL.md §5).
 * The core only emits `TransferStarted` after the receiver accepts, so this is the only
 * send-side state during that window.
 *
 * That window is the verification ceremony: an unpaired receiver is told to compare the
 * sender's fingerprint (PROTOCOL.md §4.4), so the sender must be showing it at the same
 * moment. [peerIsPaired] gates that — for a paired peer the phrase is noise.
 */
data class OutgoingOffer(
    val transferId: TransferId,
    val peerName: String,
    val peerIsPaired: Boolean,
    val fileCount: Int,
)

/**
 * App-scoped transfer orchestration: consumes the core event stream, exposes UI state,
 * routes finished files (DESIGN.md §6), and drives the foreground service (DESIGN.md §7).
 */
class TransferManager(
    private val context: Context,
    private val scope: CoroutineScope,
    private val core: WoooshCore,
    private val trustStore: TrustStore,
    private val registry: PeerRegistry,
) {

    private val storageRouter = StorageRouter(context)

    private val _transfers = MutableStateFlow<List<TransferUi>>(emptyList())
    val transfers: StateFlow<List<TransferUi>> = _transfers.asStateFlow()

    /**
     * Transfers requested but whose TransferStarted has not arrived: a connect plus an
     * OFFER/consent round trip sits in between, and the other user may take a while. The
     * foreground service must stay up across it.
     */
    private val pendingStarts = ConcurrentHashMap.newKeySet<String>()
    private val _pending = MutableStateFlow(0)
    private val pendingSeq = AtomicInteger(0)

    val hasActiveTransfers: StateFlow<Boolean> = _transfers
        .map { list -> list.any { it.status == TransferStatus.RUNNING } }
        .stateIn(scope, SharingStarted.Eagerly, false)

    /** Drives the foreground service: real running transfers OR a request in flight. */
    val serviceNeeded: StateFlow<Boolean> = _transfers
        .combine(_pending) { list, pending ->
            pending > 0 || list.any { it.status == TransferStatus.RUNNING }
        }
        .stateIn(scope, SharingStarted.Eagerly, false)

    private val _pendingOffer = MutableStateFlow<CoreEvent.IncomingOffer?>(null)
    val pendingOffer: StateFlow<CoreEvent.IncomingOffer?> = _pendingOffer.asStateFlow()

    /** Outgoing OFFERs waiting on the receiver's DECISION — see [OutgoingOffer]. */
    private val _outgoingOffers = MutableStateFlow<List<OutgoingOffer>>(emptyList())
    val outgoingOffers: StateFlow<List<OutgoingOffer>> = _outgoingOffers.asStateFlow()

    /** User-presentable failures that have no transfer card to attach to. */
    private val _errors = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val errors: SharedFlow<String> = _errors.asSharedFlow()

    /** Display names supplied at send() time — nicer than the core's peer ids. */
    private val requestedPeerNames = ConcurrentHashMap<TransferId, String>()

    /** manifest lookup for FileReady routing (name + MIME). */
    private val manifests = ConcurrentHashMap<TransferId, Map<FileId, FileMeta>>()

    /** `Wooosh/<yyyy-MM-dd>` when the transfer has > 20 files, else null (DESIGN.md §6). */
    private val subfolders = ConcurrentHashMap<TransferId, String>()

    init {
        scope.launch {
            core.events.collect(::onEvent)
        }
    }

    // ---------------------------------------------------------------- commands

    /**
     * Tap-to-send on a discovered row. Everything blocking happens off the main thread
     * inside the core adapter.
     */
    fun sendToPeer(peer: Peer, uris: List<Uri>) {
        val addr = peer.address
        if (addr == null) {
            scope.launch {
                _errors.emit(context.getString(R.string.error_still_resolving, peer.displayName))
            }
            return
        }
        val pendingKey = newPendingKey()
        beginPending(pendingKey)
        scope.launch(Dispatchers.IO) {
            try {
                // Always pin: the core's address-based fallback only covers reconnects to
                // the same ip:port, so passing the key is what covers a peer that moved
                // (PROTOCOL.md §4.5).
                val pinnedKey = trustStore.pinnedKeyFor(peer.peerId)
                Log.i(
                    TAG,
                    "connect ${peer.displayName} at $addr " +
                        "deviceId=${peer.peerId ?: "unknown"} pinned=${pinnedKey != null}",
                )
                val peerId = core.connectPeer(addr, pinnedKey)
                registry.attachPeerId(peer.rid, peerId)
                persistReadGrants(uris)
                val transferId = core.send(peerId, uris)
                requestedPeerNames[transferId] = peer.displayName
                // The core now sits on DECISION for up to two minutes. Hold the pending
                // slot on the transfer id, not the throwaway send key, so the foreground
                // service survives that wait.
                beginPending(transferId)
                val paired = trustStore.find(peerId) != null
                Log.i(TAG, "offer $transferId to ${peer.displayName} paired=$paired")
                _outgoingOffers.update { offers ->
                    offers.filterNot { it.transferId == transferId } + OutgoingOffer(
                        transferId = transferId,
                        peerName = peer.displayName,
                        peerIsPaired = paired,
                        fileCount = uris.size,
                    )
                }
            } catch (t: Throwable) {
                Log.w(TAG, "send to ${peer.displayName} ($addr) failed", t)
                // Core failures are internal English; never surface them raw.
                _errors.emit(context.getString(R.string.error_send_failed_body))
            } finally {
                endPending(pendingKey)
            }
        }
    }

    fun acceptOffer(acceptedFileIds: List<FileId>) {
        val offer = _pendingOffer.value ?: return
        _pendingOffer.value = null
        if (acceptedFileIds.isEmpty()) {
            core.respondToOffer(offer.transferId, emptyList())
            return
        }
        if (acceptedFileIds.size > FILES_PER_SUBFOLDER_THRESHOLD) {
            subfolders[offer.transferId] =
                "Wooosh/${LocalDate.now().format(DateTimeFormatter.ISO_LOCAL_DATE)}"
        }
        // The manifest is needed for FileReady routing even though TransferStarted
        // repeats it — the first FileReady can race that event on a tiny file.
        manifests[offer.transferId] = offer.manifest.associateBy { it.id }
        beginPending(offer.transferId)
        core.respondToOffer(offer.transferId, acceptedFileIds)
    }

    fun declineOffer() {
        val offer = _pendingOffer.value ?: return
        _pendingOffer.value = null
        core.respondToOffer(offer.transferId, emptyList())
    }

    /**
     * Cancels a running transfer or withdraws an offer the receiver has not answered.
     * The waiting card is dropped immediately rather than on the core's echo, so the
     * fingerprint stops being displayed the moment the user withdraws the offer.
     */
    fun cancel(transferId: TransferId) {
        clearOutgoingOffer(transferId)
        core.cancel(transferId)
    }

    fun cancelAll() {
        // Offers still waiting on a DECISION count as in-flight work too.
        _outgoingOffers.value.forEach { cancel(it.transferId) }
        _transfers.value
            .filter { it.status == TransferStatus.RUNNING }
            .forEach { core.cancel(it.id) }
    }

    /** Removes a finished/failed card from the UI. */
    fun dismiss(transferId: TransferId) {
        _transfers.update { list ->
            list.filterNot { it.id == transferId && it.status != TransferStatus.RUNNING }
        }
    }

    // ---------------------------------------------------------------- events

    private fun onEvent(event: CoreEvent) {
        when (event) {
            is CoreEvent.TransferStarted -> {
                endPending(event.transferId)
                // DECISION arrived: the comparison is over, so the fingerprint stops
                // being displayed.
                clearOutgoingOffer(event.transferId)
                manifests[event.transferId] = event.manifest.associateBy { it.id }
                if (event.direction == TransferDirection.RECEIVE &&
                    event.manifest.size > FILES_PER_SUBFOLDER_THRESHOLD
                ) {
                    subfolders.putIfAbsent(
                        event.transferId,
                        "Wooosh/${LocalDate.now().format(DateTimeFormatter.ISO_LOCAL_DATE)}",
                    )
                }
                val peerName = requestedPeerNames.remove(event.transferId)
                    ?: event.peer.displayName
                _transfers.update { list ->
                    list.filterNot { it.id == event.transferId } + TransferUi(
                        id = event.transferId,
                        peerName = peerName,
                        direction = event.direction,
                        files = event.manifest.map { FileState(it) },
                    )
                }
                ensureServiceRunning()
            }

            is CoreEvent.IncomingOffer -> _pendingOffer.value = event

            is CoreEvent.Progress -> updateTransfer(event.transferId) { transfer ->
                transfer.copy(
                    rate = event.rate,
                    etaSeconds = event.etaSeconds,
                    files = transfer.files.map { fileState ->
                        if (fileState.meta.id == event.fileId) {
                            fileState.copy(bytes = event.bytes)
                        } else fileState
                    },
                )
            }

            is CoreEvent.FileReady -> routeFile(event)

            is CoreEvent.TransferDone -> updateTransfer(event.transferId) { transfer ->
                transfer.copy(
                    status = TransferStatus.DONE,
                    message = summaryOf(event, transfer.direction),
                    durationMs = event.durationMs,
                    etaSeconds = 0,
                    files = transfer.files.map {
                        if (it.error == null) it.copy(bytes = it.meta.size) else it
                    },
                )
            }

            is CoreEvent.TransferError -> {
                endPending(event.transferId)
                // Declined, timed out or cancelled while waiting — the card goes away
                // either way; the message is surfaced below.
                clearOutgoingOffer(event.transferId)
                var known = false
                updateTransfer(event.transferId) { transfer ->
                    known = true
                    transfer.copy(status = TransferStatus.FAILED, message = event.message)
                }
                // Failures before TransferStarted have no card to update — surface them.
                if (!known) scope.launch {
                    _errors.emit(
                        transferErrorMessage(context, event.message)
                    )
                }
            }

            // HELLO is authoritative for identity and device type; the mDNS TXT the row
            // was created from is only a hint.
            is CoreEvent.PeerConnected -> registry.onConnected(
                peerId = event.peer.id,
                displayName = event.peer.displayName,
                deviceType = event.peer.deviceType,
            )

            else -> Unit // pairing events handled by PairingManager
        }
    }

    /** Completion line for the card; every number comes from the core's `TransferDone`. */
    private fun summaryOf(event: CoreEvent.TransferDone, direction: TransferDirection): String {
        val sent = direction == TransferDirection.SEND
        val elapsed = formatDuration(context, event.durationMs)
        val res = context.resources
        return when {
            // Counts go through the platform's plural machinery and the elapsed time is a
            // placeholder in the same format string: translators never get a fragment.
            event.okFiles == 0 && event.failedFiles > 0 -> res.getQuantityString(
                R.plurals.transfer_done_failed, event.failedFiles, event.failedFiles,
            )

            event.failedFiles > 0 -> {
                val total = event.okFiles + event.failedFiles
                res.getQuantityString(
                    R.plurals.transfer_done_partial, total, event.okFiles, total,
                )
            }

            elapsed.isEmpty() -> res.getQuantityString(
                if (sent) R.plurals.transfer_done_sent else R.plurals.transfer_done_received,
                event.okFiles, event.okFiles,
            )

            else -> res.getQuantityString(
                if (sent) R.plurals.transfer_done_sent_in else R.plurals.transfer_done_received_in,
                event.okFiles, event.okFiles, elapsed,
            )
        }
    }

    private fun routeFile(event: CoreEvent.FileReady) {
        val meta = manifests[event.transferId]?.get(event.fileId)
        scope.launch(Dispatchers.IO) {
            val staged = File(event.stagedPath)
            val name = meta?.name ?: staged.name
            val mime = meta?.mime ?: "application/octet-stream"
            val result = runCatching {
                storageRouter.routeToDownloads(staged, name, mime, subfolders[event.transferId])
            }
            result.onFailure { Log.w(TAG, "routing ${event.stagedPath} failed", it) }
            updateTransfer(event.transferId) { transfer ->
                transfer.copy(
                    files = transfer.files.map { fileState ->
                        if (fileState.meta.id == event.fileId) {
                            result.fold(
                                onSuccess = { fileState.copy(routed = true, bytes = fileState.meta.size) },
                                onFailure = {
                                    fileState.copy(
                                        error = context.getString(R.string.error_save_failed)
                                    )
                                },
                            )
                        } else fileState
                    },
                )
            }
        }
    }

    private fun persistReadGrants(uris: List<Uri>) {
        // Best effort: only ACTION_OPEN_DOCUMENT grants are persistable. Photo Picker and
        // share-sheet grants are not, so those items are copied out promptly instead.
        uris.forEach { uri ->
            runCatching {
                context.contentResolver.takePersistableUriPermission(
                    uri, Intent.FLAG_GRANT_READ_URI_PERMISSION
                )
            }
        }
    }

    /** Drops the waiting card for [transferId]; returns it when there was one. */
    private fun clearOutgoingOffer(transferId: TransferId): OutgoingOffer? {
        val removed = _outgoingOffers.value.firstOrNull { it.transferId == transferId }
        if (removed != null) {
            _outgoingOffers.update { list -> list.filterNot { it.transferId == transferId } }
        }
        return removed
    }

    private fun updateTransfer(transferId: TransferId, block: (TransferUi) -> TransferUi) {
        _transfers.update { list ->
            list.map { if (it.id == transferId) block(it) else it }
        }
    }

    private fun beginPending(key: String) {
        pendingStarts.add(key)
        _pending.value = pendingStarts.size
        ensureServiceRunning()
    }

    private fun endPending(key: String) {
        if (pendingStarts.remove(key)) _pending.value = pendingStarts.size
    }

    private fun newPendingKey() = "send-${pendingSeq.incrementAndGet()}"

    private fun ensureServiceRunning() {
        runCatching {
            ContextCompat.startForegroundService(context, Intent(context, TransferService::class.java))
        }.onFailure { Log.w(TAG, "could not start the transfer service", it) }
    }

    private companion object {
        const val TAG = "WoooshTransfers"
        const val FILES_PER_SUBFOLDER_THRESHOLD = 20
    }
}
