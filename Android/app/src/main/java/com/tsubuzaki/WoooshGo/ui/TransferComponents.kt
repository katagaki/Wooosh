package com.tsubuzaki.WoooshGo.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Cancel
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Upload
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.core.CoreEvent
import com.tsubuzaki.WoooshGo.core.TransferDirection
import com.tsubuzaki.WoooshGo.pairing.PairingManager
import com.tsubuzaki.WoooshGo.transfer.OutgoingOffer
import com.tsubuzaki.WoooshGo.transfer.TransferStatus
import com.tsubuzaki.WoooshGo.transfer.TransferUi
import kotlinx.coroutines.delay

// ---------------------------------------------------------------- incoming offer

/** Consent sheet for an incoming offer (DESIGN.md §5 / PROTOCOL.md §4.4). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun IncomingOfferSheet(
    offer: CoreEvent.IncomingOffer,
    senderIsPaired: Boolean,
    onAccept: () -> Unit,
    onDecline: () -> Unit,
) {
    val context = LocalContext.current
    ModalBottomSheet(onDismissRequest = onDecline) {
        Column(Modifier.padding(horizontal = 24.dp, vertical = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // Device type comes from the sender's HELLO. The core still speaks the
                // old form-factor vocabulary there, so this is usually the neutral glyph
                // — deliberately, rather than a guessed platform (PROTOCOL.md §3.1).
                offer.from.deviceType?.let { type ->
                    Icon(
                        imageVector = type.icon(),
                        contentDescription = type.label(),
                        modifier = Modifier.size(24.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Spacer(Modifier.width(12.dp))
                }
                Text(
                    text = pluralStringResource(
                        R.plurals.offer_title,
                        offer.manifest.size,
                        offer.from.displayName,
                        offer.manifest.size,
                    ),
                    style = MaterialTheme.typography.titleLarge,
                )
            }
            Spacer(Modifier.height(4.dp))
            if (senderIsPaired) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        Icons.Outlined.CheckCircle,
                        contentDescription = null,
                        modifier = Modifier.size(16.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Spacer(Modifier.width(6.dp))
                    Text(
                        text = stringResource(R.string.offer_paired_device),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            } else {
                // The sender shows the same six words on its own screen while it waits
                // for this decision (OutgoingOfferCard) — so this instruction is one the
                // user can actually carry out.
                Text(
                    text = stringResource(R.string.offer_unpaired_warning),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(8.dp))
                VerificationPhrase(
                    phrase = offer.from.fingerprint,
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Spacer(Modifier.height(16.dp))
            Column(
                modifier = Modifier
                    .heightIn(max = 220.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                offer.manifest.forEach { file ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = file.name,
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        Spacer(Modifier.width(12.dp))
                        Text(
                            text = formatBytes(context, file.size),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            Spacer(Modifier.height(8.dp))
            Text(
                text = pluralStringResource(
                    R.plurals.files_count_and_size,
                    offer.manifest.size,
                    offer.manifest.size,
                    formatBytes(context, offer.manifest.sumOf { it.size }),
                ),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
            )

            Spacer(Modifier.height(20.dp))
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 24.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OutlinedButton(onClick = onDecline, modifier = Modifier.weight(1f)) {
                    Text(stringResource(R.string.action_decline))
                }
                Button(onClick = onAccept, modifier = Modifier.weight(1f)) {
                    Text(
                        stringResource(
                            if (senderIsPaired) R.string.action_accept
                            else R.string.action_accept_once
                        )
                    )
                }
            }
        }
    }
}

// ------------------------------------------------------- outgoing offer (send side)

/**
 * Send-side card for the window between OFFER and DECISION (PROTOCOL.md §5).
 *
 * The core only emits `TransferStarted` for a send once the receiver has accepted, so
 * before this card existed the sending device showed *nothing at all* while the other
 * user sat in front of the consent sheet. That sheet tells an unpaired receiver to
 * compare the sender's fingerprint against "the sender's screen" (PROTOCOL.md §4.4) —
 * a comparison that was impossible, because the only place the phrase appeared on the
 * sender was buried in Settings.
 *
 * So: while an **unpaired** peer is deciding, this card shows our own 6-word phrase,
 * big enough to read aloud across a table. For a paired peer no comparison is asked
 * for on the other end and the phrase is suppressed — it would be pure noise.
 *
 * [ownFingerprint] is `core.fingerprintPhrase()` verbatim. The wordlist is the core's;
 * the shell never derives it (DESIGN.md §4).
 */
