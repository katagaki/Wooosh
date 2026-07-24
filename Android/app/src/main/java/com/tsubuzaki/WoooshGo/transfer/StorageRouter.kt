package com.tsubuzaki.WoooshGo.transfer

import android.content.ContentValues
import android.content.Context
import android.media.MediaScannerConnection
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import java.io.File
import java.io.IOException

/**
 * Storage routing (DESIGN.md §6): on FileReady, everything goes to Downloads.
 *
 * - API 29+: MediaStore.Downloads insert with IS_PENDING=1, stream, then publish.
 * - API 26–28: direct write to the public Downloads directory (WRITE_EXTERNAL_STORAGE,
 *   maxSdkVersion 28) + media scan.
 * - Transfers with more than 20 files land in `Wooosh/<yyyy-MM-dd>/` under Downloads
 *   (the caller passes [subfolder]); otherwise Downloads root.
 * - Name collisions get " (2)", " (3)", ... — never overwrite.
 */
class StorageRouter(context: Context) {

    private val appContext = context.applicationContext

    /**
     * Moves [staged] into public Downloads. Returns the user-visible location
     * ("Download/Wooosh/2026-07-24/IMG_0001 (2).jpeg"). Deletes [staged] on success.
     */
    @Throws(IOException::class)
    fun routeToDownloads(staged: File, preferredName: String, mime: String, subfolder: String?): String {
        if (!staged.isFile) throw IOException("Staged file missing: ${staged.path}")
        val name = sanitize(preferredName)
        return if (Build.VERSION.SDK_INT >= 29) {
            routeViaMediaStore(staged, name, mime, subfolder)
        } else {
            routeDirect(staged, name, subfolder)
        }
    }

    private fun routeViaMediaStore(staged: File, name: String, mime: String, subfolder: String?): String {
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
        return "$relativePath/$uniqueName"
    }

    @Suppress("DEPRECATION")
    private fun routeDirect(staged: File, name: String, subfolder: String?): String {
        val downloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        val targetDir = if (subfolder != null) File(downloads, subfolder) else downloads
        if (!targetDir.isDirectory && !targetDir.mkdirs()) {
            // Log detail only; the user-facing wording is chosen by the caller.
            throw IOException("cannot create " + targetDir.path)
        }
        val uniqueName = firstFreeName(name) { candidate -> File(targetDir, candidate).exists() }
        val target = File(targetDir, uniqueName)
        staged.inputStream().use { input ->
            target.outputStream().use { input.copyTo(it) }
        }
        staged.delete()
        MediaScannerConnection.scanFile(appContext, arrayOf(target.absolutePath), null, null)
        val prefix = if (subfolder != null) "Download/$subfolder" else "Download"
        return "$prefix/$uniqueName"
    }

    /** " (2)", " (3)", ... before the extension; never overwrites. */
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
}
