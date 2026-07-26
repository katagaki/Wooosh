package com.tsubuzaki.WoooshGo

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.tsubuzaki.WoooshGo.ui.IncomingOfferSheet
import com.tsubuzaki.WoooshGo.ui.KeyChangedDialog
import com.tsubuzaki.WoooshGo.ui.MainScreen
import com.tsubuzaki.WoooshGo.ui.MainViewModel
import com.tsubuzaki.WoooshGo.ui.PairingProgressDialog
import com.tsubuzaki.WoooshGo.ui.PairingScreen
import com.tsubuzaki.WoooshGo.ui.SasSheet
import com.tsubuzaki.WoooshGo.ui.OtherDeviceScreen
import com.tsubuzaki.WoooshGo.ui.SettingsScreen
import com.tsubuzaki.WoooshGo.ui.theme.WoooshTheme

class MainActivity : ComponentActivity() {

    private val viewModel: MainViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            WoooshTheme {
                AppRoot(viewModel)
            }
        }
    }
}

private enum class Screen { MAIN, SETTINGS, PAIRING, OTHER_DEVICE }

// Spelled out rather than using Manifest.permission.ACCESS_LOCAL_NETWORK so the
// build does not depend on the constant being un-gated in the compile SDK.
private const val LOCAL_NETWORK_PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

