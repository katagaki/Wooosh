package com.tsubuzaki.WoooshGo.pairing

import android.content.Context
import android.util.Log
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.core.CoreEvent
import com.tsubuzaki.WoooshGo.core.PeerRef
import com.tsubuzaki.WoooshGo.core.WoooshCore
import com.tsubuzaki.WoooshGo.core.pairingErrorMessage
import com.tsubuzaki.WoooshGo.trust.TrustStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** App-scoped so SAS and KEY_CHANGED alerts survive with no screen composed. */
class PairingManager(
    private val context: Context,
    private val scope: CoroutineScope,
    private val core: WoooshCore,
    private val trustStore: TrustStore,
    /** A lambda, not a stored value: the setting changes while this manager lives. */
    private val internetEnabled: () -> Boolean = { true },
) {

    data class SasRequest(val peer: PeerRef, val code: String)

    enum class AttemptState { CONNECTING, FAILED, SUCCEEDED }

    /**
     * Tens of seconds of silence is normal and reads as breakage, so every path that
     * starts a ceremony must publish CONNECTING and every path that ends one must settle it.
     */
    data class Attempt(
        val deviceName: String,
        val state: AttemptState,
        val message: String? = null,
        /** The internet path never pairs (PROTOCOL.md §9.4), so its wait is worded differently. */
        val isTicket: Boolean = false,
    )

    data class KeyChangedAlert(
        val peer: PeerRef,
        val expectedFingerprint: String,
        val presentedFingerprint: String?,
    )

    private val _pendingSas = MutableStateFlow<SasRequest?>(null)
    val pendingSas: StateFlow<SasRequest?> = _pendingSas.asStateFlow()

    private val _keyChanged = MutableStateFlow<KeyChangedAlert?>(null)
    val keyChanged: StateFlow<KeyChangedAlert?> = _keyChanged.asStateFlow()

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    private val _attempt = MutableStateFlow<Attempt?>(null)
    val attempt: StateFlow<Attempt?> = _attempt.asStateFlow()

    /** Fires only on the redeeming device; scanning was the whole job (PROTOCOL.md §9.4). */
    private val _ticketRedeemed = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    val ticketRedeemed: SharedFlow<Unit> = _ticketRedeemed.asSharedFlow()

    /** The ticket path settles on `TicketRedeemed`, never `PairingResult`. */
    @Volatile
    private var attemptIsTicket = false

    private var watchdog: Job? = null

    init {
        scope.launch {
            core.events.collect { event ->
                when (event) {
                    is CoreEvent.PairingSas ->
                        _pendingSas.value = SasRequest(event.peer, event.sixDigits)

                    is CoreEvent.PairingResult -> {
                        _pendingSas.value = null
                        watchdog?.cancel()
                        if (event.success) {
                            attemptIsTicket = false
                            // The core pinned it before emitting; re-read, do not guess.
                            val peers = trustStore.refreshNow()
                            val name = peers.firstOrNull { it.deviceId == event.peerId }
                                ?.displayName
                                ?: event.peer?.displayName
                                ?: event.message
                                ?: event.peerId
                            settle(AttemptState.SUCCEEDED, name, null)
                            _messages.emit(
                                context.getString(R.string.pairing_success_title, name)
                            )
                        } else {
                            // The core's message is an internal token, not copy: map it.
                            val message = pairingErrorMessage(context, event.message)
                            val name = event.peer?.displayName
                                ?: _attempt.value?.deviceName
                                ?: event.peerId
                            // Only a ceremony this device started gets the modal.
                            if (_attempt.value != null) {
                                settle(AttemptState.FAILED, name, message)
                            } else {
                                _messages.emit(message)
                            }
                        }
                    }

                    // No `PairingResult` arrives: the internet path never pairs.
                    is CoreEvent.TicketRedeemed -> if (attemptIsTicket) {
                        attemptIsTicket = false
                        watchdog?.cancel()
                        _attempt.value = null
                        // Guarded: the publisher gets this event too but must stay put.
                        _ticketRedeemed.emit(Unit)
                    }

                    is CoreEvent.KeyChanged -> _keyChanged.value = KeyChangedAlert(
                        peer = event.peer,
                        expectedFingerprint = event.peer.fingerprint,
                        presentedFingerprint = event.presentedFingerprint,
                    )

                    // Re-authentication updates last_seen in the core's store.
                    is CoreEvent.PeerConnected -> if (event.peer.paired) trustStore.refresh()

                    else -> Unit
                }
            }
        }
    }

    /** Stale and malformed codes are rejected locally rather than after a round trip. */
    fun pairWithQr(payload: String) {
        val info = core.parsePairingCode(payload)
        val name = info?.deviceName?.takeIf { it.isNotBlank() }
            ?: info?.deviceId
            ?: context.getString(R.string.peer_unnamed)
        when {
            info == null -> {
                Log.w(TAG, "pairWithQr: payload is not a Wooosh pairing code")
                failNow(name, context.getString(R.string.error_pairing_not_a_code))
            }

            info.expired -> {
                Log.w(TAG, "pairWithQr: code for ${info.deviceId} already expired")
                failNow(name, context.getString(R.string.error_pairing_code_expired_detail))
            }

            else -> {
                Log.i(TAG, "pairWithQr: connecting to ${info.deviceId} hints=${info.hints}")
                beginAttempt(name)
                core.pairWithQr(payload)
            }
        }
    }

    /** A pairing code and a ticket look identical to a camera, so the scheme decides. */
    fun pairWithScannedCode(payload: String) {
        if (!payload.trim().startsWith(TICKET_SCHEME)) {
            pairWithQr(payload)
            return
        }
        // A generic pairing failure would not tell the user the code works once enabled.
        if (!internetEnabled()) {
            failNow(
                context.getString(R.string.peer_unnamed),
                context.getString(R.string.error_internet_off),
                isTicket = true,
            )
            return
        }
        redeemTicket(payload)
    }

    /** Internet path, sender side (PROTOCOL.md §9). */
    fun redeemTicket(payload: String) {
        val info = core.parseTicket(payload)
        val name = info?.deviceName?.takeIf { it.isNotBlank() }
            ?: info?.deviceId
            ?: context.getString(R.string.peer_unnamed)
        when {
            info == null -> {
                Log.w(TAG, "redeemTicket: payload is not a Wooosh internet code")
                failNow(name, context.getString(R.string.error_pairing_not_a_code), isTicket = true)
            }

            info.expired -> {
                Log.w(TAG, "redeemTicket: ticket for ${info.deviceId} already expired")
                failNow(
                    name,
                    context.getString(R.string.error_pairing_code_expired_detail),
                    isTicket = true,
                )
            }

            else -> {
                Log.i(TAG, "redeemTicket: dialling ${info.deviceId} relay=${info.relay}")
                beginAttempt(name, TICKET_TIMEOUT_MS, isTicket = true)
                core.redeemTicket(payload)
            }
        }
    }

    fun confirmSas(accepted: Boolean) {
        val request = _pendingSas.value ?: return
        core.confirmSas(request.peer.id, accepted)
        _pendingSas.value = null
        // PAIR_CONFIRM still crosses the network, and the SAS sheet's own 60 s timer
        // would abort what was just confirmed.
        if (accepted) beginAttempt(request.peer.displayName)
    }

    /** Camera-less pairing, requires an existing connection (PROTOCOL.md §4.3). */
    fun requestSasPairing(peerId: String) {
        beginAttempt(peerId)
        core.requestSasPairing(peerId)
    }

    /** The blocking core call cannot be interrupted; this only stops the UI waiting. */
    fun cancelAttempt() {
        val abandoned = _attempt.value ?: return
        Log.i(TAG, "pairing attempt with ${abandoned.deviceName} cancelled by the user")
        attemptIsTicket = false
        watchdog?.cancel()
        _attempt.value = null
    }

    fun dismissAttempt() {
        if (_attempt.value?.state != AttemptState.CONNECTING) _attempt.value = null
    }

    private fun beginAttempt(
        deviceName: String,
        timeoutMs: Long = ATTEMPT_TIMEOUT_MS,
        isTicket: Boolean = false,
    ) {
        attemptIsTicket = isTicket
        watchdog?.cancel()
        _attempt.value = Attempt(deviceName, AttemptState.CONNECTING, isTicket = isTicket)
        watchdog = scope.launch {
            delay(timeoutMs)
            if (_attempt.value?.state == AttemptState.CONNECTING) {
                Log.w(TAG, "pairing with $deviceName timed out in the shell after ${timeoutMs}ms")
                settle(
                    AttemptState.FAILED,
                    deviceName,
                    context.getString(
                        if (isTicket) R.string.error_internet_timeout
                        else R.string.error_pairing_timeout
                    ),
                )
            }
        }
    }

    /** A no-op with no attempt on screen: the QR-showing device gets a snackbar instead. */
    private fun settle(state: AttemptState, deviceName: String, message: String?) {
        val current = _attempt.value ?: return
        watchdog?.cancel()
        _attempt.value = Attempt(
            deviceName = current.deviceName.ifBlank { deviceName },
            state = state,
            message = message,
            isTicket = current.isTicket,
        )
    }

    private fun failNow(deviceName: String, message: String, isTicket: Boolean = false) {
        watchdog?.cancel()
        _attempt.value = Attempt(deviceName, AttemptState.FAILED, message, isTicket = isTicket)
    }

    fun dismissKeyChanged() {
        _keyChanged.value = null
    }

    fun revokeForRepair() {
        val alert = _keyChanged.value ?: return
        revoke(alert.peer.id)
        _keyChanged.value = null
    }

    /** The core is the only trust store there is. */
    fun revoke(deviceId: String) {
        scope.launch {
            val key = trustStore.pinnedKeyFor(deviceId)
            if (key == null) {
                Log.w(TAG, "revoke($deviceId): no pinned key in the core's trust store")
                trustStore.refreshNow()
                return@launch
            }
            val removed = core.revokePeer(key)
            Log.i(TAG, "revoke($deviceId) -> $removed")
            trustStore.refreshNow()
        }
    }

    private companion object {
        const val TAG = "WoooshPairing"

        /** Backstop; above the core's worst case of hints at ~10 s each plus a 20 s wait. */
        const val ATTEMPT_TIMEOUT_MS = 45_000L

        /** Redeeming can spend ~30 s hole punching before the PAIR_ACCEPT wait starts. */
        const val TICKET_TIMEOUT_MS = 75_000L

        const val TICKET_SCHEME = "wooosh-net:"
    }
}
