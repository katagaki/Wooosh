package com.tsubuzaki.WoooshGo.discovery

import android.content.Context
import android.net.nsd.NsdManager
import com.tsubuzaki.WoooshGo.peers.DeviceType
import com.tsubuzaki.WoooshGo.peers.PeerRegistry
import com.tsubuzaki.WoooshGo.settings.SettingsRepository
import java.security.SecureRandom
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

/**
 * Wires settings -> advertiser and browser -> peer registry.
 *
 * [listenPort] is the core's bound QUIC port (`core.listenAddr()`), published verbatim in
 * the TXT `p` field (DESIGN.md §4); discovery does not start until it is non-zero.
 */
class DiscoveryController(
    context: Context,
    private val scope: CoroutineScope,
    private val settingsRepository: SettingsRepository,
    private val listenPort: () -> Int,
) {

    /** Rotating discovery ID: 8 random bytes, lowercase hex, new on every process start (PROTOCOL.md §3.1). */
    val rid: String = ByteArray(8)
        .also { SecureRandom().nextBytes(it) }
        .joinToString("") { "%02x".format(it) }

    private val appContext = context.applicationContext
    private val nsdManager = appContext.getSystemService(Context.NSD_SERVICE) as NsdManager

    val registry = PeerRegistry(scope)

    private val advertiser = DiscoveryAdvertiser(nsdManager)
    private val browser = DiscoveryBrowser(
        context = appContext,
        nsdManager = nsdManager,
        registry = registry,
        scope = scope,
        ownRid = { rid },
        ownServiceName = { advertiser.registeredServiceName },
    )

    /**
     * The TXT `dt` value. Platform-explicit per PROTOCOL.md §3.1 (the receiving UI cannot
     * tell a Pixel from an iPhone given only "phone"); sw600dp picks the form factor.
     */
    private val deviceType: DeviceType
        get() = if (appContext.resources.configuration.smallestScreenWidthDp >= 600) {
            DeviceType.ANDROID_TABLET
        } else {
            DeviceType.ANDROID_PHONE
        }

    private var started = false

    fun start() {
        if (started) return
        started = true

        scope.launch(Dispatchers.IO) { browser.start() }

        scope.launch(Dispatchers.IO) {
            settingsRepository.settings.collectLatest { settings ->
                // Display-name edits arrive per keystroke; don't flap the NSD
                // registration. apply()/stop() have no suspension points, so a newer
                // emission can only cancel this during the delay.
                delay(REREGISTER_DEBOUNCE_MS)
                advertiser.apply(
                    displayName = settings.displayName,
                    deviceType = deviceType,
                    visibility = settings.visibility,
                    rid = rid,
                    port = listenPort(),
                )
            }
        }
    }

    /** Explicit user refresh — the only in-process way the peer list is cleared. */
    fun refresh() {
        registry.clear()
        scope.launch(Dispatchers.IO) { browser.restart() }
    }

    private companion object {
        const val REREGISTER_DEBOUNCE_MS = 500L
    }
}
