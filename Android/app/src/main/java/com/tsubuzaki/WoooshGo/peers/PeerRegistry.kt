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
 * Ordered by a row's FIRST sighting: append-only, never re-sorted, lost peers greyed out
 * in place rather than removed (DESIGN.md §5).
 */
class PeerRegistry(private val scope: CoroutineScope) {

    private val _peers = MutableStateFlow<List<Peer>>(emptyList())
    val peers: StateFlow<List<Peer>> = _peers

    private val staleJobs = HashMap<String, Job>()

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
        // Any sighting cancels a pending stale transition.
        staleJobs.remove(rid)?.cancel()
        _peers.update { list ->
            val index = list.indexOfFirst { it.rid == rid }
            if (index >= 0) {
                // In place: discoveredAt, the ordering key, is preserved.
                list.toMutableList().also {
                    it[index] = it[index].copy(
                        displayName = displayName,
                        deviceType = deviceType,
                        port = port,
                        // A TXT-only update (API 34+) carries no addresses.
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

    /** Lets a later send to the same row pass the pinned key to `connect_peer`. */
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
                        // HELLO usually lands before connect_peer() returns.
                        displayName = hello?.displayName?.ifBlank { null } ?: it.displayName,
                        deviceType = hello?.deviceType ?: it.deviceType,
                    )
                }
            }
        }
    }

    /**
     * The HELLO name is authoritative; the mDNS TXT is an unauthenticated hint. Device
     * type is adopted only when known, or the core's UNKNOWN overwrites a correct glyph.
     * [viaTicket] relies on `PeerConnected` arriving before `TicketRedeemed`.
     */
    @Synchronized
    fun onConnected(
        peerId: String,
        displayName: String,
        deviceType: DeviceType?,
        viaTicket: Boolean = false,
    ) {
        val known = deviceType?.takeIf { it != DeviceType.UNKNOWN }
        helloByPeerId[peerId] = Hello(displayName, known)
        _peers.update { list ->
            if (list.any { it.peerId == peerId }) {
                return@update list.map { peer ->
                    if (peer.peerId != peerId) {
                        peer
                    } else {
                        peer.copy(
                            displayName = displayName.ifBlank { peer.displayName },
                            deviceType = known ?: peer.deviceType,
                            isStale = false,
                            viaTicket = viaTicket,
                        )
                    }
                }
            }
            // Adopt a row without a DeviceID, or the peer would show up twice.
            val adoptable = list.indexOfFirst {
                it.peerId == null && displayName.isNotBlank() && it.displayName == displayName
            }
            if (adoptable >= 0) {
                return@update list.toMutableList().also {
                    it[adoptable] = it[adoptable].copy(
                        peerId = peerId,
                        deviceType = known ?: it[adoptable].deviceType,
                        isStale = false,
                        viaTicket = viaTicket,
                    )
                }
            }
            // How a device reached over the internet (PROTOCOL.md §9) enters the list.
            list + Peer(
                rid = "$CORE_RID_PREFIX$peerId",
                displayName = displayName,
                // No TXT record was ever seen, so the glyph stays neutral.
                deviceType = known ?: DeviceType.UNKNOWN,
                port = 0,
                hosts = emptyList(),
                discoveredAt = SystemClock.elapsedRealtime(),
                isStale = false,
                peerId = peerId,
                viaTicket = viaTicket,
            )
        }
    }

    /**
     * A connection-only row greys out in place while an mDNS-backed one stays live.
     * [Peer.viaTicket] stays set so the row cannot re-acquire a badge it never earned.
     */
    @Synchronized
    fun onTicketSessionEnded(peerId: String) {
        _peers.update { list ->
            list.map { peer ->
                if (peer.peerId == peerId &&
                    peer.viaTicket &&
                    peer.rid.startsWith(CORE_RID_PREFIX)
                ) {
                    peer.copy(isStale = true)
                } else {
                    peer
                }
            }
        }
    }

    /** A connection-only row has no address to retry, and rows never vanish (DESIGN.md §5). */
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

    /** NsdManager has no periodic announces, so a loss starts the 10 s grace period. */
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
