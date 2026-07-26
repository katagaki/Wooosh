package com.tsubuzaki.WoooshGo.pairing

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.journeyapps.barcodescanner.BarcodeCallback
import com.journeyapps.barcodescanner.BarcodeResult
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import com.journeyapps.barcodescanner.DefaultDecoderFactory
import com.tsubuzaki.WoooshGo.R
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Live QR scanner, composed **inside** the screen that owns the ceremony.
 *
 * Deliberately not ZXing's `CaptureActivity`: a separate activity gives Wooosh no UI
 * surface at all between "code detected" and "activity finished", so the progress and
 * failure alerts could not appear until after the scanner had closed and the main
 * activity had resumed. Anything slow in that window — and on the internet path that is
 * a network dial — left the user staring at a camera that had already stopped scanning.
 * Composed here, detection happens with the alert host already on screen.
 *
 * Reads either code type. A pairing code and an internet ticket look identical to a
 * camera, so the caller dispatches on the scheme (PROTOCOL.md §4.2 / §9.2).
 */
@Composable
fun QrScanner(
    onScanned: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        )
    }
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted = it }

    // Asked on arrival rather than behind a button: the whole tab is the scanner, so
    // there is nothing else here to do while the question goes unanswered.
    LaunchedEffect(Unit) {
        if (!granted) permissionLauncher.launch(Manifest.permission.CAMERA)
    }

    if (!granted) {
        Column(
            modifier = modifier,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = stringResource(R.string.camera_permission_body),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(12.dp))
            Button(onClick = { permissionLauncher.launch(Manifest.permission.CAMERA) }) {
                Text(stringResource(R.string.action_allow_camera))
            }
        }
        return
    }

    val onScannedNow by rememberUpdatedState(onScanned)

    // One payload per scanner, whatever the camera does next. `decodeSingle` already
    // stops after a hit, but the callback can be reached again while the view is being
    // torn down, and a second delivery would start a second ceremony.
    val delivered = remember { AtomicBoolean(false) }

    val scanner = remember {
        DecoratedBarcodeView(context).apply {
            // The library's own status line duplicates the caller's instructions.
            setStatusText("")
            barcodeView.decoderFactory = DefaultDecoderFactory(listOf(BarcodeFormat.QR_CODE))
        }
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        // Camera ownership follows the host's lifecycle: held only while this is the
        // visible screen, so backgrounding Wooosh releases it. Adding the observer to an
        // already-resumed lifecycle dispatches ON_RESUME immediately, which is what
        // starts the preview — hence no explicit resume() here.
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> scanner.resume()
                Lifecycle.Event.ON_PAUSE -> scanner.pause()
                else -> Unit
            }
        }
        scanner.decodeSingle(object : BarcodeCallback {
            override fun barcodeResult(result: BarcodeResult) {
                if (!delivered.compareAndSet(false, true)) return
                // Stop the camera before handing over: the ceremony owns the screen from
                // here, and a live preview under a modal alert is just a battery drain.
                scanner.pause()
                onScannedNow(result.text)
            }
        })
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            scanner.pause()
        }
    }

    Box(modifier = modifier) {
        AndroidView(
            factory = { scanner },
            modifier = Modifier.fillMaxSize(),
        )
    }
}
