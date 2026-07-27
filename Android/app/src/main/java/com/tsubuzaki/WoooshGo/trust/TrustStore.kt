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
 * A cache of `WoooshCore.trustedPeers()` (PROTOCOL.md §4.5), never a second copy of the
 * truth: a mirror can disagree with the core's trust.json in both directions.
 */
class TrustStore(
    private val scope: CoroutineScope,
    private val core: WoooshCore,
) {

    private val _devices = MutableStateFlow<List<TrustedPeerInfo>>(emptyList())

    val devices: StateFlow<List<TrustedPeerInfo>> = _devices.asStateFlow()

    private val refreshLock = Mutex()

    fun refresh() {
        scope.launch { refreshNow() }
    }

    suspend fun refreshNow(): List<TrustedPeerInfo> = refreshLock.withLock {
        val peers = core.trustedPeers()
        _devices.value = peers
        Log.i(TAG, "trustedPeers(): ${peers.size} pinned ${peers.map { it.deviceId }}")
        peers
    }

    fun find(deviceId: String?): TrustedPeerInfo? =
        deviceId?.let { id -> _devices.value.firstOrNull { it.deviceId == id } }

    /** Pass to `connectPeer` so the first reconnect to a new address is pinned too. */
    fun pinnedKeyFor(deviceId: String?): ByteArray? = find(deviceId)?.publicKey

    private companion object {
        const val TAG = "WoooshTrust"
    }
}
