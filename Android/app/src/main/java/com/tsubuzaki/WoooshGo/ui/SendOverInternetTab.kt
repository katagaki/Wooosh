package com.tsubuzaki.WoooshGo.ui

import android.graphics.Bitmap
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Public
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.pairing.QrCodes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Nothing is paired: one-shot code, so no fingerprint to check (PROTOCOL.md §9.4). */
@Composable
fun SendOverInternetTab(
    beginInternetTicket: suspend () -> String,
    endInternetTicket: () -> Unit,
    onStageFiles: (List<Uri>) -> Unit,
    redeemedPeerId: String?,
    onRedeemed: (String) -> Unit,
) {
    var ticket by remember { mutableStateOf<String?>(null) }
    var qr by remember { mutableStateOf<Bitmap?>(null) }
    var minting by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var sent by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // The code is a live capability, so it dies with the tab.
    DisposableEffect(Unit) { onDispose { if (ticket != null || minting) endInternetTicket() } }

    LaunchedEffect(redeemedPeerId) {
        val id = redeemedPeerId ?: return@LaunchedEffect
        if (!sent) {
            sent = true
            onRedeemed(id)
        }
    }

    val picker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments()
    ) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        onStageFiles(uris)
        minting = true
        failed = false
        scope.launch {
            runCatching { beginInternetTicket() }
                .onSuccess { minted ->
                    qr = withContext(Dispatchers.Default) { QrCodes.encode(minted, 720) }
                    ticket = minted
                }
                .onFailure { failed = true }
            minting = false
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val bitmap = qr
        when {
            sent -> {
                Text(
                    text = stringResource(R.string.internet_send_started_title),
                    style = MaterialTheme.typography.titleMedium,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.internet_send_started_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }

            bitmap != null -> {
                Text(
                    text = stringResource(R.string.internet_send_ready_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(24.dp))
                // Always black-on-white so any scanner can read it.
                Box(
                    modifier = Modifier
                        .fillMaxWidth(0.8f)
                        .aspectRatio(1f)
                        .background(Color.White, RoundedCornerShape(16.dp))
                        .padding(12.dp),
                ) {
                    Image(
                        bitmap = bitmap.asImageBitmap(),
                        contentDescription = stringResource(R.string.internet_qr_a11y),
                        modifier = Modifier.fillMaxSize(),
                        contentScale = ContentScale.Fit,
                    )
                }
                Spacer(Modifier.height(24.dp))
                CircularProgressIndicator()
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.internet_send_waiting),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            minting -> {
                Spacer(Modifier.height(40.dp))
                CircularProgressIndicator()
                Spacer(Modifier.height(16.dp))
                Text(
                    text = stringResource(R.string.internet_preparing),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            else -> {
                Icon(
                    Icons.Outlined.Public,
                    contentDescription = null,
                    modifier = Modifier.size(56.dp),
                    tint = MaterialTheme.colorScheme.primary,
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    text = stringResource(R.string.internet_send_intro_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(12.dp))
                Text(
                    text = stringResource(R.string.internet_relay_note),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(24.dp))
                Button(onClick = { picker.launch(arrayOf("*/*")) }) {
                    Text(stringResource(R.string.action_choose_files))
                }
            }
        }

        if (failed) {
            Spacer(Modifier.height(16.dp))
            Text(
                text = stringResource(R.string.error_ticket_failed),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                textAlign = TextAlign.Center,
            )
        }
    }
}