@Composable
private fun AppRoot(viewModel: MainViewModel) {
    val peers by viewModel.peers.collectAsStateWithLifecycle()
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val deviceId by viewModel.deviceIdFormatted.collectAsStateWithLifecycle()
    val fingerprintPhrase by viewModel.fingerprintPhrase.collectAsStateWithLifecycle()
    val transfers by viewModel.transfers.collectAsStateWithLifecycle()
    val hasActiveTransfers by viewModel.hasActiveTransfers.collectAsStateWithLifecycle()
    val pendingOffer by viewModel.pendingOffer.collectAsStateWithLifecycle()
    val outgoingOffers by viewModel.outgoingOffers.collectAsStateWithLifecycle()
    val pendingSas by viewModel.pendingSas.collectAsStateWithLifecycle()
    val keyChanged by viewModel.keyChanged.collectAsStateWithLifecycle()
    val pairingAttempt by viewModel.pairingAttempt.collectAsStateWithLifecycle()
    val pairedDevices by viewModel.pairedDevices.collectAsStateWithLifecycle()
    val pairedDeviceIds by viewModel.pairedDeviceIds.collectAsStateWithLifecycle()
    val stagedShare by viewModel.stagedShare.collectAsStateWithLifecycle()
    val relayError by viewModel.relayError.collectAsStateWithLifecycle()
    val ticketRedeemedPeerId by viewModel.ticketRedeemedPeerId.collectAsStateWithLifecycle()

    var screen by rememberSaveable { mutableStateOf(Screen.MAIN) }

    // Screen stays awake while a transfer is active (DESIGN.md §7), cleared after.
    val context = LocalContext.current
    DisposableEffect(hasActiveTransfers) {
        val window = (context as? Activity)?.window
        if (hasActiveTransfers) {
            window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        onDispose { window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
    }

    // Asked just before a transfer starts: POST_NOTIFICATIONS (API 33+) for the
    // foreground-service notification, WRITE_EXTERNAL_STORAGE (API 26–28) for the
    // direct-to-Downloads receive path. The transfer proceeds whatever the user answers.
    var pendingPermissionAction by remember { mutableStateOf<(() -> Unit)?>(null) }
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) {
        pendingPermissionAction?.invoke()
        pendingPermissionAction = null
    }

    // ACCESS_LOCAL_NETWORK gates mDNS itself on SDK 37+, so it has to be asked
    // for at launch rather than at transfer time: without it NsdManager throws
    // and the device list stays empty forever.
    val localNetworkLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted -> if (granted) viewModel.refresh() }
    LaunchedEffect(Unit) {
        if (Build.VERSION.SDK_INT >= 37 &&
            ContextCompat.checkSelfPermission(context, LOCAL_NETWORK_PERMISSION) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            localNetworkLauncher.launch(LOCAL_NETWORK_PERMISSION)
        }
    }

    fun withTransferPermissions(includeStorage: Boolean, action: () -> Unit) {
        val needed = buildList {
            if (Build.VERSION.SDK_INT >= 33 &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
                PackageManager.PERMISSION_GRANTED
            ) {
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
            if (includeStorage && Build.VERSION.SDK_INT < 29 &&
                ContextCompat.checkSelfPermission(
                        context, Manifest.permission.WRITE_EXTERNAL_STORAGE
                    ) != PackageManager.PERMISSION_GRANTED
            ) {
                add(Manifest.permission.WRITE_EXTERNAL_STORAGE)
            }
        }
        if (needed.isEmpty()) {
            action()
        } else {
            pendingPermissionAction = action
            permissionLauncher.launch(needed.toTypedArray())
        }
    }

    val currentSettings = settings
    when {
        screen == Screen.SETTINGS && currentSettings != null -> {
            BackHandler { screen = Screen.MAIN }
            SettingsScreen(
                settings = currentSettings,
                deviceIdFormatted = deviceId,
                fingerprintPhrase = fingerprintPhrase,
                pairedDevices = pairedDevices,
                onRevokeDevice = viewModel::revokeDevice,
                onDisplayNameChange = viewModel::setDisplayName,
                onVisibilityChange = viewModel::setVisibility,
                onRelayModeChange = viewModel::setRelayMode,
                onRelayUrlChange = viewModel::setRelayUrl,
                relayError = relayError,
                onBack = { screen = Screen.MAIN },
            )
        }

        screen == Screen.OTHER_DEVICE -> {
            BackHandler { screen = Screen.MAIN }
            OtherDeviceScreen(
                beginInternetTicket = viewModel::beginInternetTicket,
                endInternetTicket = viewModel::endInternetTicket,
                onStageFiles = viewModel::stageInternetSend,
                redeemedPeerId = ticketRedeemedPeerId,
                onRedeemed = viewModel::completeInternetSend,
                onRedeemTicket = viewModel::pairWithQr,
                statusMessages = viewModel.statusMessages,
                onBack = { screen = Screen.MAIN },
            )
        }

        screen == Screen.PAIRING -> {
            BackHandler { screen = Screen.MAIN }
            PairingScreen(
                beginPairingQr = viewModel::beginPairingQr,
                onPairWithPayload = viewModel::pairWithQr,
                statusMessages = viewModel.statusMessages,
                onBack = { screen = Screen.MAIN },
            )
        }

        else -> {
            MainScreen(
                peers = peers,
                pairedDeviceIds = pairedDeviceIds,
                visibility = currentSettings?.visibility,
                transfers = transfers,
                outgoingOffers = outgoingOffers,
                ownFingerprint = fingerprintPhrase,
                stagedShare = stagedShare,
                statusMessages = viewModel.statusMessages,
                onSendFiles = { peer, uris ->
                    withTransferPermissions(includeStorage = false) {
                        viewModel.sendToPeer(peer, uris)
                    }
                },
                internetEnabled = settings?.internetEnabled != false,
                onOtherDevice = { screen = Screen.OTHER_DEVICE },
                onSendStaged = { peer ->
                    withTransferPermissions(includeStorage = false) {
                        viewModel.sendStaged(peer)
                    }
                },
                onDismissStaged = viewModel::dismissStaged,
                onCancelTransfer = viewModel::cancelTransfer,
                onDismissTransfer = viewModel::dismissTransfer,
                onRefresh = viewModel::refresh,
                onOpenSettings = { screen = Screen.SETTINGS },
                onOpenPairing = { screen = Screen.PAIRING },
            )
        }
    }

    pendingOffer?.let { offer ->
        IncomingOfferSheet(
            offer = offer,
            // `trusted` is the core's own answer; the trust list is the same fact seen
            // again, keyed by DeviceID.
            senderIsPaired = offer.from.paired ||
                pairedDevices.any { it.deviceId == offer.from.id },
            onAccept = {
                withTransferPermissions(includeStorage = true) { viewModel.acceptOffer() }
            },
            onDecline = viewModel::declineOffer,
        )
    }

    pendingSas?.let { request ->
        SasSheet(
            request = request,
            onConfirm = viewModel::confirmSas,
        )
    }

    // Covers the whole window from "scanned" to "outcome known", on any screen.
    pairingAttempt?.let { attempt ->
        PairingProgressDialog(
            attempt = attempt,
            onCancel = viewModel::cancelPairingAttempt,
            onDismiss = viewModel::dismissPairingAttempt,
        )
    }

    keyChanged?.let { alert ->
        KeyChangedDialog(
            alert = alert,
            onRepair = {
                viewModel.revokeForRepair()
                screen = Screen.PAIRING
            },
            onDismiss = viewModel::dismissKeyChanged,
        )
    }
}
