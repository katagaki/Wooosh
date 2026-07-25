package com.tsubuzaki.WoooshGo.pairing

import com.journeyapps.barcodescanner.CaptureActivity

/**
 * The scanner, free to rotate.
 *
 * `zxing-android-embedded` declares its own `CaptureActivity` with
 * `android:screenOrientation="sensorLandscape"` hardcoded in the library manifest, so a
 * phone held normally is forced into landscape to scan a code. `ScanOptions
 * .setOrientationLocked(false)` does not help: it only stops the activity pinning itself
 * to whatever orientation it started in, and cannot override a manifest attribute.
 *
 * Subclassing and re-declaring the activity with `fullSensor` is the supported way round
 * it. No behaviour is added; the manifest entry is the whole fix.
 */
class PortraitCaptureActivity : CaptureActivity()
