package dev.r1os.host.camera

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorMatrix
import android.graphics.ColorMatrixColorFilter
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.Shader
import dev.r1os.host.ui.Noise

/**
 * The camera's films (ADR-0046): a colour matrix (saturation, a tint, contrast and lifted blacks), a vignette and
 * grain. The viewfinder shows one live (the preview's layer paint, and an overlay for the vignette and grain); a photo
 * taken through it is baked with the same.
 *
 * Exposure goes in the same matrix when the camera can't do it itself (`stops`, a gain before the film): Rabbit's HAL
 * reports no exposure compensation (its range is 0..0).
 */
enum class Film(
    val word: String,
    private val sat: Float = 1f,
    private val contrast: Float = 1f,
    /** Added to every channel after the contrast (0–255): lifted blacks. */
    private val lift: Float = 0f,
    private val r: Float = 1f,
    private val g: Float = 1f,
    private val b: Float = 1f,
    /** 0–1: how dark the corners go. */
    val vignette: Float = 0f,
    /** 0–1: how much grain. */
    val grain: Float = 0f,
) {
    STD("std"),
    VIVID("vivid", sat = 1.35f, contrast = 1.08f, vignette = 0.12f),
    WARM("warm", sat = 1.05f, contrast = 1.02f, lift = 4f, r = 1.08f, b = 0.86f),
    COOL("cool", sat = 0.95f, contrast = 1.04f, r = 0.9f, b = 1.1f),
    FADE("fade", sat = 0.7f, contrast = 0.8f, lift = 30f, grain = 0.12f),
    FILM("film", sat = 0.86f, contrast = 0.94f, lift = 14f, r = 1.06f, b = 0.9f, vignette = 0.35f, grain = 0.4f),
    MONO("mono", sat = 0f, contrast = 1.1f, vignette = 0.1f, grain = 0.15f),
    NOIR("noir", sat = 0f, contrast = 1.45f, lift = -12f, vignette = 0.55f, grain = 0.3f);

    /** Its colour matrix, after `stops` of exposure; null for none. */
    fun matrix(stops: Float = 0f): ColorMatrix? {
        if (this == STD && stops == 0f) return null
        val gain = Math.pow(2.0, stops.toDouble()).toFloat()
        val m = ColorMatrix(floatArrayOf(gain, 0f, 0f, 0f, 0f, 0f, gain, 0f, 0f, 0f, 0f, 0f, gain, 0f, 0f, 0f, 0f, 0f, 1f, 0f))
        if (this == STD) return m
        m.postConcat(ColorMatrix().apply { setSaturation(sat) })
        m.postConcat(ColorMatrix(floatArrayOf(r, 0f, 0f, 0f, 0f, 0f, g, 0f, 0f, 0f, 0f, 0f, b, 0f, 0f, 0f, 0f, 0f, 1f, 0f)))
        val off = 128f * (1f - contrast) + lift
        m.postConcat(ColorMatrix(floatArrayOf(contrast, 0f, 0f, 0f, off, 0f, contrast, 0f, 0f, off, 0f, 0f, contrast, 0f, off, 0f, 0f, 0f, 1f, 0f)))
        return m
    }

    /** A paint that draws through it (the preview's layer paint, or a photo's). */
    fun paint(stops: Float = 0f): Paint? = matrix(stops)?.let { m -> Paint(Paint.FILTER_BITMAP_FLAG).apply { colorFilter = ColorMatrixColorFilter(m) } }

    /** A photo, through it: exposure, colour, vignette, grain (`grainScale`: how big a speck is, in the photo's pixels). */
    fun bake(src: Bitmap, stops: Float = 0f, grainScale: Float = 2.5f): Bitmap {
        if (this == STD && stops == 0f) return src
        val out = Bitmap.createBitmap(src.width, src.height, Bitmap.Config.ARGB_8888)
        val c = Canvas(out)
        c.drawBitmap(src, 0f, 0f, paint(stops))
        overlay(c, src.width.toFloat(), src.height.toFloat(), grainScale)
        return out
    }

    /** The vignette and the grain over a `w` × `h` picture. */
    fun overlay(c: Canvas, w: Float, h: Float, grainScale: Float) {
        if (vignette > 0f) {
            val r = kotlin.math.hypot(w, h) / 2f
            val p = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                shader = RadialGradient(w / 2f, h / 2f, r, intArrayOf(Color.TRANSPARENT, Color.TRANSPARENT, Color.argb((vignette * 235).toInt(), 0, 0, 0)),
                    floatArrayOf(0f, 0.55f, 1f), Shader.TileMode.CLAMP)
            }
            c.drawRect(0f, 0f, w, h, p)
        }
        if (grain > 0f) {
            val p = Noise.paint(Noise.Kind.GRAIN, grain)
            p.shader?.setLocalMatrix(Matrix().apply { setScale(grainScale, grainScale) })
            p.isFilterBitmap = grainScale > 1f
            c.drawRect(0f, 0f, w, h, p)
        }
    }
}

