package com.tsubuzaki.WoooshGo.share

import android.content.Context
import android.content.Intent
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import com.tsubuzaki.WoooshGo.MainActivity
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.core.TrustedPeerInfo

/** Direct Share (DESIGN.md §8); [CATEGORY] must match res/xml/shortcuts.xml. */
object ShortcutPublisher {

    const val CATEGORY = "com.tsubuzaki.WoooshGo.category.SHARE_TARGET"
    private const val SHORTCUT_ID_PREFIX = "peer-"
    private const val MAX_SHORTCUTS = 4

    fun publish(context: Context, pairedDevices: List<TrustedPeerInfo>) {
        runCatching {
            ShortcutManagerCompat.removeAllDynamicShortcuts(context)
            if (pairedDevices.isEmpty()) return
            val shortcuts = pairedDevices
                .sortedByDescending { it.pairedAtMillis }
                .take(MAX_SHORTCUTS)
                .map { device ->
                    ShortcutInfoCompat.Builder(context, shortcutId(device.deviceId))
                        .setShortLabel(
                            device.displayName.ifBlank {
                                context.getString(R.string.shortcut_fallback_name)
                            }
                        )
                        .setLongLived(true)
                        .setIcon(IconCompat.createWithResource(context, R.mipmap.ic_launcher))
                        .setCategories(setOf(CATEGORY))
                        .setIntent(
                            Intent(context, MainActivity::class.java).setAction(Intent.ACTION_MAIN)
                        )
                        .build()
                }
            ShortcutManagerCompat.addDynamicShortcuts(context, shortcuts)
        }
    }

    fun shortcutId(deviceId: String) = "$SHORTCUT_ID_PREFIX$deviceId"

    fun deviceIdFromShortcut(shortcutId: String): String? =
        shortcutId.takeIf { it.startsWith(SHORTCUT_ID_PREFIX) }?.removePrefix(SHORTCUT_ID_PREFIX)
}
