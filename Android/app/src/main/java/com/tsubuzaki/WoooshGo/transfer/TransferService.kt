package com.tsubuzaki.WoooshGo.transfer

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.tsubuzaki.WoooshGo.MainActivity
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.WoooshApplication
import com.tsubuzaki.WoooshGo.ui.formatRate
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Foreground service for active transfers (DESIGN.md §7): `dataSync` type, progress
 * notification with a cancel action, and a partial wake lock held only while at least
 * one transfer is running.
 */
class TransferService : Service() {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var observeJob: Job? = null
    private var wakeLock: PowerManager.WakeLock? = null

    /**
     * The service starts before TransferStarted arrives: a connect plus an OFFER/consent
     * round trip sits in between. `serviceNeeded` covers both that window and running
     * transfers, and the service only stops once it has gone false after having been true
     * (or the startup grace expires).
     */
    private var sawWork = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val transferManager = (application as WoooshApplication).transferManager

        if (intent?.action == ACTION_CANCEL_ALL) {
            transferManager.cancelAll()
            return START_NOT_STICKY
        }

        val running = transferManager.transfers.value.filter { it.status == TransferStatus.RUNNING }
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            buildNotification(running),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )

        if (observeJob == null) {
            observeJob = serviceScope.launch {
                launch {
                    transferManager.serviceNeeded.collect { needed ->
                        if (!needed) {
                            if (sawWork) stopService()
                        } else {
                            sawWork = true
                            acquireWakeLock()
                            updateNotification(runningTransfers(transferManager))
                        }
                    }
                }
                launch {
                    transferManager.transfers.collect { list ->
                        val active = list.filter { it.status == TransferStatus.RUNNING }
                        if (active.isNotEmpty()) updateNotification(active)
                    }
                }
            }
            serviceScope.launch {
                delay(STARTUP_GRACE_MS)
                if (!sawWork) stopService()
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        releaseWakeLock()
        serviceScope.cancel()
        super.onDestroy()
    }

    private fun runningTransfers(manager: TransferManager): List<TransferUi> =
        manager.transfers.value.filter { it.status == TransferStatus.RUNNING }

    private fun stopService() {
        releaseWakeLock()
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    // ---------------------------------------------------------------- notification

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notification_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply { description = getString(R.string.notification_channel_desc) }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(active: List<TransferUi>): android.app.Notification {
        val total = active.sumOf { it.totalBytes }
        val transferred = active.sumOf { it.transferredBytes }
        val percent = if (total > 0) ((transferred * 100) / total).toInt().coerceIn(0, 100) else 0
        val rate = active.sumOf { it.rate }

        val title = when {
            active.isEmpty() -> getString(R.string.notification_channel_name)
            active.size == 1 -> {
                val transfer = active.first()
                val format =
                    if (transfer.direction == com.tsubuzaki.WoooshGo.core.TransferDirection.SEND) {
                        R.string.transfer_sending_to
                    } else {
                        R.string.transfer_receiving_from
                    }
                getString(format, transfer.peerName)
            }

            else -> resources.getQuantityString(
                R.plurals.notification_title_multiple, active.size, active.size,
            )
        }

        val contentIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val cancelIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, TransferService::class.java).setAction(ACTION_CANCEL_ALL),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(title)
            .setContentText(
                if (rate > 0) {
                    getString(
                        R.string.notification_progress_percent_rate,
                        percent,
                        formatRate(this, rate),
                    )
                } else {
                    getString(R.string.notification_progress_percent, percent)
                }
            )
            .setProgress(100, percent, total == 0L)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(contentIntent)
            .addAction(0, getString(R.string.action_cancel), cancelIntent)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    private fun updateNotification(active: List<TransferUi>) {
        val canNotify = Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        if (!canNotify) return
        runCatching {
            NotificationManagerCompat.from(this).notify(NOTIFICATION_ID, buildNotification(active))
        }
    }

    // ---------------------------------------------------------------- wake lock

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = powerManager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "wooosh:transfer").apply {
            setReferenceCounted(false)
            // Safety timeout: released explicitly when transfers finish; this is a backstop.
            acquire(30 * 60 * 1000L)
        }
    }

    private fun releaseWakeLock() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
    }

    companion object {
        const val CHANNEL_ID = "transfers"
        const val NOTIFICATION_ID = 100
        const val ACTION_CANCEL_ALL = "com.tsubuzaki.WoooshGo.action.CANCEL_ALL_TRANSFERS"
        private const val STARTUP_GRACE_MS = 10_000L
    }
}
