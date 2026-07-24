package com.tsubuzaki.WoooshGo.discovery

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import com.tsubuzaki.WoooshGo.peers.DeviceType
import com.tsubuzaki.WoooshGo.peers.PeerRegistry
import java.net.Inet6Address
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Browses for `_wooosh._tcp.` services and feeds sightings/losses into the PeerRegistry.
 *
 * Resolution uses the ServiceInfoCallback API on 34+ (which also delivers TXT updates)
 * and the legacy one-at-a-time resolveService below that, serialized through a queue.
 *
 * ## Scan cadence (PROTOCOL.md §3.2 / §3.3)
 *
 * The spec's "every 2 s" is a scan cadence for the UDP-broadcast fallback; Android has
 * no such knob. `discoverServices` is a subscription and the platform mDNS daemon owns
 * query timing. Restarting the browser on a 2 s loop to force queries would churn the
 * peer list, which is append-only and never reorders (DESIGN.md §5) — so it is not done.
 *
 * [SCAN_INTERVAL_MS] is instead a re-resolve tick, and only on API < 34: `resolveService`
 * is one-shot, so a peer that restarts on a new ephemeral port would keep a stale address
 * until NSD reported it lost. The tick refreshes rows in place and can never add, remove
 * or reorder one. On 34+ `registerServiceInfoCallback` already streams those updates.
 *
 * The spec's 30 s background tier is not implemented: the shell has no
 * foreground/background distinction and discovery runs for as long as the process does.
 */
