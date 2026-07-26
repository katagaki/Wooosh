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

/**
 * App-scoped holder for pairing UI state, fed by the core event stream so SAS requests
 * and KEY_CHANGED alerts are not lost while no screen is composed.
 *
 * The core emits a single `PairingResult` for both the QR and SAS paths (DESIGN.md §4).
 * The shell never records a pairing itself: it re-reads `trustedPeers()`, which the core
 * has already written by the time the event fires.
 */
class PairingManager(
    private val context: Context,
    private val scope: CoroutineScope,
    private val core: WoooshCore,
    private val trustStore: TrustStore,
    /**
     * Whether the internet path is switched on (DESIGN.md §9.1). A lambda rather than a
     * stored value: the setting changes while this manager lives, and reading it at the
     * moment of use is the only way to be right.
     */
    private val internetEnabled: () -> Boolean = { true },
) {

    data class SasRequest(val peer: PeerRef, val code: String)

    enum class AttemptState { CONNECTING, FAILED, SUCCEEDED }

    /**
     * A pairing ceremony the user is waiting on.
     *
     * Pairing dials each QR address hint in turn and then waits for PAIR_ACCEPT, so tens
     * of seconds is normal, not an anomaly — and silence for that long is indistinguishable
     * from pairing being broken. Every path that starts a ceremony must publish
     * [AttemptState.CONNECTING], and every path that ends one must settle it.
     */
    data class Attempt(
        val deviceName: String,
        val state: AttemptState,
        val message: String? = null,
        /**
         * Whether this is a ticket redemption rather than a pairing ceremony. The
         * internet path never pairs (PROTOCOL.md §9.4), so its wait must not be
         * labelled "Pairing" or resolved as "Paired with" — nothing is pinned.
         */
        val isTicket: Boolean = false,
    )

    /** A pinned peer presenting a different key: both phrases, so the user can compare. */
    data class KeyChangedAlert(
        val peer: PeerRef,
        val expectedFingerprint: String,
        val presentedFingerprint: String?,
    )

    private val _pendingSas = MutableStateFlow<SasRequest?>(null)
    val pendingSas: StateFlow<SasRequest?> = _pendingSas.asStateFlow()

    /** Non-null while a KEY_CHANGED warning must be shown (PROTOCOL.md §4.5). */
    private val _keyChanged = MutableStateFlow<KeyChangedAlert?>(null)
    val keyChanged: StateFlow<KeyChangedAlert?> = _keyChanged.asStateFlow()

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    /** Non-null while a pairing ceremony is running, or has just ended. */
    private val _attempt = MutableStateFlow<Attempt?>(null)
    val attempt: StateFlow<Attempt?> = _attempt.asStateFlow()

    /**
     * Whether the in-flight attempt is a ticket redemption. Decides which event settles
     * it: the ticket path succeeds with `TicketRedeemed`, never `PairingResult`.
     */
    @Volatile
    private var attemptIsTicket = false

    /** Client-side deadline for the current attempt — the "no event ever arrives" net. */
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
                            // The core pinned it before emitting; re-read rather than guess.
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
                            // Only a ceremony this device started gets the modal. The
                            // QR-*showing* side asked for no dialog, just a snackbar.
                            if (_attempt.value != null) {
                                settle(AttemptState.FAILED, name, message)
                            } else {
                                _messages.emit(message)
                            }
                        }
                    }

                    // The internet path never pairs (PROTOCOL.md §9.4), so a redeemed
                    // ticket succeeds with this event and no `PairingResult` ever
                    // arrives. Without settling here the ceremony would hang until the
                    // watchdog reported a timeout for a transfer that is running fine.
                    //
                    // Nothing is pinned, so there is no "Paired with" outcome to show.
                    // Redeeming is itself the consent (DESIGN.md §9.1) and the transfer
                    // UI takes over from here, so the wait simply ends.
                    is CoreEvent.TicketRedeemed -> if (attemptIsTicket) {
                        attemptIsTicket = false
                        watchdog?.cancel()
                        _attempt.value = null
                    }

                    is CoreEvent.KeyChanged -> _keyChanged.value = KeyChangedAlert(
                        peer = event.peer,
                        expectedFingerprint = event.peer.fingerprint,
                        presentedFingerprint = event.presentedFingerprint,
                    )

                    // Re-authentication updates last_seen in the core's store; re-read it
                    // so the Settings list is not stale.
                    is CoreEvent.PeerConnected -> if (event.peer.paired) trustStore.refresh()

                    else -> Unit
                }
            }
        }
    }

    // ------------------------------------------------------------- starting a ceremony

    /**
     * QR path (PROTOCOL.md §4.2). Malformed and expired codes are rejected locally: they
     * need no round trip, so do not make the user wait to be told the code was stale.
     *
     * `core.pairWithQr` returns at once and blocks on an IO dispatcher, so nothing here
     * touches the main thread.
     */
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

    /**
     * One entry point for every code the user can scan or paste. A pairing code and an
     * internet ticket look identical to a camera, so the scheme decides which path runs
     * rather than asking the user to classify a code they did not author.
     */
    fun pairWithScannedCode(payload: String) {
        if (!payload.trim().startsWith(TICKET_SCHEME)) {
            pairWithQr(payload)
            return
        }
        // A ticket scanned while the internet path is off: say so rather than dial. The
        // user is holding a code that would work if they turned it on, which a generic
        // pairing failure would not tell them.
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

    /**
     * Internet path, sender side (PROTOCOL.md §9). Same shape as [pairWithQr]: reject a
     * stale or malformed ticket locally, then hand the rest to the core and wait on its
     * `PairingResult`.
     */
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
        // "Codes match" is not the end: PAIR_CONFIRM still crosses the network both ways.
        // Leaving the SAS sheet up would let its own 60 s timer abort what was just
        // confirmed.
        if (accepted) beginAttempt(request.peer.displayName)
    }

    /** Camera-less pairing with a peer we are already connected to (PROTOCOL.md §4.3). */
    fun requestSasPairing(peerId: String) {
        beginAttempt(peerId)
        core.requestSasPairing(peerId)
    }

    /**
     * User gave up on the wait. The blocking core call cannot be interrupted, so this
     * stops the UI waiting rather than aborting the handshake; a late `PairingResult` is
     * then ignored (the snackbar still reports it).
     */
    fun cancelAttempt() {
        val abandoned = _attempt.value ?: return
        Log.i(TAG, "pairing attempt with ${abandoned.deviceName} cancelled by the user")
        attemptIsTicket = false
        watchdog?.cancel()
        _attempt.value = null
    }

    /** Dismisses a settled (failed or succeeded) attempt. */
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
            // Only fires when the core produced neither a result nor an error.
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

    /**
     * Terminal state for an attempt that is still on screen. Deliberately a no-op when
     * there is none: a `PairingResult` also reaches the device that merely *displayed*
     * the QR, and that user asked for no dialog — the snackbar is their feedback.
     */
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

    /** Raises a failed attempt with no prior CONNECTING state (pre-flight rejections). */
    private fun failNow(deviceName: String, message: String, isTicket: Boolean = false) {
        watchdog?.cancel()
        _attempt.value = Attempt(deviceName, AttemptState.FAILED, message, isTicket = isTicket)
    }

    fun dismissKeyChanged() {
        _keyChanged.value = null
    }

    /** "Re-pair" from the KEY_CHANGED dialog: drop the stale pin; caller navigates to pairing. */
    fun revokeForRepair() {
        val alert = _keyChanged.value ?: return
        revoke(alert.peer.id)
        _keyChanged.value = null
    }

    /** Revokes in the core — the only trust store there is — then re-reads the list. */
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

        /**
         * Shell-side backstop only; the core normally reports failure by throwing out of
         * `pairWithQr`. Must stay above the core's worst case (address hints dialled in
         * turn at ~10 s each, then a 20 s wait for PAIR_ACCEPT) so it never turns a slow
         * but working handshake into a false failure.
         */
        const val ATTEMPT_TIMEOUT_MS = 45_000L

        /**
         * The internet path gets its own, longer ceiling: redeeming a ticket can spend
         * ~30 s hole punching before the 20 s wait for PAIR_ACCEPT even starts, so the
         * LAN budget would report a working connection as a failure.
         */
        const val TICKET_TIMEOUT_MS = 75_000L

        const val TICKET_SCHEME = "wooosh-net:"
    }
}
