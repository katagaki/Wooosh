package com.tsubuzaki.WoooshGo.ui

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.tsubuzaki.WoooshGo.WoooshApplication
import com.tsubuzaki.WoooshGo.core.CoreEvent
import com.tsubuzaki.WoooshGo.core.TransferId
import com.tsubuzaki.WoooshGo.core.TrustedPeerInfo
import com.tsubuzaki.WoooshGo.pairing.PairingManager
import com.tsubuzaki.WoooshGo.peers.Peer
import com.tsubuzaki.WoooshGo.settings.RelayMode
import com.tsubuzaki.WoooshGo.settings.Settings
import com.tsubuzaki.WoooshGo.settings.Visibility
import com.tsubuzaki.WoooshGo.share.OutboxRepository
import com.tsubuzaki.WoooshGo.transfer.OutgoingOffer
import com.tsubuzaki.WoooshGo.transfer.TransferUi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.flow.shareIn
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.plus

class MainViewModel(application: Application) : AndroidViewModel(application) {

    private val app = application as WoooshApplication

    val peers: StateFlow<List<Peer>> = app.discovery.registry.peers

    val settings: StateFlow<Settings?> = app.settingsRepository.settings
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /** The core boots asynchronously, so poll until it answers (PROTOCOL.md §2). */
    val deviceIdFormatted: StateFlow<String?> = flow {
        while (true) {
            val id = app.core.deviceId()
            if (id != null) {
                emit(id)
                return@flow
            }
            delay(200)
        }
    }.stateIn(viewModelScope + Dispatchers.IO, SharingStarted.Eagerly, null)

    val fingerprintPhrase: StateFlow<String?> = flow {
        while (true) {
            val phrase = app.core.fingerprintPhrase()
            if (phrase != null) {
                emit(phrase)
                return@flow
            }
            delay(200)
        }
    }.stateIn(viewModelScope + Dispatchers.IO, SharingStarted.Eagerly, null)

    val transfers: StateFlow<List<TransferUi>> = app.transferManager.transfers
    val hasActiveTransfers: StateFlow<Boolean> = app.transferManager.hasActiveTransfers
    val pendingOffer: StateFlow<CoreEvent.IncomingOffer?> = app.transferManager.pendingOffer

    val outgoingOffers: StateFlow<List<OutgoingOffer>> = app.transferManager.outgoingOffers

    val pendingSas: StateFlow<PairingManager.SasRequest?> = app.pairingManager.pendingSas
    val keyChanged: StateFlow<PairingManager.KeyChangedAlert?> = app.pairingManager.keyChanged

    val pairingAttempt: StateFlow<PairingManager.Attempt?> = app.pairingManager.attempt

    /** Fires for a ticket *this* device redeemed, not one it published (PROTOCOL.md §9.4). */
    val ticketRedeemed: SharedFlow<Unit> = app.pairingManager.ticketRedeemed

    val statusMessages: SharedFlow<String> = merge(
        app.pairingManager.messages,
        app.transferManager.errors,
    ).shareIn(viewModelScope, SharingStarted.Eagerly)

    // The core's own pinned set, never a shell-side mirror.
    val pairedDevices: StateFlow<List<TrustedPeerInfo>> = app.trustStore.devices
    val pairedDeviceIds: StateFlow<Set<String>> = app.trustStore.devices
        .map { devices -> devices.map { it.deviceId }.toSet() }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptySet())

    val stagedShare: StateFlow<OutboxRepository.StagedShare?> = app.outbox.staged

    fun refresh() = app.discovery.refresh()

    fun setDisplayName(name: String) {
        viewModelScope.launch { app.settingsRepository.setDisplayName(name) }
    }

    fun setVisibility(visibility: Visibility) {
        viewModelScope.launch { app.settingsRepository.setVisibility(visibility) }
    }

    fun setRelayMode(mode: RelayMode) {
        viewModelScope.launch { app.settingsRepository.setRelayMode(mode) }
    }

    fun setRelayUrl(url: String) {
        viewModelScope.launch { app.settingsRepository.setRelayUrl(url) }
    }

    /** Set when the core refused the relay address; it keeps its last working config. */
    val relayError: StateFlow<String?> = app.relayError

    fun sendToPeer(peer: Peer, uris: List<Uri>) {
        app.transferManager.sendToPeer(peer, uris)
    }

    fun sendStaged(peer: Peer) {
        val staged = app.outbox.staged.value ?: return
        app.outbox.clear()
        app.transferManager.sendToPeer(peer, staged.uris)
    }

    fun dismissStaged() = app.outbox.clear()

    fun acceptOffer() {
        val offer = pendingOffer.value ?: return
        app.transferManager.acceptOffer(offer.manifest.map { it.id })
    }

    fun declineOffer() = app.transferManager.declineOffer()

    fun cancelTransfer(id: TransferId) = app.transferManager.cancel(id)

    fun dismissTransfer(id: TransferId) = app.transferManager.dismiss(id)

    fun confirmSas(accepted: Boolean) = app.pairingManager.confirmSas(accepted)

    fun dismissKeyChanged() = app.pairingManager.dismissKeyChanged()

    fun revokeForRepair() = app.pairingManager.revokeForRepair()

    fun revokeDevice(deviceId: String) = app.pairingManager.revoke(deviceId)

    fun beginPairingQr(): String = app.core.beginPairingQr()

    /** Via the manager, not the core: it owns the in-progress state and the timeout. */
    fun pairWithQr(payload: String) = app.pairingManager.pairWithScannedCode(payload)

    /** Throws rather than swallowing: a slow relay must not look like a broken one. */
    suspend fun beginInternetTicket(): String {
        // A redemption of the *previous* code must not fire this one's send.
        app.transferManager.clearTicketRedeemedPeer()
        return app.core.beginInternetTicket()
    }

    fun endInternetTicket() {
        app.core.endInternetTicket()
        app.transferManager.clearTicketRedeemedPeer()
        internetOutbox = emptyList()
    }

    private var internetOutbox: List<Uri> = emptyList()

    fun stageInternetSend(uris: List<Uri>) {
        internetOutbox = uris
    }

    val ticketRedeemedPeerId: StateFlow<String?> = app.transferManager.ticketRedeemedPeerId

    /** Clearing the id stops a redemption *this* device made replaying as an outgoing send. */
    fun ticketRedemptionHandled() = app.transferManager.clearTicketRedeemedPeer()

    /** The core refuses unless that peer really redeemed, so a stray call cannot leak files. */
    fun completeInternetSend(peerId: String) {
        val uris = internetOutbox
        if (uris.isEmpty()) return
        internetOutbox = emptyList()
        // A ticket is single-use: the id must not survive to trigger a second send.
        app.transferManager.clearTicketRedeemedPeer()
        app.transferManager.sendToPeerId(peerId, uris)
    }

    fun cancelPairingAttempt() = app.pairingManager.cancelAttempt()

    fun dismissPairingAttempt() = app.pairingManager.dismissAttempt()
}
