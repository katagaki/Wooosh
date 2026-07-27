package com.tsubuzaki.WoooshGo.peers

/**
 * The `dt` TXT value (PROTOCOL.md §3.1), platform-explicit because the receiving UI picks
 * a glyph and a wrong one is worse than a generic one. Unrecognised values map to [UNKNOWN].
 */
enum class DeviceType(val txtValue: String?) {
    IPHONE("iphone"),
    IPAD("ipad"),
    MAC("mac"),
    WINDOWS("windows"),
    ANDROID_PHONE("android-phone"),
    ANDROID_TABLET("android-tablet"),

    UNKNOWN(null);

    companion object {
        fun fromTxt(value: String?): DeviceType =
            entries.firstOrNull { it.txtValue != null && it.txtValue == value } ?: UNKNOWN
    }
}

data class Peer(
    /** Rotating discovery ID (PROTOCOL.md §3.1); the de-duplication key. */
    val rid: String,
    val displayName: String,
    val deviceType: DeviceType,
    val port: Int,
    /** IPv4 first; what `connect_peer` takes. */
    val hosts: List<String> = emptyList(),
    /** elapsedRealtime of the FIRST sighting — the permanent ordering key. */
    val discoveredAt: Long,
    val isStale: Boolean,
    /** Set once connected, so a repeat send can pin the key we hold. */
    val peerId: String? = null,
    /**
     * Authorised by a redeemed ticket (PROTOCOL.md §9.4), not a pinned key, so the row
     * must never wear the paired badge. Cleared on `PeerConnected`, not on disconnect.
     */
    val viaTicket: Boolean = false,
) {
    /** Null when nothing has resolved yet. */
    val address: String?
        get() = hosts.firstOrNull()?.let { host ->
            if (host.contains(':') && !host.startsWith("[")) "[$host]:$port" else "$host:$port"
        }
}
