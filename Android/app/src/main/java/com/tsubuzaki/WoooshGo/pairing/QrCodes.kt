package com.tsubuzaki.WoooshGo.pairing

import android.graphics.Bitmap
import android.graphics.Color
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter

/** zxing-core, so no Play Services dependency. */
object QrCodes {

    fun encode(payload: String, sizePx: Int): Bitmap {
        val matrix = QRCodeWriter().encode(
            payload,
            BarcodeFormat.QR_CODE,
            sizePx,
            sizePx,
            mapOf(EncodeHintType.MARGIN to 1),
        )
        val pixels = IntArray(sizePx * sizePx)
        for (y in 0 until sizePx) {
            val offset = y * sizePx
            for (x in 0 until sizePx) {
                pixels[offset + x] = if (matrix.get(x, y)) Color.BLACK else Color.WHITE
            }
        }
        return Bitmap.createBitmap(sizePx, sizePx, Bitmap.Config.ARGB_8888).apply {
            setPixels(pixels, 0, sizePx, 0, 0, sizePx, sizePx)
        }
    }
}
