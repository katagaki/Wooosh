package com.tsubuzaki.WoooshGo.share

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import com.tsubuzaki.WoooshGo.MainActivity
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.WoooshApplication
import com.tsubuzaki.WoooshGo.ui.theme.WoooshTheme
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Share entry point (DESIGN.md §8): receives ACTION_SEND / ACTION_SEND_MULTIPLE,
 * stages the shared items (copies them into cacheDir — share-sheet URI grants do not
 * outlive this activity reliably), then opens the device list armed to send on tap.
 *
 * When launched from a Direct Share shortcut, the target paired device rides along and
 * the main screen auto-sends as soon as that device is alive in the list.
 */
class ShareTargetActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            WoooshTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    Column(
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(32.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center,
                    ) {
                        CircularProgressIndicator()
                        Spacer(Modifier.height(16.dp))
                        Text(
                            stringResource(R.string.share_preparing),
                            style = MaterialTheme.typography.bodyLarge,
                        )
                    }
                }
            }
        }

        val app = application as WoooshApplication
        lifecycleScope.launch {
            val staged = withContext(Dispatchers.IO) { stageAll(extractUris(intent)) }
            if (staged.isEmpty()) {
                Toast.makeText(
                    this@ShareTargetActivity,
                    getString(R.string.share_nothing),
                    Toast.LENGTH_SHORT,
                ).show()
                finish()
                return@launch
            }

            val shortcutId = intent.getStringExtra(Intent.EXTRA_SHORTCUT_ID)
            val targetDevice = shortcutId
                ?.let(ShortcutPublisher::deviceIdFromShortcut)
                ?.let { deviceId ->
                    app.trustStore.refreshNow().firstOrNull { it.deviceId == deviceId }
                }

            app.outbox.arm(
                OutboxRepository.StagedShare(
                    uris = staged,
                    targetDeviceId = targetDevice?.deviceId,
                    targetDisplayName = targetDevice?.displayName,
                )
            )
            startActivity(
                Intent(this@ShareTargetActivity, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            )
            finish()
        }
    }

    private fun extractUris(intent: Intent): List<Uri> {
        val uris = when (intent.action) {
            Intent.ACTION_SEND ->
                listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))

            Intent.ACTION_SEND_MULTIPLE ->
                IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                    ?.filterNotNull()
                    .orEmpty()

            else -> emptyList()
        }
        if (uris.isNotEmpty()) return uris
        // ClipData fallback (some apps only populate clipData).
        val clip = intent.clipData ?: return emptyList()
        return (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
    }

    /** Copies each shared item into cacheDir/share_staging and returns file:// URIs. */
    private fun stageAll(uris: List<Uri>): List<Uri> {
        val stagingRoot = File(cacheDir, "share_staging/${UUID.randomUUID()}").apply { mkdirs() }
        val staged = uris.mapNotNull { uri ->
            runCatching {
                val name = displayNameOf(uri)
                val target = uniqueIn(stagingRoot, name)
                contentResolver.openInputStream(uri)?.use { input ->
                    target.outputStream().use { input.copyTo(it) }
                } ?: return@runCatching null
                Uri.fromFile(target)
            }.getOrNull()
        }
        if (staged.isNotEmpty()) return staged

        // Text-only share (EXTRA_TEXT, no stream) → stage as a .txt file.
        val text = intent.getStringExtra(Intent.EXTRA_TEXT) ?: return emptyList()
        val target = uniqueIn(stagingRoot, "Shared text.txt")
        target.writeText(text)
        return listOf(Uri.fromFile(target))
    }

    private fun displayNameOf(uri: Uri): String {
        if (uri.scheme == "content") {
            runCatching {
                contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                    ?.use { cursor ->
                        if (cursor.moveToFirst() && !cursor.isNull(0)) return cursor.getString(0)
                    }
            }
        }
        return uri.lastPathSegment?.substringAfterLast('/')?.takeIf { it.isNotBlank() } ?: "shared-file"
    }

    private fun uniqueIn(dir: File, name: String): File {
        var candidate = File(dir, name)
        var counter = 2
        while (candidate.exists()) {
            val dot = name.lastIndexOf('.')
            val base = if (dot > 0) name.substring(0, dot) else name
            val ext = if (dot > 0) name.substring(dot) else ""
            candidate = File(dir, "$base ($counter)$ext")
            counter++
        }
        return candidate
    }
}
