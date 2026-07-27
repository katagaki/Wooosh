package com.tsubuzaki.WoooshGo

import android.app.Application
import android.util.Log
import com.tsubuzaki.WoooshGo.core.CoreConfig
import com.tsubuzaki.WoooshGo.core.CoreVisibility
import com.tsubuzaki.WoooshGo.core.RealCore
import com.tsubuzaki.WoooshGo.core.WoooshCore
import com.tsubuzaki.WoooshGo.discovery.DiscoveryController
import com.tsubuzaki.WoooshGo.identity.IdentityManager
import com.tsubuzaki.WoooshGo.pairing.PairingManager
import com.tsubuzaki.WoooshGo.peers.DeviceType
import com.tsubuzaki.WoooshGo.settings.SettingsRepository
import com.tsubuzaki.WoooshGo.settings.Visibility
import com.tsubuzaki.WoooshGo.share.OutboxRepository
import com.tsubuzaki.WoooshGo.share.ShortcutPublisher
import com.tsubuzaki.WoooshGo.transfer.TransferManager
import com.tsubuzaki.WoooshGo.trust.TrustStore
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

class WoooshApplication : Application() {

    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val identityManager by lazy { IdentityManager(this) }
    val settingsRepository by lazy { SettingsRepository(this) }

    val core: WoooshCore by lazy { RealCore(this, appScope, identityManager) }

    val discovery by lazy {
        DiscoveryController(this, appScope, settingsRepository) {
            // TXT `p` must advertise the core's real QUIC port (DESIGN.md §4).
            core.listenAddr()?.substringAfterLast(':')?.toIntOrNull() ?: 0
        }
    }

    /** A cache of the CORE's trust store (`trustedPeers()`), not a second copy of it. */
    val trustStore by lazy { TrustStore(appScope, core) }
    val outbox = OutboxRepository()
    val transferManager by lazy {
        TransferManager(this, appScope, core, trustStore, discovery.registry)
    }
    val pairingManager by lazy {
        PairingManager(this, appScope, core, trustStore) { internetEnabled }
    }

    /** Mirrors the stored relay setting so the pairing manager can read it synchronously. */
    @Volatile
    private var internetEnabled: Boolean = true

    @Volatile
    var coreStartError: String? = null
        private set

    /** Set when the core refused the relay address; it keeps its last working config. */
    private val _relayError = MutableStateFlow<String?>(null)
    val relayError: StateFlow<String?> = _relayError.asStateFlow()

    override fun onCreate() {
        super.onCreate()

        appScope.launch(Dispatchers.IO) {
            val settings = settingsRepository.settings.first()
            // App-private internal storage: staged bytes are unverified, trust is security state.
            val stagingDir = File(filesDir, "staging")
            try {
                core.start(
                    CoreConfig(
                        displayName = settings.displayName,
                        deviceType = deviceType(),
                        visibility = settings.visibility.toCore(),
                        stagingDir = stagingDir.absolutePath,
                        trustStorePath = File(filesDir, "trust/trust.json").absolutePath,
                    )
                )
                Log.i(
                    TAG,
                    "core ready: deviceId=${core.deviceId()} listenAddr=${core.listenAddr()}",
                )
                // The core is the only trust list; read it at launch (PROTOCOL.md §4.5).
                trustStore.refreshNow()
            } catch (t: Throwable) {
                // Core failures are internal English: never surface them raw.
                coreStartError = getString(R.string.error_core_start)
                Log.e(TAG, "core failed to start", t)
            }

            // Touch the lazily-created event consumers so no core event is missed.
            transferManager
            pairingManager

            // Discovery advertises the core's bound port, so it starts after the core.
            discovery.start()

            internetEnabled = settings.internetEnabled
            var appliedRelays: List<String>? = settings.relayUrls
            // The core boots on its own default, so push the stored preference before
            // anything can mint a ticket.
            runCatching { core.setRelayUrls(appliedRelays) }
                .onFailure {
                    _relayError.value = getString(R.string.error_relay_url_invalid)
                    Log.w(TAG, "initial setRelayUrls failed", it)
                }

            settingsRepository.settings.collect { current ->
                core.setVisibility(current.visibility.toCore())
                internetEnabled = current.internetEnabled
                // Only on a real change: applying this tears the iroh endpoint down.
                if (current.relayUrls != appliedRelays) {
                    val wanted = current.relayUrls
                    runCatching { core.setRelayUrls(wanted) }
                        .onSuccess {
                            appliedRelays = wanted
                            _relayError.value = null
                        }
                        .onFailure {
                            _relayError.value = getString(R.string.error_relay_url_invalid)
                            Log.w(TAG, "setRelayUrls($wanted) rejected", it)
                        }
                }
            }
        }

        appScope.launch {
            trustStore.devices.collect { devices ->
                ShortcutPublisher.publish(this@WoooshApplication, devices)
            }
        }
    }

    override fun onTerminate() {
        core.stop()
        super.onTerminate()
    }

    private fun deviceType(): DeviceType =
        if (resources.configuration.smallestScreenWidthDp >= 600) {
            DeviceType.ANDROID_TABLET
        } else {
            DeviceType.ANDROID_PHONE
        }

    private fun Visibility.toCore() = when (this) {
        Visibility.EVERYONE -> CoreVisibility.EVERYONE
        Visibility.PAIRED_ONLY -> CoreVisibility.PAIRED_ONLY
        Visibility.OFF -> CoreVisibility.OFF
    }

    private companion object {
        const val TAG = "WoooshApp"
    }
}
