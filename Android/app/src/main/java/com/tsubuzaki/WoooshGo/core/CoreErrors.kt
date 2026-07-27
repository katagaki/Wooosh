package com.tsubuzaki.WoooshGo.core

import android.content.Context
import com.tsubuzaki.WoooshGo.R
import uniffi.wooosh_core.relayMaxFileBytes

/** The core's outcome tokens are untranslatable and log-shaped; never shown verbatim. */
fun transferErrorMessage(context: Context, raw: String?): String {
    val text = raw?.lowercase().orEmpty()
    // The limit comes from the core so the copy cannot drift from the rule.
    if (text.contains("relay_file_too_large")) {
        return context.getString(
            R.string.error_relay_file_too_large,
            android.text.format.Formatter.formatShortFileSize(context, relayMaxFileBytes().toLong()),
        )
    }
    return context.getString(
        when {

            text.contains("declined") || text.contains("rejected") ->
                R.string.error_declined_by_peer

            text.contains("cancelled by peer") || text.contains("canceled by peer") ->
                R.string.error_cancelled_by_peer

            text.contains("cancel") -> R.string.transfer_state_cancelled
            text.contains("timed out") || text.contains("timeout") -> R.string.error_no_answer
            text.contains("pairing required") -> R.string.error_pairing_required
            text.contains("version") -> R.string.error_version_mismatch
            text.contains("key changed") -> R.string.error_key_changed
            else -> R.string.error_transfer_failed
        }
    )
}

fun pairingErrorMessage(context: Context, raw: String?): String {
    val text = raw?.lowercase().orEmpty()
    return context.getString(
        when {
            text.contains("expired") -> R.string.error_pairing_expired
            text.contains("mismatch") -> R.string.error_qr_key_mismatch
            text.contains("invalid") || text.contains("payload") -> R.string.error_invalid_qr
            text.contains("timed out") || text.contains("timeout") -> R.string.error_sas_timeout
            text.contains("connect") || text.contains("unreachable") -> R.string.error_connect
            else -> R.string.error_pairing_failed
        }
    )
}
