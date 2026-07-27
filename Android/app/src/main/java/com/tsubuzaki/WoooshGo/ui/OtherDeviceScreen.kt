package com.tsubuzaki.WoooshGo.ui

import android.net.Uri
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.pairing.QrScanner
import kotlinx.coroutines.flow.SharedFlow

/** Transfers with a device that is not on this network (PROTOCOL.md §9). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OtherDeviceScreen(
    beginInternetTicket: suspend () -> String,
    endInternetTicket: () -> Unit,
    onStageFiles: (List<Uri>) -> Unit,
    redeemedPeerId: String?,
    onRedeemed: (String) -> Unit,
    onRedeemTicket: (String) -> Unit,
    /** False while a redemption is in flight; re-arms the camera once it has been dismissed. */
    scannerEnabled: Boolean,
    statusMessages: SharedFlow<String>,
    onBack: () -> Unit,
) {
    var tab by rememberSaveable { mutableIntStateOf(0) }

    // `statusMessages` has no replay: a failure emitted with no collector attached is gone.
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
            // Leaving the Send tab tears it down, which is what ends the live code.
            when (tab) {
                0 -> SendOverInternetTab(
                    beginInternetTicket = beginInternetTicket,
                    endInternetTicket = endInternetTicket,
                    onStageFiles = onStageFiles,
                    redeemedPeerId = redeemedPeerId,
                    onRedeemed = onRedeemed,
                )

                else -> ReceiveOverInternetTab(onRedeemTicket, scannerEnabled)
            }
        }
    }
}

/** Scanning *is* the consent: no second prompt, nothing paired, no fingerprint to check. */
@Composable
private fun ReceiveOverInternetTab(onRedeemTicket: (String) -> Unit, scannerEnabled: Boolean) {
    var pastedCode by rememberSaveable { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // Scanning inside this tree lets the alert appear the moment the code is read.
        QrScanner(
            onScanned = onRedeemTicket,
            enabled = scannerEnabled,
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(1f)
                .clip(RoundedCornerShape(16.dp)),
        )
        Spacer(Modifier.height(16.dp))
        Text(
            text = stringResource(R.string.other_device_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
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
        Text(
            text = stringResource(R.string.other_device_connecting_body),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
