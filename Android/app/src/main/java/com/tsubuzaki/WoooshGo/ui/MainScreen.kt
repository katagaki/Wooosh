package com.tsubuzaki.WoooshGo.ui

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.PhotoLibrary
import androidx.compose.material.icons.outlined.QrCode2
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.WifiTethering
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.core.TransferId
import com.tsubuzaki.WoooshGo.peers.Peer
import com.tsubuzaki.WoooshGo.settings.Visibility
import com.tsubuzaki.WoooshGo.share.OutboxRepository
import com.tsubuzaki.WoooshGo.transfer.OutgoingOffer
import com.tsubuzaki.WoooshGo.transfer.TransferUi
import kotlinx.coroutines.flow.SharedFlow

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen(
    peers: List<Peer>,
    /**
     * DeviceIDs of the core's pinned peers. A row can only be matched against this once
     * it has a DeviceID of its own — i.e. after a connection — because the mDNS TXT
     * carries a rotating rid, not an identity (PROTOCOL.md §3.1). Matching on the
     * display name, as this screen used to, would put a "Paired" checkmark on any device
     * that happened to share a name with a paired one.
     */
    pairedDeviceIds: Set<String>,
    visibility: Visibility?,
    transfers: List<TransferUi>,
    /** Offers on the wire whose receiver has not answered yet (PROTOCOL.md §4.4). */
    outgoingOffers: List<OutgoingOffer>,
    /** This device's own 6-word phrase, straight from `core.fingerprintPhrase()`. */
    ownFingerprint: String?,
    stagedShare: OutboxRepository.StagedShare?,
    statusMessages: SharedFlow<String>,
    onSendFiles: (Peer, List<Uri>) -> Unit,
    onSendStaged: (Peer) -> Unit,
    onDismissStaged: () -> Unit,
    onCancelTransfer: (TransferId) -> Unit,
    onDismissTransfer: (TransferId) -> Unit,
    onRefresh: () -> Unit,
    onOpenSettings: () -> Unit,
    onOpenPairing: () -> Unit,
) {
    val snackbarHostState = remember { SnackbarHostState() }
    LaunchedEffect(Unit) {
        statusMessages.collect { snackbarHostState.showSnackbar(it) }
    }

    // Peer awaiting a "photos or documents?" choice, then a system picker result.
    var pickerPeer by remember { mutableStateOf<Peer?>(null) }
    var launchTarget by remember { mutableStateOf<Peer?>(null) }

    val mediaPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.PickMultipleVisualMedia()
    ) { uris ->
        launchTarget?.let { peer -> if (uris.isNotEmpty()) onSendFiles(peer, uris) }
        launchTarget = null
    }
    val documentPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments()
    ) { uris ->
        launchTarget?.let { peer -> if (uris.isNotEmpty()) onSendFiles(peer, uris) }
        launchTarget = null
    }

    // Direct Share arrival: auto-send once the targeted paired device is alive in the list.
    // Prefer an identity match; fall back to the display name only because a freshly
    // discovered row has no DeviceID until something connects to it.
    LaunchedEffect(stagedShare, peers) {
        val share = stagedShare ?: return@LaunchedEffect
        val alive = peers.filterNot { it.isStale }
        val target = share.targetDeviceId
            ?.let { id -> alive.firstOrNull { it.peerId == id } }
            ?: share.targetDisplayName?.let { name -> alive.firstOrNull { it.displayName == name } }
        target?.let(onSendStaged)
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.app_name)) },
                actions = {
                    IconButton(onClick = onOpenPairing) {
                        Icon(
                            Icons.Outlined.QrCode2,
                            contentDescription = stringResource(R.string.action_pair_device),
                        )
                    }
                    IconButton(onClick = onRefresh) {
                        Icon(
                            Icons.Outlined.Refresh,
                            contentDescription = stringResource(R.string.action_refresh),
                        )
                    }
                    IconButton(onClick = onOpenSettings) {
                        Icon(
                            Icons.Outlined.Settings,
                            contentDescription = stringResource(R.string.action_settings),
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        if (peers.isEmpty() && transfers.isEmpty() && outgoingOffers.isEmpty() && stagedShare == null) {
            EmptyState(
                visibility = visibility,
                onOpenPairing = onOpenPairing,
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
            )
        } else {
            // Order comes straight from the registry: append-only by first sighting.
            // Deliberately no sorting and no reorder animations here (DESIGN.md §5).
            LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
            ) {
                if (stagedShare != null) {
                    item(key = "staged-share") {
                        StagedShareBanner(
                            share = stagedShare,
                            onDismiss = onDismissStaged,
                        )
                    }
                }
                // Above the transfer cards: the verification window. The receiver is
                // looking at its consent sheet right now, so an unpaired send has to be
                // showing our fingerprint here (PROTOCOL.md §4.4).
                items(outgoingOffers, key = { "offer-${it.transferId}" }) { offer ->
                    OutgoingOfferCard(
                        offer = offer,
                        ownFingerprint = ownFingerprint,
                        onCancel = { onCancelTransfer(offer.transferId) },
                    )
                }
                items(transfers, key = { "transfer-${it.id}" }) { transfer ->
                    TransferCard(
                        transfer = transfer,
                        onCancel = { onCancelTransfer(transfer.id) },
                        onDismiss = { onDismissTransfer(transfer.id) },
                    )
                }
                items(peers, key = { it.rid }) { peer ->
                    PeerRow(
                        peer = peer,
                        isPaired = peer.peerId != null && peer.peerId in pairedDeviceIds,
                        armedToSend = stagedShare != null,
                        onClick = {
                            if (stagedShare != null) {
                                onSendStaged(peer)
                            } else {
                                pickerPeer = peer
                            }
                        },
                    )
                }
            }
        }
    }

    pickerPeer?.let { peer ->
        AlertDialog(
            onDismissRequest = { pickerPeer = null },
            title = { Text(stringResource(R.string.transfer_send_to_title, peer.displayName)) },
            text = {
                Column {
                    PickerOption(
                        icon = Icons.Outlined.PhotoLibrary,
                        label = stringResource(R.string.action_photos_and_videos),
                        onClick = {
                            pickerPeer = null
                            launchTarget = peer
                            mediaPicker.launch(
                                PickVisualMediaRequest(
                                    ActivityResultContracts.PickVisualMedia.ImageAndVideo
                                )
                            )
                        },
                    )
                    PickerOption(
                        icon = Icons.AutoMirrored.Outlined.InsertDriveFile,
                        label = stringResource(R.string.action_files),
                        onClick = {
                            pickerPeer = null
                            launchTarget = peer
                            documentPicker.launch(arrayOf("*/*"))
                        },
                    )
                }
            },
            confirmButton = {},
            dismissButton = {
                TextButton(onClick = { pickerPeer = null }) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }
}

@Composable
private fun PickerOption(icon: ImageVector, label: String, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 12.dp),
    ) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.width(16.dp))
        Text(label, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun StagedShareBanner(
    share: OutboxRepository.StagedShare,
    onDismiss: () -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer,
        ),
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp, end = 4.dp),
        ) {
            Icon(
                Icons.AutoMirrored.Outlined.Send,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimaryContainer,
            )
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    text = pluralStringResource(
                        R.plurals.share_batch_ready, share.uris.size, share.uris.size,
                    ),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
                Text(
                    text = share.targetDisplayName
                        ?.let { stringResource(R.string.share_batch_waiting, it) }
                        ?: stringResource(R.string.share_batch_tap_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }
            IconButton(onClick = onDismiss) {
                Icon(
                    Icons.Outlined.Close,
                    contentDescription = stringResource(R.string.action_discard_shared),
                    tint = MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }
        }
    }
}

