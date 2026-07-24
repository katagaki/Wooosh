package com.tsubuzaki.WoooshGo.core

import android.content.Context
import com.tsubuzaki.WoooshGo.R

/**
 * Turns a core failure into something a person can act on.
 *
 * The core reports outcomes as short internal English tokens ("cancelled", "declined by
 * receiver", "timed out waiting for DECISION"). Never show those verbatim: they cannot be
 * translated and they read like a log line. Map them to real copy; the raw text stays in
 * the log.
 */
fun transferErrorMessage(context: Context, raw: String?): String {
    val text = raw?.lowercase().orEmpty()
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

/** The same treatment for a failure reported while pairing. */
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