/**
 * The camera's settings, kept on the R1 (the host's preferences): the dials (film, exposure, white balance, zoom),
 * what the wheel turns, the timer, the grid, the shutter's sound, and a frame counter.
 */
object CameraPrefs {
    private const val PREFS = "camera"
    private var prefs: android.content.SharedPreferences? = null

    fun init(context: Context) {
        if (prefs == null) prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    private fun int(k: String, d: Int) = prefs?.getInt(k, d) ?: d
    private fun bool(k: String, d: Boolean) = prefs?.getBoolean(k, d) ?: d
    private fun put(k: String, v: Any) {
        val e = prefs?.edit() ?: return
        when (v) {
            is Int -> e.putInt(k, v)
            is Boolean -> e.putBoolean(k, v)
            is String -> e.putString(k, v)
        }
        e.apply()
    }

    var film: Film
        get() = Film.entries.firstOrNull { it.name == prefs?.getString("film", null) } ?: Film.STD
        set(v) = put("film", v.name)
    /** Exposure, in thirds of a stop (−6..6). */
    var ev: Int
        get() = int("ev", 0)
        set(v) = put("ev", v.coerceIn(-6, 6))
    /** Camera2's white balance mode. */
    var awb: Int
        get() = int("awb", android.hardware.camera2.CaptureRequest.CONTROL_AWB_MODE_AUTO)
        set(v) = put("awb", v)
    /** The zoom's step ([ZOOMS]). */
    var zoom: Int
        get() = int("zoom", 0)
        set(v) = put("zoom", v)
    /** The knob the wheel turns: film, exp, wb or zoom. */
    var wheel: String
        get() = prefs?.getString("wheel", "film") ?: "film"
        set(v) = put("wheel", v)
    /** Seconds before the shot: 0, 3 or 10. */
    var timer: Int
        get() = int("timer", 0)
        set(v) = put("timer", v)
    var grid: Boolean
        get() = bool("grid", false)
        set(v) = put("grid", v)
    var shutter: Boolean
        get() = bool("shutter", true)
        set(v) = put("shutter", v)
    /** Facing you, the viewfinder and the photo as in a mirror. */
    var mirror: Boolean
        get() = bool("mirror", true)
        set(v) = put("mirror", v)
    /** Photos taken with the camera, ever: the counter's number. */
    var frames: Int
        get() = int("frames", 0)
        set(v) = put("frames", v)

    val ZOOMS = listOf(1f, 1.25f, 1.5f, 2f, 2.5f, 3f, 4f)

    /** The dials back to their defaults. */
    fun resetDials() {
        film = Film.STD
        ev = 0
        awb = android.hardware.camera2.CaptureRequest.CONTROL_AWB_MODE_AUTO
        zoom = 0
    }
}
