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
 * App-scoped holder for pairing UI state, fed by the core event stream so SAS
 * requests / KEY_CHANGED alerts are not lost while no screen is composed.
 *
 * The core emits a single `PairingResult` for both the QR and SAS paths (DESIGN.md §4),
 * now carrying the pinned key and its fingerprint. The shell no longer records the
 * pairing itself: it re-reads `trustedPeers()` from the core, which has already written
 * trust.json by the time the event is emitted.
 */
class PairingManager(
    private val context: Context,
    private val scope: CoroutineScope,
    private val core: WoooshCore,
    private val trustStore: TrustStore,
) {

    data class SasRequest(val peer: PeerRef, val code: String)

    enum class AttemptState { CONNECTING, FAILED, SUCCEEDED }

    /**
     * A pairing ceremony the user is waiting on.
     *
     * Pairing crosses a network: it dials each address hint in the QR in turn and then
     * waits for PAIR_ACCEPT, so tens of seconds is a normal outcome, not an anomaly.
     * Before this existed the UI showed nothing at all between the scan and the result —
     * one report had 19 s of silence end with the user force-quitting the app, which is
     * indistinguishable from pairing being broken. Every path that starts a ceremony now
     * publishes [AttemptState.CONNECTING] and every path that ends one settles it.
     */
    data class Attempt(
        val deviceName: String,
        val state: AttemptState,
        val message: String? = null,
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
                            // The core pinned it before emitting; re-read the real list
                            // instead of guessing what it now contains.
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
                            // The core's own message is an internal token, not
                            // copy: map it rather than showing it.
                            val message = pairingErrorMessage(context, event.message)
                            val name = event.peer?.displayName
                                ?: _attempt.value?.deviceName
                                ?: event.peerId
                            // A ceremony this device started gets the modal; an
                            // unsolicited failure (we were the QR-*showing* side) is
                            // still worth a snackbar but must not raise a dialog.
                            if (_attempt.value != null) {
                                settle(AttemptState.FAILED, name, message)
                            } else {
                                _messages.emit(message)
                            }
                        }
                    }

                    is CoreEvent.KeyChanged -> _keyChanged.value = KeyChangedAlert(
                        peer = event.peer,
                        expectedFingerprint = event.peer.fingerprint,
                        presentedFingerprint = event.presentedFingerprint,
                    )

                    // A pinned peer that just re-authenticated updates last_seen in the
                    // core's store; keep the Settings list from showing a stale value.
                    is CoreEvent.PeerConnected -> if (event.peer.paired) trustStore.refresh()

                    else -> Unit
                }
            }
        }
    }

    // ------------------------------------------------------------- starting a ceremony

    /**
     * QR path (PROTOCOL.md §4.2). Rejects a malformed or expired code immediately —
     * those need no network round trip and it is rude to make the user wait to be told
     * their code was stale — then hands off to the core and shows progress until the
     * outcome is known.
     *
     * `core.pairWithQr` returns at once and does its blocking work on an IO dispatcher,
     * so nothing here touches the main thread.
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

    fun confirmSas(accepted: Boolean) {
        val request = _pendingSas.value ?: return
        core.confirmSas(request.peer.id, accepted)
        _pendingSas.value = null
        // "Codes match" is not the end of the ceremony: PAIR_CONFIRM still has to cross
        // the network both ways. Show the wait instead of leaving the SAS sheet sitting
        // there — where, worse, its own 60 s timer would have aborted what the user just
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
     * stops the UI waiting rather than aborting the handshake — a late `PairingResult`
     * for an abandoned attempt is then ignored (the snackbar still reports it). The
     * point is that the user always has a way out that is not force-quitting the app.
     */
    fun cancelAttempt() {
        val abandoned = _attempt.value ?: return
        Log.i(TAG, "pairing attempt with ${abandoned.deviceName} cancelled by the user")
        watchdog?.cancel()
        _attempt.value = null
    }

    /** Dismisses a settled (failed or succeeded) attempt. */
    fun dismissAttempt() {
        if (_attempt.value?.state != AttemptState.CONNECTING) _attempt.value = null
    }

    private fun beginAttempt(deviceName: String) {
        watchdog?.cancel()
        _attempt.value = Attempt(deviceName, AttemptState.CONNECTING)
        watchdog = scope.launch {
            delay(ATTEMPT_TIMEOUT_MS)
            // Only fires when the core produced neither a result nor an error. That is
            // the failure mode this whole state exists for: silence.
            if (_attempt.value?.state == AttemptState.CONNECTING) {
                Log.w(TAG, "pairing with $deviceName timed out in the shell after ${ATTEMPT_TIMEOUT_MS}ms")
                settle(
                    AttemptState.FAILED,
                    deviceName,
                    context.getString(R.string.error_pairing_timeout),
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
        )
    }

    /** Raises a failed attempt with no prior CONNECTING state (pre-flight rejections). */
    private fun failNow(deviceName: String, message: String) {
        watchdog?.cancel()
        _attempt.value = Attempt(deviceName, AttemptState.FAILED, message)
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

    /**
     * Revokes in the core — the only trust store there is — then re-reads the list.
     * The key comes from `trustedPeers()`, so this now works for every paired device
     * instead of only the ones paired by scanning a QR.
     */
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
         * Shell-side deadline for a pairing attempt, and *only* a backstop: the core
         * normally reports failure well before this by throwing out of `pairWithQr`.
         * Sized above the core's own worst case (address hints dialled in turn at ~10 s
         * each, then a 20 s wait for PAIR_ACCEPT) so it never pre-empts a slow but
         * working handshake with a false failure. It exists purely so that "the core
         * emitted nothing at all" — the bug that made users force-quit — ends in a
         * message instead of an indefinite spinner.
         */
        const val ATTEMPT_TIMEOUT_MS = 45_000L
    }
}
