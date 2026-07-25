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

    /**
     * Identity comes from the core (PROTOCOL.md §2) — one keypair per install. The core
     * boots asynchronously, so poll briefly until it answers.
     */
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

    // ---- transfers ----
    val transfers: StateFlow<List<TransferUi>> = app.transferManager.transfers
    val hasActiveTransfers: StateFlow<Boolean> = app.transferManager.hasActiveTransfers
    val pendingOffer: StateFlow<CoreEvent.IncomingOffer?> = app.transferManager.pendingOffer

    /** Sends whose OFFER is out and whose receiver has not answered yet. */
    val outgoingOffers: StateFlow<List<OutgoingOffer>> = app.transferManager.outgoingOffers

    // ---- pairing ----
    val pendingSas: StateFlow<PairingManager.SasRequest?> = app.pairingManager.pendingSas
    val keyChanged: StateFlow<PairingManager.KeyChangedAlert?> = app.pairingManager.keyChanged

    /** Non-null while a pairing ceremony is running or has just resolved. */
    val pairingAttempt: StateFlow<PairingManager.Attempt?> = app.pairingManager.attempt

    /** Snackbar channel: pairing outcomes plus transfer failures with no card yet. */
    val statusMessages: SharedFlow<String> = merge(
        app.pairingManager.messages,
        app.transferManager.errors,
    ).shareIn(viewModelScope, SharingStarted.Eagerly)

    // ---- trust: the core's own pinned set, never a shell-side mirror ----
    val pairedDevices: StateFlow<List<TrustedPeerInfo>> = app.trustStore.devices
    val pairedDeviceIds: StateFlow<Set<String>> = app.trustStore.devices
        .map { devices -> devices.map { it.deviceId }.toSet() }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptySet())

    // ---- share target ----
    val stagedShare: StateFlow<OutboxRepository.StagedShare?> = app.outbox.staged

    // ------------------------------------------------------------- actions

    fun refresh() = app.discovery.refresh()

    fun setDisplayName(name: String) {
        viewModelScope.launch { app.settingsRepository.setDisplayName(name) }
    }

    fun setVisibility(visibility: Visibility) {
        viewModelScope.launch { app.settingsRepository.setVisibility(visibility) }
    }

    /** Relay selection for the internet path (DESIGN.md §9.1). */
    fun setRelayMode(mode: RelayMode) {
        viewModelScope.launch { app.settingsRepository.setRelayMode(mode) }
    }

    fun setRelayUrl(url: String) {
        viewModelScope.launch { app.settingsRepository.setRelayUrl(url) }
    }

    /**
     * Non-null when the core refused the relay address. The core keeps its previous
     * working configuration in that case, so this is a correction to make rather than a
     * broken state.
     */
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

    /** Drops the stale pin; the caller then navigates to the pairing screen. */
    fun revokeForRepair() = app.pairingManager.revokeForRepair()

    /** [deviceId] is a DeviceID from [pairedDevices]; the key is resolved from the core. */
    fun revokeDevice(deviceId: String) = app.pairingManager.revoke(deviceId)

    fun beginPairingQr(): String = app.core.beginPairingQr()

    /**
     * Goes through the pairing manager, not straight at the core: it owns the
     * in-progress state, the pre-flight checks and the timeout that keep this from
     * looking like a hang.
     */
    fun pairWithQr(payload: String) = app.pairingManager.pairWithScannedCode(payload)

    /**
     * Mints an internet ticket (PROTOCOL.md §9.2). Suspends for as long as it takes the
     * core to bind its endpoint and find a relay, and throws rather than swallowing: this
     * is the one call that contacts a relay, and a user who asked for a code and got
     * silence cannot tell a slow relay from a broken one.
     */
    suspend fun beginInternetTicket(): String = app.core.beginInternetTicket()

    fun endInternetTicket() {
        app.core.endInternetTicket()
        internetOutbox = emptyList()
    }

    /** Files waiting for someone to scan the code (PROTOCOL.md §9.4). */
    private var internetOutbox: List<Uri> = emptyList()

    fun stageInternetSend(uris: List<Uri>) {
        internetOutbox = uris
    }

    /** Set when someone redeems a ticket this device published. */
    val ticketRedeemedPeerId: StateFlow<String?> = app.transferManager.ticketRedeemedPeerId

    /**
     * Hands the staged files to whoever redeemed. The core refuses this unless that peer
     * really did redeem (PROTOCOL.md §9.4), so a stray call cannot leak them.
     */
    fun completeInternetSend(peerId: String) {
        val uris = internetOutbox
        if (uris.isEmpty()) return
        internetOutbox = emptyList()
        app.transferManager.sendToPeerId(peerId, uris)
    }

    /** DeviceID of a peer that just arrived by redeeming a ticket; consumed once. */
    val redeemedPeerId: StateFlow<String?> = app.pairingManager.redeemedPeerId

    fun clearRedeemedPeer() {
        app.pairingManager.takeRedeemedPeerId()
    }

    fun cancelPairingAttempt() = app.pairingManager.cancelAttempt()

    fun dismissPairingAttempt() = app.pairingManager.dismissAttempt()
}
