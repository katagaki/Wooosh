package com.tsubuzaki.WoooshGo.peers

/**
 * Advertised device type — the `dt` TXT value of PROTOCOL.md §3.1.
 *
 * The vocabulary is deliberately **platform-explicit** rather than form-factor-only:
 * form factor cannot tell a Pixel from an iPhone, and the receiving UI has to pick a
 * glyph. A generic icon is always acceptable; a confidently wrong one is not.
 *
 * Anything this build does not recognise maps to [UNKNOWN] and renders neutrally. That
 * covers both the pre-vocabulary values (`phone` / `tablet` / `laptop` / `desktop`) that
 * older Wooosh builds on other platforms are still advertising while they are updated,
 * and any value the spec grows later.
 */
enum class DeviceType(val txtValue: String?) {
    IPHONE("iphone"),
    IPAD("ipad"),
    MAC("mac"),
    WINDOWS("windows"),
    ANDROID_PHONE("android-phone"),
    ANDROID_TABLET("android-tablet"),

    /** Absent `dt`, or a value this build does not know. Never guessed at. */
    UNKNOWN(null);

    companion object {
        fun fromTxt(value: String?): DeviceType =
            entries.firstOrNull { it.txtValue != null && it.txtValue == value } ?: UNKNOWN
    }
}

data class Peer(
    /** Rotating discovery ID (PROTOCOL.md §3.1) — the de-duplication key. */
    val rid: String,
    val displayName: String,
    val deviceType: DeviceType,
    val port: Int,
    /**
     * Addresses mDNS resolved for this instance, IPv4 first. Milestone 1 dropped these
     * (only rid/port were kept), which left nothing to hand `connect_peer` — they are
     * captured now from `ServiceInfoCallback.hostAddresses` (API 34+) or the legacy
     * `NsdServiceInfo.host`.
     */
    val hosts: List<String> = emptyList(),
    /** SystemClock.elapsedRealtime() of the FIRST sighting — the permanent ordering key. */
    val discoveredAt: Long,
    val isStale: Boolean,
    /**
     * The core's peer id (= the peer's DeviceID) once a connection to this row has been
     * established in this session. Lets a repeat send pin the pubkey we hold for it.
     */
    val peerId: String? = null,
) {
    /** "host:port" for `connect_peer`, or null when nothing resolved yet. */
    val address: String?
        get() = hosts.firstOrNull()?.let { host ->
            if (host.contains(':') && !host.startsWith("[")) "[$host]:$port" else "$host:$port"
        }
}
