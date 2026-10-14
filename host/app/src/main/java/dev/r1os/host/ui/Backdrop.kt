package dev.r1os.host.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.Shader
import android.media.ExifInterface
import android.os.Handler
import android.os.Looper
import android.view.Choreographer
import android.view.View
import dev.r1os.host.net.R1d
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.File
import java.util.concurrent.Executors
import kotlin.math.abs
import kotlin.math.sign
import kotlin.math.sqrt

/**
 * A screen's background (ADR-0043): plain, a tint, a styled gradient, a soft gradient of your own colours (`mesh`,
 * after Arc's: large glows melting into each other), a texture, or one of your photos, darkened (lightened in light
 * mode) so text stays readable. Fine grain goes over all but plain and photos, at the look's amount.
 *
 * It's drawn into a GPU layer, so standing still it costs one textured quad a frame. With `flow`, the wheel and the
 * button nudge a gradient's glows: they drift a little each their own way and springs bring them back, so it moves
 * like something floating, and only while it moves.
 */
class Backdrop(context: Context, private val spec: JSONObject) : View(context), Choreographer.FrameCallback {
    private val kind = spec.optString("kind", "plain")
    private val c = Theme.colorOf(spec.optString("color").ifEmpty { Theme.accentName })
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)
    private val veil = Paint()
    private var photo: Bitmap? = null
    private val place = Matrix()
    /** The fine grain over it (none for plain and photos). */
    private val grain: Paint? = spec.optDouble("grain", GRAIN_DEFAULT[kind] ?: 0.0).toFloat().takeIf { it > 0f && kind !in listOf("plain", "photo") }?.let {
        Noise.paint(if (kind == "texture" && spec.optString("style") == "noise") Noise.Kind.NOISE else Noise.Kind.GRAIN, it)
    }

    /** A gradient's glows: where each rests, its paint, and (flow) how far it has drifted. */
    private class Glow(val paint: Paint, val cx: Float, val cy: Float, val weight: Float, val swirl: Float) {
        var x = 0f; var y = 0f; var vx = 0f; var vy = 0f
        val m = Matrix()
    }
    private val glows = ArrayList<Glow>()
    private var base = Theme.BG
    private val flow = spec.optBoolean("flow", false) && kind in listOf("mesh", "gradient")
    /** The button's squeeze: the glows draw in while it's held, and out again. */
    private var q = 0f
    private var vq = 0f
    private var lastFrame = 0L
    private var moving = false

    init {
        if (kind != "plain") setLayerType(LAYER_TYPE_HARDWARE, null)
        if (kind == "photo") {
            spec.optString("photo").takeIf { it.isNotEmpty() }?.let { id ->
                BackdropPhotos.load(context, id) { b ->
                    photo = b
                    setUp()
                    invalidate()
                }
            }
        }
    }

    /** `c` moved toward the screen's colour by `t`: dark mode's near black, light mode's near white. */
    private fun toward(c: Int, t: Float) = Theme.mix(c, Theme.BG, t)

    /** How far a tint (and a texture's ground) is toward the screen's colour: a deep shade, or a pastel. */
    private val tint = if (Theme.light) 0.7f else 0.82f

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) = setUp()

    private fun setUp() {
        val w = width.toFloat()
        val h = height.toFloat()
        if (w <= 0f || h <= 0f) return
        glows.clear()
        fill.shader = null
        fill.color = Theme.BG
        base = Theme.BG
        when (kind) {
            "tint", "texture" -> base = toward(c, tint)
            "gradient" -> {
                val shader = when (spec.optString("style")) {
                    "dusk" -> LinearGradient(0f, 0f, 0f, h, intArrayOf(Theme.mix(Theme.hue(c, -30f), Theme.BG, 0.55f), toward(c, 0.8f), Theme.BG), floatArrayOf(0f, 0.55f, 1f), Shader.TileMode.CLAMP)
                    "diagonal" -> LinearGradient(0f, 0f, w, h, intArrayOf(toward(c, 0.5f), Theme.mix(Theme.hue(c, 60f), Theme.BG, 0.78f), Theme.BG), floatArrayOf(0f, 0.55f, 1f), Shader.TileMode.CLAMP)
                    // A glow from below, where the agent peeks up.
                    "horizon" -> LinearGradient(0f, h, 0f, 0f, intArrayOf(toward(c, 0.45f), toward(c, 0.82f), Theme.BG), floatArrayOf(0f, 0.4f, 1f), Shader.TileMode.CLAMP)
                    else -> RadialGradient(w / 2f, h * 0.12f, h * 0.8f, intArrayOf(toward(c, 0.5f), toward(c, 0.8f), Theme.BG), floatArrayOf(0f, 0.45f, 1f), Shader.TileMode.CLAMP)
                }
                glows += Glow(Paint(Paint.ANTI_ALIAS_FLAG).apply { this.shader = shader }, w / 2f, h / 2f, 1f, 0.5f)
            }
            "mesh" -> setUpMesh(w, h)
            "photo" -> photo?.let { p ->
                // Covers the screen, centred.
                val s = maxOf(w / p.width, h / p.height)
                place.setScale(s, s)
                place.postTranslate((w - p.width * s) / 2f, (h - p.height * s) / 2f)
                veil.color = Theme.withAlpha(Theme.BG, spec.optDouble("dim", 0.45).toFloat().coerceIn(0f, 0.85f))
            }
        }
        applyFlow()
    }

    /** One to three colours as glows over a ground of their blend, pulled toward the screen's colour by `dim`. */
    private fun setUpMesh(w: Float, h: Float) {
        val a = spec.optJSONArray("colors")
        val raw = (0 until (a?.length() ?: 0)).map { Theme.colorOf(a!!.optString(it)) }.ifEmpty { listOf(Theme.accent) }
        val dim = spec.optDouble("dim", 0.4).toFloat().coerceIn(0f, 0.85f)
        // One colour: itself, and a turned shade of it.
        val cs = (if (raw.size == 1) listOf(raw[0], Theme.mix(Theme.hue(raw[0], 28f), Theme.BG, 0.25f)) else raw).map { toward(it, dim) }
        val avg = cs.reduce { x, y -> Theme.mix(x, y, 0.5f) }
        base = toward(avg, 0.32f)
        // Where each glows from (fractions of the screen) and how far it reaches (of its height); how it drifts.
        val spots = if (cs.size == 2) {
            listOf(floatArrayOf(0.12f, 0.06f, 1.0f, 1f, 0.6f), floatArrayOf(0.92f, 0.92f, 0.95f, 0.7f, -0.8f))
        } else {
            listOf(floatArrayOf(0.1f, 0.04f, 0.85f, 1f, 0.6f), floatArrayOf(0.98f, 0.42f, 0.78f, 0.65f, -0.8f), floatArrayOf(0.28f, 1.02f, 0.88f, 1.3f, 0.4f))
        }
        for ((i, col) in cs.withIndex()) {
            val sp = spots[i]
            val cx = w * sp[0]
            val cy = h * sp[1]
            val shader = RadialGradient(cx, cy, h * sp[2], intArrayOf(col, Theme.withAlpha(col, 0.55f), Theme.withAlpha(col, 0f)), floatArrayOf(0f, 0.45f, 1f), Shader.TileMode.CLAMP)
            glows += Glow(Paint(Paint.ANTI_ALIAS_FLAG).apply { this.shader = shader }, cx, cy, sp[3], sp[4])
        }
    }

    override fun onDraw(canvas: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        val p = photo
        if (kind == "photo" && p != null) {
            canvas.drawBitmap(p, place, fill)
            canvas.drawRect(0f, 0f, w, h, veil)
            return
        }
        canvas.drawColor(base)
        for (g in glows) canvas.drawRect(0f, 0f, w, h, g.paint)
        grain?.let { canvas.drawRect(0f, 0f, w, h, it) }
    }

    // ---- flow ----------------------------------------------------------------------------------------

    /** A wheel tick: the glows drift with it, each its own way. */
    fun nudge(delta: Int) {
        if (!flow || glows.isEmpty()) return
        val d = sign(delta.toFloat())
        val kick = context.dp(KICK_DP)
        for (g in glows) {
            g.vy -= d * kick * g.weight
            g.vx += d * kick * 0.8f * g.swirl
        }
        // And a breath: the glows swell a little with each tick.
        vq += SQUEEZE * 0.35f
        start()
    }

    /** The button: the glows draw in while it's down, and spring out when it's let go. */
    fun press(down: Boolean) {
        if (!flow || glows.isEmpty()) return
        vq += if (down) -SQUEEZE else SQUEEZE * 0.8f
        start()
    }

    /**
     * Back at rest at once: a page is opening over it (T-106). While it flows, each frame redraws its layer (the
     * gradient's glows and grain, ~25 ms of the R1's GPU), and that under a page's slide-in made the slide stutter.
     */
    fun settle() {
        if (!moving && q == 0f) return
        Choreographer.getInstance().removeFrameCallback(this)
        moving = false
        for (g in glows) { g.x = 0f; g.y = 0f; g.vx = 0f; g.vy = 0f }
        q = 0f
        vq = 0f
        applyFlow()
        invalidate()
    }

    private fun start() {
        if (moving) return
        moving = true
        lastFrame = 0L
        Choreographer.getInstance().postFrameCallback(this)
    }

    override fun doFrame(frameTimeNanos: Long) {
        val dt = if (lastFrame == 0L) 1f / 60 else ((frameTimeNanos - lastFrame) / 1e9f).coerceIn(0f, 1f / 20)
        lastFrame = frameTimeNanos
        val limit = context.dp(MAX_DRIFT_DP)
        var resting = true
        var left = dt
        while (left > 0f) {
            val step = minOf(STEP, left)
            left -= step
            for (g in glows) {
                g.vx += (-STIFF * g.x - DAMP * g.vx) * step
                g.vy += (-STIFF * g.y - DAMP * g.vy) * step
                g.x = (g.x + g.vx * step).coerceIn(-limit, limit)
                g.y = (g.y + g.vy * step).coerceIn(-limit, limit)
            }
            vq += (-STIFF * q - DAMP * vq) * step
            q += vq * step
        }
        val px = context.dp(1f)
        for (g in glows) if (abs(g.x) > 0.3f * px || abs(g.y) > 0.3f * px || abs(g.vx) > 2f * px || abs(g.vy) > 2f * px) resting = false
        if (abs(q) > 0.002f || abs(vq) > 0.01f) resting = false
        if (resting) {
            for (g in glows) { g.x = 0f; g.y = 0f; g.vx = 0f; g.vy = 0f }
            q = 0f
            vq = 0f
            moving = false
        }
        applyFlow()
        invalidate()
        if (moving && isAttachedToWindow) Choreographer.getInstance().postFrameCallback(this) else moving = false
    }

    /** Each glow where it has drifted to, and the squeeze's scale. */
    private fun applyFlow() {
        val s = 1f + 0.1f * q
        for (g in glows) {
            g.m.setScale(s, s, g.cx, g.cy)
            g.m.postTranslate(g.x, g.y)
            g.paint.shader?.setLocalMatrix(g.m)
        }
    }

    override fun onDetachedFromWindow() {
        Choreographer.getInstance().removeFrameCallback(this)
        moving = false
        super.onDetachedFromWindow()
    }

    companion object {
        /** Springs: soft, and a little under-damped, so a glow overshoots its rest once, like something in water. */
        private const val STIFF = 40f
        private val DAMP = 2f * 0.45f * sqrt(STIFF)
        private const val STEP = 1f / 240
        /** A tick's push, in dp a second; how far a glow may drift. */
        private const val KICK_DP = 700f
        private const val MAX_DRIFT_DP = 180f
        private const val SQUEEZE = 1.4f
        /** A look saved before grain was a setting: r1d's defaults (crate::look). */
        val GRAIN_DEFAULT = mapOf("mesh" to 0.5, "texture" to 0.7, "gradient" to 0.3)
    }
}