@Composable
fun OutgoingOfferCard(
    offer: OutgoingOffer,
    ownFingerprint: String?,
    onCancel: () -> Unit,
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp),
    ) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Outlined.Upload,
                    contentDescription = null,
                    modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.primary,
                )
                Spacer(Modifier.width(8.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        text = stringResource(R.string.outgoing_waiting_title, offer.peerName),
                        style = MaterialTheme.typography.titleSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = pluralStringResource(
                            R.plurals.outgoing_waiting_body, offer.fileCount, offer.fileCount,
                        ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(onClick = onCancel) {
                    Icon(
                        Icons.Outlined.Cancel,
                        contentDescription = stringResource(R.string.action_cancel_transfer),
                    )
                }
            }
            Spacer(Modifier.height(10.dp))
            // Indeterminate: nothing is measurable until the receiver answers.
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth())

            if (!offer.peerIsPaired) {
                Spacer(Modifier.height(16.dp))
                Text(
                    text = stringResource(R.string.verify_prompt_peer, offer.peerName),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(8.dp))
                VerificationPhrase(
                    phrase = ownFingerprint ?: stringResource(R.string.settings_starting),
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                    contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }
        }
    }
}

/**
 * The 6-word verification phrase (PROTOCOL.md §2) styled for the ceremony: tonal block,
 * monospace, large and letter-spaced so two people can read it to each other. Same
 * treatment as the SAS code below, scaled for words instead of six digits.
 */
@Composable
fun VerificationPhrase(
    phrase: String,
    containerColor: Color,
    contentColor: Color,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = containerColor,
        contentColor = contentColor,
        shape = MaterialTheme.shapes.medium,
        modifier = modifier.fillMaxWidth(),
    ) {
        Text(
            text = phrase,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.SemiBold,
            fontSize = 20.sp,
            lineHeight = 30.sp,
            letterSpacing = 0.5.sp,
            textAlign = TextAlign.Center,
        )
    }
}

// ---------------------------------------------------------------- SAS

/** SAS numeric-comparison sheet (PROTOCOL.md §4.3): 60 s window, both users compare codes. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SasSheet(
    request: PairingManager.SasRequest,
    onConfirm: (accepted: Boolean) -> Unit,
) {
    var secondsLeft by remember(request) { mutableIntStateOf(SAS_TIMEOUT_SECONDS) }
    LaunchedEffect(request) {
        while (secondsLeft > 0) {
            delay(1000)
            secondsLeft--
        }
        onConfirm(false) // timeout → abort, key NOT stored (PROTOCOL.md §4.3)
    }

    ModalBottomSheet(onDismissRequest = { onConfirm(false) }) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = stringResource(R.string.sas_title, request.peer.displayName),
                style = MaterialTheme.typography.titleLarge,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = stringResource(R.string.sas_body),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(20.dp))
            Text(
                text = "${request.code.take(3)} ${request.code.drop(3)}",
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 44.sp,
                letterSpacing = 4.sp,
            )
            Spacer(Modifier.height(12.dp))
            LinearProgressIndicator(
                progress = { secondsLeft / SAS_TIMEOUT_SECONDS.toFloat() },
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.sas_expires_in, secondsLeft),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(20.dp))
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 24.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OutlinedButton(onClick = { onConfirm(false) }, modifier = Modifier.weight(1f)) {
                    Text(stringResource(R.string.action_cancel))
                }
                Button(onClick = { onConfirm(true) }, modifier = Modifier.weight(1f)) {
                    Text(stringResource(R.string.action_codes_match))
                }
            }
        }
    }
}

private const val SAS_TIMEOUT_SECONDS = 60

// ---------------------------------------------------------------- key changed

/**
 * Prominent KEY_CHANGED warning (PROTOCOL.md §4.5) — never a silent re-pin.
 *
 * Both phrases are shown now: the core hands over the expected and the presented key,
 * and `fingerprint_phrase_for` turns each into the same 6 words the other device shows
 * in its own settings. That turns "something changed" into something the user can
 * actually check by reading words aloud.
 */
