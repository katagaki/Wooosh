package com.tsubuzaki.WoooshGo.transfer

import android.Manifest
import android.app.DownloadManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.tsubuzaki.WoooshGo.MainActivity
import com.tsubuzaki.WoooshGo.R
import kotlin.math.absoluteValue

/**
 * The "files arrived" notification, posted once a received transfer has finished and every
 * file has been routed (DESIGN.md §6).
 *
 * Separate from [TransferService]'s ongoing progress notification in both channel and id:
 * that one is IMPORTANCE_LOW and is torn down with the foreground service the moment the
 * transfer ends, and it is the arrival — not the progress — that the user wants to be told
 * about and to tap.
 */
class ReceivedNotifier(context: Context) {

    private val appContext = context.applicationContext

    init {
        val channel = NotificationChannel(
            CHANNEL_ID,
            appContext.getString(R.string.notification_received_channel_name),
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply { description = appContext.getString(R.string.notification_received_channel_desc) }
        appContext.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    fun notifyReceived(transfer: TransferUi) {
        if (!canNotify()) return
        val saved = transfer.files.filter { it.error == null && it.routed }
        if (saved.isEmpty()) return

        val title = appContext.resources.getQuantityString(
            R.plurals.notification_received_title, saved.size, saved.size,
        )
        val text = if (saved.size == 1) {
            saved.first().meta.name
        } else {
            appContext.getString(R.string.notification_received_from, transfer.peerName)
        }

        val notification = NotificationCompat.Builder(appContext, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(title)
            .setContentText(text)
            .setSubText(if (saved.size == 1) transfer.peerName else null)
            .setContentIntent(openIntent(saved))
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .build()

        runCatching {
            NotificationManagerCompat.from(appContext).notify(notificationId(transfer.id), notification)
        }
    }

    /**
     * Tapping opens the file itself when the transfer carried exactly one, and the system
     * Downloads list when it carried several. Both fall back to Wooosh when nothing on the
     * device can handle the intent, so the tap is never a no-op.
     */
    private fun openIntent(saved: List<FileState>): PendingIntent {
        val single = saved.singleOrNull()?.takeIf { it.savedUri != null }
        val view = if (single != null) {
            Intent(Intent.ACTION_VIEW)
                .setDataAndType(single.savedUri, single.meta.mime)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        } else {
            Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        val intent = if (view.resolveActivity(appContext.packageManager) != null) {
            view
        } else {
            Intent(appContext, MainActivity::class.java)
        }
        return PendingIntent.getActivity(
            appContext,
            requestCode(saved.firstOrNull()?.savedUri),
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun canNotify(): Boolean =
        Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(appContext, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    /**
     * One notification per transfer rather than one global "received" notification: two
     * transfers landing together are two separate things to open. Distinct request codes for
     * the same reason, or [PendingIntent.FLAG_UPDATE_CURRENT] would rewrite the earlier
     * notification's intent to point at the later transfer's file.
     */
    private fun notificationId(transferId: String) =
        RECEIVED_ID_BASE + (transferId.hashCode().absoluteValue % 1000)

    private fun requestCode(uri: Uri?) = (uri?.hashCode() ?: 0).absoluteValue % 100_000

    private companion object {
        const val CHANNEL_ID = "received"

        /** Clear of [TransferService.NOTIFICATION_ID] and the 1000 ids reserved above it. */
        const val RECEIVED_ID_BASE = 1000
    }
}
