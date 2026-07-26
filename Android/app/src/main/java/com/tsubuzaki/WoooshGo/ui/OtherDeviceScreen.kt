package com.tsubuzaki.WoooshGo.ui

import android.Manifest
import android.content.pm.PackageManager
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.pairing.PortraitCaptureActivity
import kotlinx.coroutines.flow.SharedFlow

/**
 * Transfers with a device that is not on this network (PROTOCOL.md §9).
 *
 * Sending and receiving are two directions of one job, so they are two tabs of a single
 * screen rather than a question asked before the screen opens. Asking first made the user
 * commit to a direction while looking at nothing; here both are visible and switching
 * costs a tap.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OtherDeviceScreen(
    beginInternetTicket: suspend () -> String,
    endInternetTicket: () -> Unit,
    onStageFiles: (List<Uri>) -> Unit,
    redeemedPeerId: String?,
    onRedeemed: (String) -> Unit,
    onRedeemTicket: (String) -> Unit,
    statusMessages: SharedFlow<String>,
    onBack: () -> Unit,
) {
    var tab by rememberSaveable { mutableIntStateOf(0) }

    // The screen stays put after redeeming, as the pairing screen does. Navigating away on
    // the press would drop whatever the core says next: `statusMessages` has no replay, so
    // a failure emitted with no collector attached is gone.
    val snackbarHostState = remember { SnackbarHostState() }
    LaunchedEffect(Unit) {
        statusMessages.collect { snackbarHostState.showSnackbar(it) }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.other_device_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            PrimaryTabRow(selectedTabIndex = tab) {
                Tab(
                    selected = tab == 0,
                    onClick = { tab = 0 },
                    text = { Text(stringResource(R.string.other_device_send)) },
                )
                Tab(
                    selected = tab == 1,
                    onClick = { tab = 1 },
                    text = { Text(stringResource(R.string.other_device_receive)) },
                )
            }
            // Leaving the Send tab tears its content down, which is what ends a live
            // code: it authorises one transfer and must not outlive the screen showing it.
            when (tab) {
                0 -> SendOverInternetTab(
                    beginInternetTicket = beginInternetTicket,
                    endInternetTicket = endInternetTicket,
                    onStageFiles = onStageFiles,
                    redeemedPeerId = redeemedPeerId,
                    onRedeemed = onRedeemed,
                )

                else -> ReceiveOverInternetTab(onRedeemTicket)
            }
        }
    }
}

/**
 * The receiving half (PROTOCOL.md §9.4): scan the code the sender is showing, and the
 * files follow.
 *
 * Scanning *is* the consent, so the incoming offer never raises a second prompt. Nothing
 * is paired, and no fingerprint is shown: there is no prior relationship to check one
 * against.
 */
@Composable
private fun ReceiveOverInternetTab(onRedeemTicket: (String) -> Unit) {
    val context = LocalContext.current
    var pastedCode by rememberSaveable { mutableStateOf("") }

    val scanLauncher = rememberLauncherForActivityResult(ScanContract()) { result ->
        result.contents?.let(onRedeemTicket)
    }
    val scanPrompt = stringResource(R.string.other_device_scan_prompt)
    val scanOptions = remember(scanPrompt) {
        ScanOptions()
            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            .setPrompt(scanPrompt)
            .setBeepEnabled(false)
            // Our own capture activity: the library's is pinned to landscape.
            .setCaptureActivity(PortraitCaptureActivity::class.java)
            .setOrientationLocked(false)
    }
    val cameraPermissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted -> if (granted) scanLauncher.launch(scanOptions) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            Icons.Outlined.QrCodeScanner,
            contentDescription = null,
            modifier = Modifier.size(56.dp),
            tint = MaterialTheme.colorScheme.primary,
        )
        Spacer(Modifier.height(16.dp))
        Text(
            text = stringResource(R.string.other_device_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(16.dp))
        Button(onClick = {
            val granted = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
            if (granted) {
                scanLauncher.launch(scanOptions)
            } else {
                cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
            }
        }) {
            Text(stringResource(R.string.action_scan_qr))
        }
        Spacer(Modifier.height(32.dp))
        OutlinedTextField(
            value = pastedCode,
            onValueChange = { pastedCode = it },
            label = { Text(stringResource(R.string.pairing_paste_label)) },
            minLines = 2,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(12.dp))
        OutlinedButton(
            onClick = { onRedeemTicket(pastedCode) },
            enabled = pastedCode.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(stringResource(R.string.action_pair_with_pasted))
        }
        Spacer(Modifier.height(24.dp))
        // Hole punching before the reply is slower than the LAN, and silence for that
        // long reads as a hang.
        Text(
            text = stringResource(R.string.other_device_connecting_body),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
