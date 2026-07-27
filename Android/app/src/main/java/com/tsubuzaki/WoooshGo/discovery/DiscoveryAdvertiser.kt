package com.tsubuzaki.WoooshGo.discovery

import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.util.Log
import com.tsubuzaki.WoooshGo.peers.DeviceType
import com.tsubuzaki.WoooshGo.settings.Visibility

/** TXT layout of PROTOCOL.md §3.1; the port is the core's QUIC listener. */
class DiscoveryAdvertiser(private val nsdManager: NsdManager) {

    /** NSD may rename on conflict, and self-filtering needs the name it actually used. */
    @Volatile
    var registeredServiceName: String? = null
        private set

    private var registrationListener: NsdManager.RegistrationListener? = null

    @Synchronized
    fun apply(
        displayName: String,
        deviceType: DeviceType,
        visibility: Visibility,
        rid: String,
        port: Int,
    ) {
        stop()
        if (visibility == Visibility.OFF) return
        // Nothing to advertise until the core has bound its UDP socket.
        if (port <= 0) return

        val serviceInfo = NsdServiceInfo().apply {
            serviceName = "$displayName (${rid.takeLast(4)})"
            serviceType = SERVICE_TYPE
            // DNS-SD convention: SRV mirrors the QUIC UDP port (PROTOCOL.md §1).
            this.port = port
            setAttribute("v", "1")
            setAttribute("rid", rid)
            setAttribute("dn", displayName)
            // UNKNOWN has no wire value; an absent `dt` reads as unknown (PROTOCOL.md §3.1).
            deviceType.txtValue?.let { setAttribute("dt", it) }
            setAttribute("p", port.toString())
            setAttribute("vis", visibility.txtValue)
        }
        Log.i(
            TAG,
            "advertising \"${serviceInfo.serviceName}\" dt=${deviceType.txtValue ?: "(omitted)"} " +
                "rid=$rid p=$port vis=${visibility.txtValue}",
        )

        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {
                registeredServiceName = info.serviceName
            }

            override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) = Unit
            override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
            override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) = Unit
        }
        registrationListener = listener
        // SDK 37+ throws SecurityException without ACCESS_LOCAL_NETWORK; losing discovery
        // must not take the app down.
        try {
            nsdManager.registerService(serviceInfo, NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (e: SecurityException) {
            Log.w(TAG, "cannot advertise without local network permission", e)
            registrationListener = null
        }
    }

    @Synchronized
    fun stop() {
        registrationListener?.let { runCatching { nsdManager.unregisterService(it) } }
        registrationListener = null
        registeredServiceName = null
    }

    companion object {
        private const val TAG = "WoooshDiscovery"
        const val SERVICE_TYPE = "_wooosh._tcp."
    }
}
