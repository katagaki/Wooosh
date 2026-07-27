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
    val savedUri: Uri? = null,
) {
    /** Routing has run and either placed the file or given up. */
    val isSettled: Boolean get() = routed || error != null
}

data class TransferUi(
    val id: TransferId,
    val peerName: String,
    val direction: TransferDirection,
    val files: List<FileState>,
    val rate: Long = 0,
    val etaSeconds: Long = -1,
    val status: TransferStatus = TransferStatus.RUNNING,
    val message: String? = null,
    /** Reported by the core on TransferDone; 0 while running. */
    val durationMs: Long = 0,
) {
    val totalBytes: Long get() = files.sumOf { it.meta.size }
    val transferredBytes: Long get() = files.sumOf { it.bytes }
}

/**
 * An unpaired receiver is told to compare the sender's fingerprint (PROTOCOL.md §4.4), so
 * [peerIsPaired] false means the sender must display it during this window.
 */
data class OutgoingOffer(
    val transferId: TransferId,
    val peerName: String,
    val peerIsPaired: Boolean,
    val fileCount: Int,
)

class TransferManager(
    private val context: Context,
    private val scope: CoroutineScope,
    private val core: WoooshCore,
    private val trustStore: TrustStore,
    private val registry: PeerRegistry,
) {

    private val storageRouter = StorageRouter(context)
    private val receivedNotifier = ReceivedNotifier(context)

    private val notifiedReceived = ConcurrentHashMap.newKeySet<TransferId>()

    private val _transfers = MutableStateFlow<List<TransferUi>>(emptyList())
    val transfers: StateFlow<List<TransferUi>> = _transfers.asStateFlow()

    /** A connect plus a consent round trip, across which the service must stay up. */
    private val pendingStarts = ConcurrentHashMap.newKeySet<String>()
    private val _pending = MutableStateFlow(0)
    private val pendingSeq = AtomicInteger(0)

    val hasActiveTransfers: StateFlow<Boolean> = _transfers
        .map { list -> list.any { it.status == TransferStatus.RUNNING } }
        .stateIn(scope, SharingStarted.Eagerly, false)

    val serviceNeeded: StateFlow<Boolean> = _transfers
        .combine(_pending) { list, pending ->
            pending > 0 || list.any { it.status == TransferStatus.RUNNING }
        }
        .stateIn(scope, SharingStarted.Eagerly, false)

    private val _pendingOffer = MutableStateFlow<CoreEvent.IncomingOffer?>(null)
    val pendingOffer: StateFlow<CoreEvent.IncomingOffer?> = _pendingOffer.asStateFlow()

    private val _outgoingOffers = MutableStateFlow<List<OutgoingOffer>>(emptyList())
    val outgoingOffers: StateFlow<List<OutgoingOffer>> = _outgoingOffers.asStateFlow()

    private val _errors = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val errors: SharedFlow<String> = _errors.asSharedFlow()

    private val requestedPeerNames = ConcurrentHashMap<TransferId, String>()

    /** Never written to disk: the internet path does not pair (PROTOCOL.md §9.4). */
    private val ticketPeers = ConcurrentHashMap.newKeySet<String>()

    private val _ticketRedeemedPeerId = MutableStateFlow<String?>(null)
    val ticketRedeemedPeerId: StateFlow<String?> = _ticketRedeemedPeerId.asStateFlow()

    /** A ticket is single-use; left set, the next send tab visit fires against a dead code. */
    fun clearTicketRedeemedPeer() {
        _ticketRedeemedPeerId.value = null
    }

    private val manifests = ConcurrentHashMap<TransferId, Map<FileId, FileMeta>>()

    /** `TransferDone` does not name its peer, and the ticket rows need to know. */
    private val peerIdByTransfer = ConcurrentHashMap<TransferId, String>()

    /** `Wooosh/<yyyy-MM-dd>` when the transfer has > 20 files, else null (DESIGN.md §6). */
    private val subfolders = ConcurrentHashMap<TransferId, String>()

    init {
        scope.launch {
            core.events.collect(::onEvent)
        }
    }

    fun sendToPeer(peer: Peer, uris: List<Uri>) {
        val addr = peer.address
        // An internet row (PROTOCOL.md §9) never has an mDNS address, only a DeviceID.
        val established = peer.peerId?.takeIf { addr == null }
        if (addr == null && established == null) {
            scope.launch {
                _errors.emit(context.getString(R.string.error_still_resolving, peer.displayName))
            }
            return
        }
        val pendingKey = newPendingKey()
        beginPending(pendingKey)
        scope.launch(Dispatchers.IO) {
            try {
                // Always pin: the core's address fallback misses a peer that moved.
                val pinnedKey = trustStore.pinnedKeyFor(peer.peerId)
                Log.i(
                    TAG,
                    "connect ${peer.displayName} at ${addr ?: "established connection"} " +
                        "deviceId=${peer.peerId ?: "unknown"} pinned=${pinnedKey != null}",
                )
                val peerId = established ?: core.connectPeer(addr!!, pinnedKey)
                registry.attachPeerId(peer.rid, peerId)
                persistReadGrants(uris)
                val transferId = core.send(peerId, uris)
                requestedPeerNames[transferId] = peer.displayName
                // DECISION can take two minutes; the service must survive that wait.
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
                // Core failures are internal English; never surfaced raw.
                _errors.emit(context.getString(R.string.error_send_failed_body))
            } finally {
                endPending(pendingKey)
            }
        }
    }

    /** The internet path: no mDNS row and no address to dial, only a DeviceID. */
    fun sendToPeerId(peerId: String, uris: List<Uri>) {
        beginPending(peerId)
        scope.launch(Dispatchers.IO) {
            try {
                persistReadGrants(uris)
                val transferId = core.send(peerId, uris)
                requestedPeerNames[transferId] = registry.peers.value
                    .firstOrNull { it.peerId == peerId }?.displayName
                    ?: context.getString(R.string.peer_unnamed)
                beginPending(transferId)
                _outgoingOffers.update { offers ->
                    offers.filterNot { it.transferId == transferId } + OutgoingOffer(
                        transferId = transferId,
                        peerName = requestedPeerNames[transferId].orEmpty(),
                        // The receiver consented by scanning, so no fingerprint is shown.
                        peerIsPaired = true,
                        fileCount = uris.size,
                    )
                }
            } catch (t: Throwable) {
                Log.w(TAG, "internet send to $peerId failed", t)
                _errors.emit(transferErrorMessage(context, t.message))
            } finally {
                endPending(peerId)
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
        // Also stored on TransferStarted, which a tiny file's FileReady can beat.
        manifests[offer.transferId] = offer.manifest.associateBy { it.id }
        beginPending(offer.transferId)
        core.respondToOffer(offer.transferId, acceptedFileIds)
    }

    /** Not via [acceptOffer]: [_pendingOffer] may belong to another, unpaired sender. */
    private fun autoAccept(offer: CoreEvent.IncomingOffer) {
        val ids = offer.manifest.map { it.id }
        if (ids.size > FILES_PER_SUBFOLDER_THRESHOLD) {
            subfolders[offer.transferId] =
                "Wooosh/${LocalDate.now().format(DateTimeFormatter.ISO_LOCAL_DATE)}"
        }
        manifests[offer.transferId] = offer.manifest.associateBy { it.id }
        beginPending(offer.transferId)
        core.respondToOffer(offer.transferId, ids)
    }

    fun declineOffer() {
        val offer = _pendingOffer.value ?: return
        _pendingOffer.value = null
        core.respondToOffer(offer.transferId, emptyList())
    }

    /** Drops the card at once, not on the core's echo, so the fingerprint stops showing. */
    fun cancel(transferId: TransferId) {
        clearOutgoingOffer(transferId)
        core.cancel(transferId)
    }

    fun cancelAll() {
        _outgoingOffers.value.forEach { cancel(it.transferId) }
        _transfers.value
            .filter { it.status == TransferStatus.RUNNING }
            .forEach { core.cancel(it.id) }
    }

    fun dismiss(transferId: TransferId) {
        _transfers.update { list ->
            list.filterNot { it.id == transferId && it.status != TransferStatus.RUNNING }
        }
    }

    private fun onEvent(event: CoreEvent) {
        when (event) {
            is CoreEvent.TransferStarted -> {
                endPending(event.transferId)
                clearOutgoingOffer(event.transferId)
                manifests[event.transferId] = event.manifest.associateBy { it.id }
                peerIdByTransfer[event.transferId] = event.peer.id
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

            is CoreEvent.TicketRedeemed -> {
                // Lets the peer's offer skip a consent sheet already given by scanning.
                ticketPeers.add(event.peer.id)
                registry.onConnected(
                    peerId = event.peer.id,
                    displayName = event.peer.displayName,
                    deviceType = event.peer.deviceType,
                    // A pin may exist, but the ticket is what admitted this peer.
                    viaTicket = true,
                )
                _ticketRedeemedPeerId.value = event.peer.id
            }

            is CoreEvent.IncomingOffer ->
                // Pairing already is the consent (PROTOCOL.md §4), and prompting anyway
                // trains the user to dismiss the unpaired sender's sheet too.
                if (event.from.paired || event.from.id in ticketPeers) {
                    autoAccept(event)
                } else {
                    _pendingOffer.value = event
                }

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

            is CoreEvent.TransferDone -> {
                updateTransfer(event.transferId) { transfer ->
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
                maybeNotifyReceived(event.transferId)
                endTicketSession(event.transferId)
            }

            is CoreEvent.TransferError -> {
                endPending(event.transferId)
                clearOutgoingOffer(event.transferId)
                var known = false
                updateTransfer(event.transferId) { transfer ->
                    known = true
                    transfer.copy(status = TransferStatus.FAILED, message = event.message)
                }
                // Failures before TransferStarted have no card to update.
                if (!known) scope.launch {
                    _errors.emit(
                        transferErrorMessage(context, event.message)
                    )
                }
                endTicketSession(event.transferId)
            }

            // HELLO is authoritative for identity and device type; the mDNS TXT is a hint.
            is CoreEvent.PeerConnected -> registry.onConnected(
                peerId = event.peer.id,
                displayName = event.peer.displayName,
                deviceType = event.peer.deviceType,
            )

            is CoreEvent.PeerDisconnected -> registry.onDisconnected(event.peerId)

            else -> Unit // handled by PairingManager
        }
    }

    /** A ticket authorises exactly one transfer, however long iroh keeps the link open. */
    private fun endTicketSession(transferId: TransferId) {
        val peerId = peerIdByTransfer.remove(transferId) ?: return
        registry.onTicketSessionEnded(peerId)
    }

    private fun summaryOf(event: CoreEvent.TransferDone, direction: TransferDirection): String {
        val sent = direction == TransferDirection.SEND
        val elapsed = formatDuration(context, event.durationMs)
        val res = context.resources
        return when {
            // Counts and elapsed time share one format string, never a fragment.
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
            result
                .onSuccess { Log.i(TAG, "routed $name to ${it.location} uri=${it.uri}") }
                .onFailure { Log.w(TAG, "routing ${event.stagedPath} failed", it) }
            updateTransfer(event.transferId) { transfer ->
                transfer.copy(
                    files = transfer.files.map { fileState ->
                        if (fileState.meta.id == event.fileId) {
                            result.fold(
                                onSuccess = { routed ->
                                    fileState.copy(
                                        routed = true,
                                        bytes = fileState.meta.size,
                                        savedUri = routed.uri,
                                    )
                                },
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
            // Routing runs off the event thread and can settle after TransferDone.
            maybeNotifyReceived(event.transferId)
        }
    }

    /** Only once routing has finished: a notification opening a staged file would be a lie. */
    private fun maybeNotifyReceived(transferId: TransferId) {
        val transfer = _transfers.value.firstOrNull { it.id == transferId } ?: return
        if (transfer.direction != TransferDirection.RECEIVE) return
        if (transfer.status != TransferStatus.DONE) return
        if (!transfer.files.all { it.isSettled }) return
        if (!notifiedReceived.add(transferId)) return
        receivedNotifier.notifyReceived(transfer)
    }

    private fun persistReadGrants(uris: List<Uri>) {
        // Only ACTION_OPEN_DOCUMENT grants are persistable; the rest are copied out.
        uris.forEach { uri ->
            runCatching {
                context.contentResolver.takePersistableUriPermission(
                    uri, Intent.FLAG_GRANT_READ_URI_PERMISSION
                )
            }
        }
    }

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