@Composable
private fun PeerRow(
    peer: Peer,
    isPaired: Boolean,
    armedToSend: Boolean,
    onClick: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            // Stale rows stay in place: grayed out and not clickable, never removed.
            .alpha(if (peer.isStale) 0.38f else 1f)
            .clickable(enabled = !peer.isStale, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Icon(
            imageVector = peer.deviceType.icon(),
            // The glyph is now the only place the platform is stated, so name it.
            contentDescription = peer.deviceType.label(),
            modifier = Modifier.size(28.dp),
            tint = MaterialTheme.colorScheme.primary,
        )
        Spacer(Modifier.width(16.dp))
        Column(Modifier.weight(1f)) {
            Text(
                text = peer.displayName,
                style = MaterialTheme.typography.bodyLarge,
            )
            Text(
                    text = stringResource(
                    when {
                        peer.isStale -> R.string.peer_state_away
                        armedToSend -> R.string.peer_state_tap_to_send
                        isPaired -> R.string.peer_state_ready_paired
                        else -> R.string.peer_state_ready
                    }
                ),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (isPaired) {
            // Checkmark on the row, never a separate section (DESIGN.md §5).
            Icon(
                Icons.Outlined.CheckCircle,
                contentDescription = stringResource(R.string.peer_badge_paired),
                modifier = Modifier.size(20.dp),
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

@Composable
private fun EmptyState(
    visibility: Visibility?,
    onOpenPairing: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier.padding(32.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Icon(
                imageVector = Icons.Outlined.WifiTethering,
                contentDescription = null,
                modifier = Modifier.size(56.dp),
                tint = MaterialTheme.colorScheme.primary,
            )
            Spacer(Modifier.height(16.dp))
            Text(
                text = stringResource(R.string.empty_title),
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = stringResource(R.string.empty_body),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = when (visibility) {
                    Visibility.EVERYONE -> stringResource(R.string.empty_visibility_everyone)
                    Visibility.PAIRED_ONLY -> stringResource(R.string.empty_visibility_paired)
                    Visibility.OFF -> stringResource(R.string.empty_visibility_off)
                    null -> ""
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(24.dp))
            OutlinedButton(onClick = onOpenPairing) {
                Icon(Icons.Outlined.QrCode2, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.action_pair_device))
            }
        }
    }
}
