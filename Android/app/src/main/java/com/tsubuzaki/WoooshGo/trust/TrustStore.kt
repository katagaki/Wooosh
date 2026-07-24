package com.tsubuzaki.WoooshGo.trust

import android.util.Log
import com.tsubuzaki.WoooshGo.core.TrustedPeerInfo
import com.tsubuzaki.WoooshGo.core.WoooshCore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * The shell's view of the trust list (PROTOCOL.md §4.5) — a *cache* of
 * `WoooshCore.trustedPeers()`, never a second copy of the truth. Never mirror pairings
 * into local storage: a mirror can disagree with the core's trust.json in both directions.
 * This holds the last snapshot and re-reads at launch, after a pairing and after a revoke.
 *
 * Everything here is keyed by DeviceID ([TrustedPeerInfo.deviceId]) — the same string the
 * core puts in `peerId` on every event. Display names are labels, never identity.
 */
class TrustStore(
    private val scope: CoroutineScope,
    private val core: WoooshCore,
) {

    private val _devices = MutableStateFlow<List<TrustedPeerInfo>>(emptyList())

    /** Last snapshot read from the core, ordered by pairing time then DeviceID. */
    val devices: StateFlow<List<TrustedPeerInfo>> = _devices.asStateFlow()

    private val refreshLock = Mutex()

    /** Fire-and-forget re-read; safe to call from any thread. */
    fun refresh() {
        scope.launch { refreshNow() }
    }

    /** Re-reads the core's trust store and publishes the snapshot. */
    suspend fun refreshNow(): List<TrustedPeerInfo> = refreshLock.withLock {
        val peers = core.trustedPeers()
        _devices.value = peers
        Log.i(TAG, "trustedPeers(): ${peers.size} pinned ${peers.map { it.deviceId }}")
        peers
    }

    /** Latest snapshot for [deviceId], or null when that device is not pinned. */
    fun find(deviceId: String?): TrustedPeerInfo? =
        deviceId?.let { id -> _devices.value.firstOrNull { it.deviceId == id } }

    /**
     * The pinned key for [deviceId] — what `connectPeer` should be given so the very
     * first reconnect to a new address is pinned too.
     */
    fun pinnedKeyFor(deviceId: String?): ByteArray? = find(deviceId)?.publicKey

    private companion object {
        const val TAG = "WoooshTrust"
    }
}
