package com.tsubuzaki.WoooshGo.ui

import android.content.Context
import android.icu.text.MeasureFormat
import android.icu.util.Measure
import android.icu.util.MeasureUnit
import android.text.format.Formatter
import com.tsubuzaki.WoooshGo.R
import java.util.Locale

/** Platform-rendered: never hand-build with a fixed Locale, ICU follows the reader's. */
fun formatBytes(context: Context, bytes: Long): String =
    Formatter.formatFileSize(context, bytes)

fun formatRate(context: Context, bytesPerSecond: Long): String =
    context.getString(R.string.transfer_progress_rate, formatBytes(context, bytesPerSecond))

/** Empty when the core has no estimate yet. */
fun formatEta(context: Context, seconds: Long): String = when {
    seconds < 0 -> ""
    else -> context.getString(R.string.transfer_progress_eta, formatDurationSeconds(seconds))
}

fun formatDuration(context: Context, millis: Long): String = when {
    millis <= 0 -> ""
    millis < 1000 -> measure(1, MeasureUnit.SECOND)
    else -> formatDurationSeconds(millis / 1000)
}

private fun formatDurationSeconds(seconds: Long): String = when {
    seconds < 60 -> measure(seconds, MeasureUnit.SECOND)
    seconds < 3600 -> measure(seconds / 60, MeasureUnit.MINUTE, seconds % 60, MeasureUnit.SECOND)
    else -> measure(seconds / 3600, MeasureUnit.HOUR, (seconds % 3600) / 60, MeasureUnit.MINUTE)
}

private fun measure(value: Long, unit: MeasureUnit): String =
    icu().format(Measure(value, unit))

private fun measure(a: Long, aUnit: MeasureUnit, b: Long, bUnit: MeasureUnit): String =
    if (b == 0L) measure(a, aUnit)
    else icu().formatMeasures(Measure(a, aUnit), Measure(b, bUnit))

private fun icu(): MeasureFormat =
    MeasureFormat.getInstance(Locale.getDefault(), MeasureFormat.FormatWidth.SHORT)
