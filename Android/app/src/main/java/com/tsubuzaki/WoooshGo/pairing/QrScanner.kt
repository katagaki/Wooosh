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
 *
 * [enabled] is the caller's "I am not busy with a code" signal: false while a ceremony is
 * in flight, true again once it has been settled and dismissed. It is what re-arms the
 * scanner, so a failed or expired code leaves a live camera rather than a frozen one.
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

    // One payload per arming, whatever the camera does next. Decoding is continuous, so
    // the same code is read several times a second, and the callback can be reached again
    // while the view is being torn down; either way a second delivery would start a
    // second ceremony.
    val delivered = remember { AtomicBoolean(false) }

    /** Set the instant a code is read, so feedback does not wait on the caller. */
    var detected by remember { mutableStateOf(false) }
    var focusPoint by remember { mutableStateOf<Offset?>(null) }
    val haptics = LocalHapticFeedback.current

    val scanner = remember {
        DecoratedBarcodeView(context).apply {
            // The library's own status line duplicates the caller's instructions.
            setStatusText("")
            barcodeView.decoderFactory = DefaultDecoderFactory(
                listOf(BarcodeFormat.QR_CODE),
                // The viewfinder is a fixed square held over a single code, so paying more
                // CPU per frame is the right trade against a code that the cheap pass keeps
                // missing — which is what a scanner that "does nothing" actually is.
                mapOf(DecodeHintType.TRY_HARDER to true),
                null,
                0,
            )
        }
    }

    // Continuous, not `decodeSingle`: a single decode clears its own callback and mode on
    // the first hit, so nothing short of re-registering could ever make the view scan
    // again — a failed code left a dead camera. Continuous decoding survives pause/resume,
    // which makes [enabled] the only thing that has to be right.
    DisposableEffect(scanner) {
        scanner.decodeContinuous(object : BarcodeCallback {
            override fun barcodeResult(result: BarcodeResult) {
                if (!delivered.compareAndSet(false, true)) return
                detected = true
                haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                // Stop the camera before handing over: the ceremony owns the screen from
                // here, and a live preview under a modal alert is just a battery drain.
                scanner.pause()
                onScannedNow(result.text)
            }
        })
        onDispose { scanner.pause() }
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, enabled) {
        // Camera ownership follows the host's lifecycle *and* [enabled]: held only while
        // this is the visible screen with no code in flight, so backgrounding Wooosh
        // releases it. `resume()` is documented as safe to call repeatedly, and it
        // restarts the decoder thread by itself.
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> if (enabled) scanner.resume()
                Lifecycle.Event.ON_PAUSE -> scanner.pause()
                else -> Unit
            }
        }
        if (enabled) {
            // Re-arming: the caller has finished with the previous code, so forget it.
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

        // Detection has to read as instant. Everything downstream — parsing the payload,
        // dialling the peer, raising the progress dialog — takes long enough that a
        // viewfinder which merely froze looked like the scanner had jammed.
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
 * Tap to focus, which the on-screen hint has always promised. A pairing QR is small and
 * held close, which is exactly where a whole-frame autofocus hunts and never settles.
 *
 * Only the areas are set. Switching to `FOCUS_MODE_AUTO` here would be worse than doing
 * nothing: the library's `AutoFocusManager` decides once, when the preview starts,
 * whether to drive focus passes at all, so a mode changed underneath it can leave nobody
 * calling `autoFocus()`. The mode it already picked re-reads these areas on its next pass.
 */
// Camera1 throughout: zxing-android-embedded is built on it, so `Camera.Area` is the only
// focus API reachable from here.
@Suppress("DEPRECATION")
private fun focusAt(view: BarcodeView, nx: Float, ny: Float) {
    if (!view.isPreviewActive) return
    val rotation = view.cameraInstance?.cameraRotation ?: return
    if (rotation < 0) return

    // Camera1 focus areas live in the *sensor's* frame, which `setDisplayOrientation`
    // does not touch, so the display rotation has to be undone before mapping.
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
    // A degenerate rectangle is rejected by the camera HAL, and clamping at an edge can
    // produce one.
    if (rect.width() <= 0 || rect.height() <= 0) return

    view.changeCameraParameters { parameters ->
        val areas = listOf(Camera.Area(rect, 1000))
        if (parameters.maxNumFocusAreas > 0) parameters.focusAreas = areas
        if (parameters.maxNumMeteringAreas > 0) parameters.meteringAreas = areas
        parameters
    }
}

/** Half-width of the focus box, as a fraction of the frame. */
private const val FOCUS_AREA_HALF = 0.1f

private const val FOCUS_INDICATOR_MS = 700L

private val FOCUS_INDICATOR_SIDE = 68.dp