class DiscoveryBrowser(
    context: Context,
    private val nsdManager: NsdManager,
    private val registry: PeerRegistry,
    private val scope: CoroutineScope,
    private val ownRid: () -> String,
    private val ownServiceName: () -> String?,
) {

    private val appContext = context.applicationContext
    private val executor = Executors.newSingleThreadExecutor()

    private var multicastLock: WifiManager.MulticastLock? = null
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private var rescanJob: Job? = null

    private val serviceNameToRid = ConcurrentHashMap<String, String>()
    private val infoCallbacks = ConcurrentHashMap<String, NsdManager.ServiceInfoCallback>()

    // Legacy (< API 34) resolve path: NsdManager only allows one resolve at a time.
    private val resolveQueue = ArrayDeque<NsdServiceInfo>()
    private var resolveInFlight = false

    /** Services seen by name on the legacy path, for the periodic re-resolve. */
    private val knownServices = ConcurrentHashMap<String, NsdServiceInfo>()

    @Synchronized
    fun start() {
        if (discoveryListener != null) return

        // Many devices drop mDNS multicast unless a multicast lock is held.
        val wifiManager = appContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifiManager.createMulticastLock("wooosh-discovery").apply {
            setReferenceCounted(false)
            acquire()
        }

        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                releaseLock()
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit

            override fun onServiceFound(service: NsdServiceInfo) {
                if (service.serviceName == ownServiceName()) return
                if (Build.VERSION.SDK_INT >= 34) {
                    registerInfoCallback(service)
                } else {
                    knownServices[service.serviceName] = service
                    enqueueResolve(service)
                }
            }

            override fun onServiceLost(service: NsdServiceInfo) {
                if (Build.VERSION.SDK_INT >= 34) {
                    infoCallbacks.remove(service.serviceName)?.let {
                        runCatching { nsdManager.unregisterServiceInfoCallback(it) }
                    }
                }
                knownServices.remove(service.serviceName)
                // Starts the registry's 10 s grace period rather than an immediate stale.
                serviceNameToRid[service.serviceName]?.let { registry.onLost(it) }
            }
        }
        discoveryListener = listener
        // Same SDK 37+ SecurityException as the advertiser (ACCESS_LOCAL_NETWORK).
        try {
            nsdManager.discoverServices(
                DiscoveryAdvertiser.SERVICE_TYPE,
                NsdManager.PROTOCOL_DNS_SD,
                listener,
            )
        } catch (e: SecurityException) {
            Log.w("WoooshDiscovery", "cannot browse without local network permission", e)
            discoveryListener = null
            return
        }
        startRescanTicker()
    }

    /**
     * Legacy-path refresh at the PROTOCOL.md §3.3 cadence — see the class doc for why
     * this re-resolves instead of restarting discovery, and why 34+ needs no ticker.
     *
     * Re-resolving can never create or drop a row, and never shortcuts the 10 s stale
     * threshold.
     */
    private fun startRescanTicker() {
        if (Build.VERSION.SDK_INT >= 34) return
        rescanJob?.cancel()
        rescanJob = scope.launch {
            while (isActive) {
                delay(SCAN_INTERVAL_MS)
                // Skip the tick entirely if the single-resolve queue is still busy;
                // piling on would just grow an unbounded backlog on a crowded network.
                val busy = synchronized(resolveQueue) { resolveInFlight || resolveQueue.isNotEmpty() }
                if (busy) continue
                knownServices.values.forEach(::enqueueResolve)
            }
        }
    }

    @Synchronized
    fun stop() {
        rescanJob?.cancel()
        rescanJob = null
        discoveryListener?.let { runCatching { nsdManager.stopServiceDiscovery(it) } }
        discoveryListener = null
        if (Build.VERSION.SDK_INT >= 34) {
            infoCallbacks.values.forEach {
                runCatching { nsdManager.unregisterServiceInfoCallback(it) }
            }
        }
        infoCallbacks.clear()
        knownServices.clear()
        synchronized(resolveQueue) {
            resolveQueue.clear()
            resolveInFlight = false
        }
        releaseLock()
    }

    fun restart() {
        stop()
        start()
    }

    private fun releaseLock() {
        multicastLock?.let { runCatching { if (it.isHeld) it.release() } }
        multicastLock = null
    }

    private fun registerInfoCallback(service: NsdServiceInfo) {
        if (Build.VERSION.SDK_INT < 34) return
        if (infoCallbacks.containsKey(service.serviceName)) return
        val callback = object : NsdManager.ServiceInfoCallback {
            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                infoCallbacks.remove(service.serviceName)
            }

            override fun onServiceUpdated(serviceInfo: NsdServiceInfo) {
                handleResolved(serviceInfo)
            }

            override fun onServiceLost() = Unit
            override fun onServiceInfoCallbackUnregistered() = Unit
        }
        infoCallbacks[service.serviceName] = callback
        runCatching { nsdManager.registerServiceInfoCallback(service, executor, callback) }
            .onFailure { infoCallbacks.remove(service.serviceName) }
    }

    private fun enqueueResolve(service: NsdServiceInfo) {
        synchronized(resolveQueue) {
            resolveQueue.addLast(service)
        }
        drainResolveQueue()
    }

    private fun drainResolveQueue() {
        val next: NsdServiceInfo
        synchronized(resolveQueue) {
            if (resolveInFlight) return
            next = resolveQueue.removeFirstOrNull() ?: return
            resolveInFlight = true
        }
        @Suppress("DEPRECATION")
        nsdManager.resolveService(next, object : NsdManager.ResolveListener {
            override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                handleResolved(serviceInfo)
                finishResolve()
            }

            override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                finishResolve()
            }
        })
    }

    private fun finishResolve() {
        synchronized(resolveQueue) { resolveInFlight = false }
        drainResolveQueue()
    }

    private fun handleResolved(serviceInfo: NsdServiceInfo) {
        val attributes = serviceInfo.attributes ?: return
        val rid = attributes["rid"]?.toString(Charsets.UTF_8) ?: return
        if (!RID_PATTERN.matches(rid)) return
        if (rid == ownRid()) return

        serviceNameToRid[serviceInfo.serviceName] = rid
        val displayName = attributes["dn"]?.toString(Charsets.UTF_8)
            ?.takeIf { it.isNotBlank() }
            ?: serviceInfo.serviceName
        val deviceType = DeviceType.fromTxt(attributes["dt"]?.toString(Charsets.UTF_8))
        val port = attributes["p"]?.toString(Charsets.UTF_8)?.toIntOrNull() ?: serviceInfo.port

        registry.onSighting(rid, displayName, deviceType, port, hostsOf(serviceInfo))
    }

    /**
     * The resolved addresses are what `connect_peer` needs (DESIGN.md §4); the TXT record
     * only carries the port.
     *
     * IPv4 first, and link-local IPv6 dropped: those need a scope id that
     * `InetAddress.getHostAddress()` renders as `fe80::1%wlan0`, which the core's
     * `lookup_host` cannot parse on every platform.
     */
    @Suppress("DEPRECATION")
    private fun hostsOf(serviceInfo: NsdServiceInfo): List<String> {
        val addresses = if (Build.VERSION.SDK_INT >= 34) {
            serviceInfo.hostAddresses
        } else {
            listOfNotNull(serviceInfo.host)
        }
        return addresses
            .mapNotNull { address ->
                when {
                    address.isAnyLocalAddress || address.isLoopbackAddress -> null
                    address is Inet6Address && address.isLinkLocalAddress -> null
                    else -> address.hostAddress?.substringBefore('%')
                }
            }
            .distinct()
            .sortedBy { if (it.contains(':')) 1 else 0 }
    }

    private companion object {
        val RID_PATTERN = Regex("[0-9a-f]{16}")

        /**
         * Foreground scan cadence (PROTOCOL.md §3.3). Never pair a change here with a
         * change to the registry's 10 s stale threshold: scanning faster is for finding
         * devices sooner, never for dropping them sooner.
         */
        const val SCAN_INTERVAL_MS = 2_000L
    }
}