/**
 * Photos as backgrounds: fetched from r1d's media store once, made screen-sized (turned upright), and kept on disk so
 * the next start doesn't fetch it again.
 */
object BackdropPhotos {
    private val io = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val kept = LinkedHashMap<String, Bitmap>()

    fun load(context: Context, id: String, onReady: (Bitmap) -> Unit) {
        kept[id]?.let { onReady(it); return }
        val file = File(context.cacheDir, "backdrop-${id.filter { it.isLetterOrDigit() || it == '-' }}.jpg")
        val ready = { b: Bitmap ->
            main.post {
                kept[id] = b
                while (kept.size > 2) kept.remove(kept.keys.first())
                onReady(b)
            }
        }
        io.execute {
            if (file.exists()) {
                BitmapFactory.decodeFile(file.path)?.let { ready(it); return@execute }
            }
            R1d.bytes("/api/media/${R1d.enc(id)}/file") { bytes ->
                if (bytes == null) return@bytes
                io.execute {
                    val b = runCatching { screenSized(bytes) }.getOrNull() ?: return@execute
                    runCatching { file.outputStream().use { b.compress(Bitmap.CompressFormat.JPEG, 90, it) } }
                    ready(b)
                }
            }
        }
    }

    /** Decoded at about twice the screen's size at most, and turned the way the camera held it. */
    private fun screenSized(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        var sample = 1
        while (minOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MIN_EDGE) sample *= 2
        val b = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
        val turn = when (runCatching { ExifInterface(ByteArrayInputStream(bytes)).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) }.getOrDefault(1)) {
            ExifInterface.ORIENTATION_ROTATE_90 -> 90f
            ExifInterface.ORIENTATION_ROTATE_180 -> 180f
            ExifInterface.ORIENTATION_ROTATE_270 -> 270f
            else -> 0f
        }
        if (turn == 0f) return b
        return Bitmap.createBitmap(b, 0, 0, b.width, b.height, Matrix().apply { postRotate(turn) }, true).also { if (it != b) b.recycle() }
    }

    /** A photo's short edge is decoded to at least this (the R1's screen is 480 × 640). */
    private const val MIN_EDGE = 640
}
