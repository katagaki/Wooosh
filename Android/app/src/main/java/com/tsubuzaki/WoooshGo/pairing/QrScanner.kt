package com.tsubuzaki.WoooshGo.pairing

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Rect
import android.hardware.Camera
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.border
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.google.zxing.DecodeHintType
import com.journeyapps.barcodescanner.BarcodeCallback
import com.journeyapps.barcodescanner.BarcodeResult
import com.journeyapps.barcodescanner.BarcodeView
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import com.journeyapps.barcodescanner.DefaultDecoderFactory
import com.tsubuzaki.WoooshGo.R
import kotlinx.coroutines.delay
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.roundToInt

/**
 * Composed in place, not via ZXing's `CaptureActivity`, which leaves no UI surface between
 * detection and finishing. [enabled] re-arms the scanner after a failed code.
 */
@Composable
fun QrScanner(
    onScanned: (String) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
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

    // Asked on arrival: the whole tab is the scanner, so there is nothing else to do here.
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

    // One payload per arming: decoding is continuous and the callback also fires during
    // teardown, and a second delivery starts a second ceremony.
    val delivered = remember { AtomicBoolean(false) }

    var detected by remember { mutableStateOf(false) }
    var focusPoint by remember { mutableStateOf<Offset?>(null) }
    val haptics = LocalHapticFeedback.current

    val scanner = remember {
        DecoratedBarcodeView(context).apply {
            setStatusText("")
            barcodeView.decoderFactory = DefaultDecoderFactory(
                listOf(BarcodeFormat.QR_CODE),
                // A fixed square held over one code, so CPU per frame is the cheap trade.
                mapOf(DecodeHintType.TRY_HARDER to true),
                null,
                0,
            )
        }
    }

    // Continuous, not `decodeSingle`: a single decode clears its own callback on the first
    // hit, leaving a dead camera after a failed code. Continuous survives pause/resume.
    DisposableEffect(scanner) {
        scanner.decodeContinuous(object : BarcodeCallback {
            override fun barcodeResult(result: BarcodeResult) {
                if (!delivered.compareAndSet(false, true)) return
                detected = true
                haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                scanner.pause()
                onScannedNow(result.text)
            }
        })
        onDispose { scanner.pause() }
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, enabled) {
        // Camera ownership follows the lifecycle and [enabled], so backgrounding releases
        // it. `resume()` is safe to call repeatedly and restarts the decoder.
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> if (enabled) scanner.resume()
                Lifecycle.Event.ON_PAUSE -> scanner.pause()
                else -> Unit
            }
        }
        if (enabled) {
            delivered.set(false)
            detected = false
            scanner.resume()
        } else {
            scanner.pause()
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            scanner.pause()
        }
    }

    LaunchedEffect(focusPoint) {
        if (focusPoint != null) {
            delay(FOCUS_INDICATOR_MS)
            focusPoint = null
        }
    }

    Box(
        modifier = modifier.pointerInput(scanner) {
            detectTapGestures { offset ->
                focusPoint = offset
                focusAt(
                    view = scanner.barcodeView,
                    nx = (offset.x / size.width).coerceIn(0f, 1f),
                    ny = (offset.y / size.height).coerceIn(0f, 1f),
                )
            }
        },
    ) {
        AndroidView(
            factory = { scanner },
            modifier = Modifier.fillMaxSize(),
        )

        focusPoint?.let { point ->
            val side = FOCUS_INDICATOR_SIDE
            Box(
                modifier = Modifier
                    .offset {
                        val half = side.toPx() / 2f
                        IntOffset(
                            (point.x - half).roundToInt(),
                            (point.y - half).roundToInt(),
                        )
                    }
                    .size(side)
                    .border(1.5.dp, Color.Yellow, RoundedCornerShape(8.dp)),
            )
        }

        // Downstream is slow enough that a frozen viewfinder reads as a jammed scanner.
        if (detected) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.6f)),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                CircularProgressIndicator(color = Color.White)
                Spacer(Modifier.height(12.dp))
                Text(
                    text = stringResource(R.string.pairing_code_detected),
                    style = MaterialTheme.typography.titleSmall,
                    color = Color.White,
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}

/**
 * Only the areas are set: `AutoFocusManager` decides at preview start whether to drive
 * focus passes, so setting `FOCUS_MODE_AUTO` here can leave nobody calling `autoFocus()`.
 */
@Suppress("DEPRECATION")
private fun focusAt(view: BarcodeView, nx: Float, ny: Float) {
    if (!view.isPreviewActive) return
    val rotation = view.cameraInstance?.cameraRotation ?: return
    if (rotation < 0) return

    // Focus areas live in the sensor's frame, untouched by `setDisplayOrientation`.
    val (sx, sy) = when (rotation) {
        90 -> ny to (1f - nx)
        180 -> (1f - nx) to (1f - ny)
        270 -> (1f - ny) to nx
        else -> nx to ny
    }

    fun bound(centre: Float, delta: Float) =
        ((centre + delta) * 2000f - 1000f).coerceIn(-1000f, 1000f).roundToInt()

    val rect = Rect(
        bound(sx, -FOCUS_AREA_HALF),
        bound(sy, -FOCUS_AREA_HALF),
        bound(sx, FOCUS_AREA_HALF),
        bound(sy, FOCUS_AREA_HALF),
    )
    // The camera HAL rejects a degenerate rectangle, which clamping at an edge can produce.
    if (rect.width() <= 0 || rect.height() <= 0) return

    view.changeCameraParameters { parameters ->
        val areas = listOf(Camera.Area(rect, 1000))
        if (parameters.maxNumFocusAreas > 0) parameters.focusAreas = areas
        if (parameters.maxNumMeteringAreas > 0) parameters.meteringAreas = areas
        parameters
    }
}

/** Fraction of the frame. */
private const val FOCUS_AREA_HALF = 0.1f

private const val FOCUS_INDICATOR_MS = 700L

private val FOCUS_INDICATOR_SIDE = 68.dp
