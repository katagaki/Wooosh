package com.tsubuzaki.WoooshGo.transfer

import android.content.ContentValues
import android.content.Context
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import java.io.File
import java.io.IOException
import kotlin.coroutines.resume
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout

/** [uri] is null when the pre-API-29 media scan produced none; the file is still saved. */
data class RoutedFile(val location: String, val uri: Uri?)

class StorageRouter(context: Context) {

    private val appContext = context.applicationContext

    /**
     * Moves [staged] into public Downloads (DESIGN.md §6) and deletes it on success. API
     * 29+ goes through MediaStore; 26-28 writes directly under WRITE_EXTERNAL_STORAGE.
     */
    @Throws(IOException::class)
    suspend fun routeToDownloads(
        staged: File,
        preferredName: String,
        mime: String,
        subfolder: String?,
    ): RoutedFile {
        if (!staged.isFile) throw IOException("Staged file missing: ${staged.path}")
        val name = sanitize(preferredName)
        return if (Build.VERSION.SDK_INT >= 29) {
            routeViaMediaStore(staged, name, mime, subfolder)
        } else {
            routeDirect(staged, name, subfolder)
        }
    }

    private fun routeViaMediaStore(staged: File, name: String, mime: String, subfolder: String?): RoutedFile {
        check(Build.VERSION.SDK_INT >= 29)
        val resolver = appContext.contentResolver
        val relativePath =
            if (subfolder != null) "${Environment.DIRECTORY_DOWNLOADS}/$subfolder"
            else Environment.DIRECTORY_DOWNLOADS
        val uniqueName = firstFreeName(name) { candidate ->
            resolver.query(
                MediaStore.Downloads.EXTERNAL_CONTENT_URI,
                arrayOf(MediaStore.MediaColumns._ID),
                "${MediaStore.MediaColumns.RELATIVE_PATH}=? AND ${MediaStore.MediaColumns.DISPLAY_NAME}=?",
                arrayOf("$relativePath/", candidate),
                null,
            )?.use { it.count > 0 } ?: false
        }

        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, uniqueName)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val itemUri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: throw IOException("MediaStore insert failed for $uniqueName")
        try {
            resolver.openOutputStream(itemUri)?.use { out ->
                staged.inputStream().use { it.copyTo(out) }
            } ?: throw IOException("Cannot open output stream for $itemUri")
            resolver.update(
                itemUri,
                ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
                null,
                null,
            )
        } catch (t: Throwable) {
            resolver.delete(itemUri, null, null)
            throw t
        }
        staged.delete()
        return RoutedFile("$relativePath/$uniqueName", itemUri)
    }

    @Suppress("DEPRECATION")
    private suspend fun routeDirect(staged: File, name: String, subfolder: String?): RoutedFile {
        val downloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        val targetDir = if (subfolder != null) File(downloads, subfolder) else downloads
        if (!targetDir.isDirectory && !targetDir.mkdirs()) {
            throw IOException("cannot create " + targetDir.path)
        }
        val uniqueName = firstFreeName(name) { candidate -> File(targetDir, candidate).exists() }
        val target = File(targetDir, uniqueName)
        staged.inputStream().use { input ->
            target.outputStream().use { input.copyTo(it) }
        }
        staged.delete()
        val prefix = if (subfolder != null) "Download/$subfolder" else "Download"
        return RoutedFile("$prefix/$uniqueName", scan(target))
    }

    /**
     * The only route to an openable URI on API 26-28 without a FileProvider (`file://` in
     * ACTION_VIEW throws `FileUriExposedException`). Best effort: the file is saved anyway.
     */
    private suspend fun scan(file: File): Uri? = try {
        withTimeout(SCAN_TIMEOUT_MS) {
            suspendCancellableCoroutine { continuation ->
                MediaScannerConnection.scanFile(
                    appContext, arrayOf(file.absolutePath), null,
                ) { _, uri -> if (continuation.isActive) continuation.resume(uri) }
            }
        }
    } catch (_: TimeoutCancellationException) {
        null
    }

    /** Never overwrites: " (2)", " (3)", ... before the extension. */
    private fun firstFreeName(name: String, isTaken: (String) -> Boolean): String {
        if (!isTaken(name)) return name
        val dot = name.lastIndexOf('.')
        val base = if (dot > 0) name.substring(0, dot) else name
        val ext = if (dot > 0) name.substring(dot) else ""
        var counter = 2
        while (true) {
            val candidate = "$base ($counter)$ext"
            if (!isTaken(candidate)) return candidate
            counter++
        }
    }

    /** Receiver-side name sanitization (PROTOCOL.md §5). */
    private fun sanitize(name: String): String {
        val cleaned = name
            .replace('/', '_')
            .replace('\\', '_')
            .filter { it.code >= 0x20 }
            .trim()
        return cleaned.ifBlank { "received-file" }
    }

    private companion object {
        const val SCAN_TIMEOUT_MS = 5_000L
    }
}
