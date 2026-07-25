package com.tsubuzaki.WoooshGo.peers

import android.os.SystemClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update

/**
 * Canonical peer list (DESIGN.md §5 / PROTOCOL.md §3.3).
 *
 * Ordering contract: rows are ordered strictly by the discoveredAt of their FIRST
 * sighting in this process. The list is append-only and is NEVER re-sorted — lost
 * peers are grayed out in place, never removed. It clears only on process restart
 * or an explicit user refresh.
 */
class PeerRegistry(private val scope: CoroutineScope) {

    private val _peers = MutableStateFlow<List<Peer>>(emptyList())
    val peers: StateFlow<List<Peer>> = _peers

    private val staleJobs = HashMap<String, Job>()

    /** Last HELLO seen per DeviceID, applied to a row as soon as it learns its peer id. */
    private data class Hello(val displayName: String, val deviceType: DeviceType?)

    private val helloByPeerId = HashMap<String, Hello>()

    @Synchronized
    fun onSighting(
        rid: String,
        displayName: String,
        deviceType: DeviceType,
        port: Int,
        hosts: List<String>,
    ) {
        // Any sighting cancels a pending stale transition (grace timer restart).
        staleJobs.remove(rid)?.cancel()
        _peers.update { list ->
            val index = list.indexOfFirst { it.rid == rid }
            if (index >= 0) {
                // Re-enable in place; discoveredAt (the ordering key) is preserved.
                list.toMutableList().also {
                    it[index] = it[index].copy(
                        displayName = displayName,
                        deviceType = deviceType,
                        port = port,
                        // A TXT-only update (API 34+ delivers those) carries no
                        // addresses; keep the ones we already resolved.
                        hosts = hosts.ifEmpty { it[index].hosts },
                        isStale = false,
                    )
                }
            } else {
                list + Peer(
                    rid = rid,
                    displayName = displayName,
                    deviceType = deviceType,
                    port = port,
                    hosts = hosts,
                    discoveredAt = SystemClock.elapsedRealtime(),
                    isStale = false,
                )
            }
        }
    }

    /**
     * Records the core's peer id for a row after a successful connect, so a later send
     * to the same row can pass the pinned key to `connect_peer`.
     */
    @Synchronized
    fun attachPeerId(rid: String, peerId: String) {
        val hello = helloByPeerId[peerId]
        _peers.update { list ->
            list.map {
                if (it.rid != rid) {
                    it
                } else {
                    it.copy(
                        peerId = peerId,
                        // The HELLO for this peer usually lands before connect_peer()
                        // returns, i.e. before the row knew its DeviceID.
                        displayName = hello?.displayName?.ifBlank { null } ?: it.displayName,
                        deviceType = hello?.deviceType ?: it.deviceType,
                    )
                }
            }
        }
    }

    /**
     * A control channel came up (HELLO, PROTOCOL.md §4.1). The display name in HELLO is
     * authoritative — the mDNS TXT is an unauthenticated hint — and rows tied to this
     * DeviceID adopt it.
     *
     * The device type is adopted only when it is a known platform: the core's HELLO type
     * arrives as [DeviceType.UNKNOWN], and letting that win would replace a correct
     * `android-phone` glyph from the TXT record with a neutral one.
     */
    @Synchronized
    fun onConnected(peerId: String, displayName: String, deviceType: DeviceType?) {
        val known = deviceType?.takeIf { it != DeviceType.UNKNOWN }
        helloByPeerId[peerId] = Hello(displayName, known)
        _peers.update { list ->
            // Already ours: just refresh it in place.
            if (list.any { it.peerId == peerId }) {
                return@update list.map { peer ->
                    if (peer.peerId != peerId) {
                        peer
                    } else {
                        peer.copy(
                            displayName = displayName.ifBlank { peer.displayName },
                            deviceType = known ?: peer.deviceType,
                            isStale = false,
                        )
                    }
                }
            }
            // A discovered row that has not learned its DeviceID yet: adopt it, so a
            // peer found over mDNS and then connected does not show up twice. HELLO
            // usually beats connect_peer() returning, so this is the common case.
            val adoptable = list.indexOfFirst {
                it.peerId == null && displayName.isNotBlank() && it.displayName == displayName
            }
            if (adoptable >= 0) {
                return@update list.toMutableList().also {
                    it[adoptable] = it[adoptable].copy(
                        peerId = peerId,
                        deviceType = known ?: it[adoptable].deviceType,
                        isStale = false,
                    )
                }
            }
            // Nothing matches: a peer the browser never saw. This is how a device
            // reached over the internet (PROTOCOL.md §9) enters the list at all —
            // there is no mDNS record behind it, and without a row the user has
            // nothing to tap. Appended like every other row (DESIGN.md §5).
            list + Peer(
                rid = "$CORE_RID_PREFIX$peerId",
                displayName = displayName,
                // No TXT record was ever seen, so the glyph stays neutral rather than
                // being guessed from the coarse HELLO type.
                deviceType = known ?: DeviceType.UNKNOWN,
                port = 0,
                hosts = emptyList(),
                discoveredAt = SystemClock.elapsedRealtime(),
                isStale = false,
                peerId = peerId,
            )
        }
    }

    /**
     * The core dropped a connection. A row that only ever existed *because* of that
     * connection has no address to reconnect with, so it grays out in place rather than
     * disappearing — rows never move or vanish (DESIGN.md §5).
     */
    @Synchronized
    fun onDisconnected(peerId: String) {
        _peers.update { list ->
            list.map { peer ->
                if (peer.peerId == peerId && peer.rid.startsWith(CORE_RID_PREFIX)) {
                    peer.copy(isStale = true)
                } else {
                    peer
                }
            }
        }
    }

    /**
     * NsdManager emits onServiceLost rather than periodic announces, so a loss event
     * starts the 10 s grace period; a re-sighting within it cancels the transition
     * (avoids flicker from transient mDNS churn).
     */
    @Synchronized
    fun onLost(rid: String) {
        if (_peers.value.none { it.rid == rid }) return
        staleJobs.remove(rid)?.cancel()
        staleJobs[rid] = scope.launch {
            delay(STALE_GRACE_MS)
            markStale(rid)
        }
    }

    @Synchronized
    private fun markStale(rid: String) {
        staleJobs.remove(rid)
        _peers.update { list ->
            list.map { if (it.rid == rid) it.copy(isStale = true) else it }
        }
    }

    @Synchronized
    fun clear() {
        staleJobs.values.forEach { it.cancel() }
        staleJobs.clear()
        helloByPeerId.clear()
        _peers.value = emptyList()
    }

    private companion object {
        const val STALE_GRACE_MS = 10_000L

        /** Marks a row that exists only because of a core connection, not a sighting. */
        const val CORE_RID_PREFIX = "core:"
    }
}
