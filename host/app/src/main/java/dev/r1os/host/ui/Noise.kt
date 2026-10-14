package dev.r1os.host.ui

import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.BlendMode
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Shader

/**
 * Film grain for the characters' finishes and the backgrounds (ADR-0043's update: finer than the web shell's): one
 * speck per pixel, as often lighter as darker (a triangular spread, so most are faint and a few stand out), tiled
 * large enough not to show its repeat and drawn 1:1 on the screen's pixels. Blended as soft light, it lifts and
 * deepens the colour under it instead of greying it.
 */
object Noise {
    enum class Kind(val size: Int, val seed: Long, val scale: Float, val amp: Double) {
        /** Fine film grain: a speck per pixel. */
        GRAIN(256, 7, 1f, 190.0),
        /** Softer and larger: half the resolution, drawn twice the size and filtered (clouds of grain, no blocks). */
        NOISE(160, 13, 2f, 230.0),
    }

    private val cache = HashMap<Kind, Bitmap>()

    fun bitmap(kind: Kind): Bitmap = cache.getOrPut(kind) {
        val n = kind.size
        val pixels = IntArray(n * n)
        var s = kind.seed * 0x9E3779B97F4A7C15uL.toLong() or 1L
        fun next(): Double {
            // xorshift64*: even, without the LCG's patterns on a 2D tile.
            s = s xor (s ushr 12)
            s = s xor (s shl 25)
            s = s xor (s ushr 27)
            return ((s * 0x2545F4914F6CDD1DL) ushr 11).toDouble() / (1L shl 53).toDouble()
        }
        for (i in 0 until n * n) {
            val t = next() + next() - 1.0
            val shade = if (t > 0) 255 else 0
            val a = Math.round(Math.pow(Math.abs(t), 1.35) * kind.amp).toInt().coerceIn(0, 255)
            pixels[i] = Color.argb(a, shade, shade, shade)
        }
        Bitmap.createBitmap(pixels, n, n, Bitmap.Config.ARGB_8888)
    }

    /** Its tile as a shader, at its scale in the screen's pixels. */
    fun shader(kind: Kind): BitmapShader = BitmapShader(bitmap(kind), Shader.TileMode.REPEAT, Shader.TileMode.REPEAT).apply {
        if (kind.scale != 1f) setLocalMatrix(Matrix().apply { setScale(kind.scale, kind.scale) })
    }

    /** A paint that lays it over what's drawn, as soft light, at `amount` (0–1). */
    fun paint(kind: Kind, amount: Float): Paint = Paint(if (kind.scale != 1f) Paint.FILTER_BITMAP_FLAG else 0).apply {
        shader = shader(kind)
        blendMode = BlendMode.SOFT_LIGHT
        alpha = (amount.coerceIn(0f, 1f) * 255).toInt()
    }
}