@Composable
fun KeyChangedDialog(
    alert: PairingManager.KeyChangedAlert,
    onRepair: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = {
            Icon(
                Icons.Outlined.Warning,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.error,
            )
        },
        title = { Text(stringResource(R.string.keychanged_title)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(stringResource(R.string.keychanged_body, alert.peer.displayName))
                Spacer(Modifier.height(16.dp))
                FingerprintRow(
                    label = stringResource(R.string.keychanged_expected_label),
                    // The phrase is a shared verification artifact: it has to
                    // read identically on both devices, so it is never
                    // translated and never reformatted.
                    phrase = alert.expectedFingerprint,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Spacer(Modifier.height(8.dp))
                FingerprintRow(
                    label = stringResource(R.string.keychanged_presented_label),
                    phrase = alert.presentedFingerprint
                        ?: stringResource(R.string.keychanged_presented_unknown),
                    color = MaterialTheme.colorScheme.error,
                )
                Spacer(Modifier.height(12.dp))
                Text(
                    text = stringResource(R.string.keychanged_advice),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            Button(onClick = onRepair) { Text(stringResource(R.string.action_pair_again)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_not_now)) }
        },
    )
}

@Composable
private fun FingerprintRow(label: String, phrase: String, color: androidx.compose.ui.graphics.Color) {
    Column {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = phrase,
            style = MaterialTheme.typography.bodyMedium,
            fontFamily = FontFamily.Monospace,
            color = color,
        )
    }
}

// ---------------------------------------------------------------- transfer card

@Composable
fun TransferCard(
    transfer: TransferUi,
    onCancel: () -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp),
    ) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = if (transfer.direction == TransferDirection.SEND) {
                        Icons.Outlined.Upload
                    } else {
                        Icons.Outlined.Download
                    },
                    contentDescription = null,
                    modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.primary,
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    text = when (transfer.status) {
                        TransferStatus.RUNNING -> stringResource(
                            if (transfer.direction == TransferDirection.SEND) {
                                R.string.transfer_sending_to
                            } else {
                                R.string.transfer_receiving_from
                            },
                            transfer.peerName,
                        )

                        TransferStatus.DONE ->
                            transfer.message ?: stringResource(R.string.transfer_state_done)

                        TransferStatus.FAILED ->
                            transfer.message ?: stringResource(R.string.error_transfer_failed)
                    },
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                    color = if (transfer.status == TransferStatus.FAILED) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.onSurface
                    },
                )
                when (transfer.status) {
                    TransferStatus.RUNNING -> IconButton(onClick = onCancel) {
                        Icon(
                            Icons.Outlined.Cancel,
                            contentDescription = stringResource(R.string.action_cancel_transfer),
                        )
                    }

                    else -> IconButton(onClick = onDismiss) {
                        Icon(
                            Icons.Outlined.Close,
                            contentDescription = stringResource(R.string.action_dismiss),
                        )
                    }
                }
            }

            if (transfer.status == TransferStatus.RUNNING) {
                val total = transfer.totalBytes
                LinearProgressIndicator(
                    progress = {
                        if (total > 0) (transfer.transferredBytes.toFloat() / total).coerceIn(0f, 1f) else 0f
                    },
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(6.dp))
                // Three self-contained measurements joined by a separator.
                // Each is a whole format string of its own, so nothing here
                // asks a translator to reorder half a sentence.
                val parts = buildList {
                    add(
                        stringResource(
                            R.string.transfer_progress_bytes,
                            formatBytes(context, transfer.transferredBytes),
                            formatBytes(context, total),
                        )
                    )
                    if (transfer.rate > 0) add(formatRate(context, transfer.rate))
                    formatEta(context, transfer.etaSeconds).takeIf { it.isNotEmpty() }?.let(::add)
                }
                Text(
                    text = parts.joinToString(" · "),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(10.dp))
            } else {
                Spacer(Modifier.height(4.dp))
            }

            transfer.files.forEach { fileState ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 3.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = fileState.meta.name,
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Spacer(Modifier.width(12.dp))
                    when {
                        fileState.error != null -> Icon(
                            Icons.Outlined.ErrorOutline,
                            contentDescription = fileState.error,
                            modifier = Modifier.size(16.dp),
                            tint = MaterialTheme.colorScheme.error,
                        )

                        fileState.bytes >= fileState.meta.size &&
                            (transfer.direction == TransferDirection.SEND || fileState.routed) -> Icon(
                            Icons.Outlined.CheckCircle,
                            contentDescription = stringResource(R.string.transfer_state_done),
                            modifier = Modifier.size(16.dp),
                            tint = MaterialTheme.colorScheme.primary,
                        )

                        else -> {
                            val fileProgress = if (fileState.meta.size > 0) {
                                (fileState.bytes.toFloat() / fileState.meta.size).coerceIn(0f, 1f)
                            } else 0f
                            LinearProgressIndicator(
                                progress = { fileProgress },
                                modifier = Modifier.width(72.dp),
                            )
                        }
                    }
                }
                fileState.error?.let { error ->
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        }
    }
}
