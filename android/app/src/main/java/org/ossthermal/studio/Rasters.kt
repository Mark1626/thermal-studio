package org.ossthermal.studio

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.graphics.createBitmap
import uniffi.thermal_ffi.ImageSource
import uniffi.thermal_ffi.Raster

private const val INK = 0xFF18181C.toInt()
private const val PAPER = 0xFFFAFAF6.toInt()

/** Expand a packed 1-bit raster (MSB first, 1 = black) into a displayable bitmap. */
fun Raster.toImageBitmap(): ImageBitmap {
    val w = width.coerceAtLeast(1)
    val h = height.coerceAtLeast(1)
    val pixels = IntArray(w * h) { PAPER }
    for (y in 0 until height) {
        val row = y * stride
        for (x in 0 until width) {
            val byte = data[row + (x ushr 3)].toInt()
            if (byte and (0x80 ushr (x and 7)) != 0) pixels[y * w + x] = INK
        }
    }
    return Bitmap.createBitmap(pixels, w, h, Bitmap.Config.ARGB_8888).asImageBitmap()
}

/**
 * Decode any image Android understands (JPEG, PNG, WebP, HEIC, AVIF…), applying EXIF
 * rotation, flatten transparency onto white paper and hand straight RGBA to Rust.
 * Large photos are decoded at most [maxSide] px on the long side; the print head is
 * only 384 dots wide, so this loses nothing.
 */
fun decodeImageSource(context: Context, uri: Uri, maxSide: Int = 2048): ImageSource {
    val source = ImageDecoder.createSource(context.contentResolver, uri)
    val decoded = ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        val (w, h) = info.size.width to info.size.height
        val scale = maxSide.toFloat() / maxOf(w, h)
        if (scale < 1f) {
            decoder.setTargetSize((w * scale).toInt().coerceAtLeast(1), (h * scale).toInt().coerceAtLeast(1))
        }
    }
    val w = decoded.width
    val h = decoded.height
    val flat = createBitmap(w, h)
    Canvas(flat).apply {
        drawColor(Color.WHITE)
        drawBitmap(decoded, 0f, 0f, null)
    }
    decoded.recycle()
    val pixels = IntArray(w * h)
    flat.getPixels(pixels, 0, w, 0, 0, w, h)
    flat.recycle()
    val rgba = ByteArray(w * h * 4)
    for (i in pixels.indices) {
        val c = pixels[i]
        rgba[4 * i] = (c ushr 16).toByte()
        rgba[4 * i + 1] = (c ushr 8).toByte()
        rgba[4 * i + 2] = c.toByte()
        rgba[4 * i + 3] = 0xFF.toByte()
    }
    return ImageSource.fromRgba(w, h, rgba)
}
