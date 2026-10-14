package dev.r1os.host.shell

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RadialGradient
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.view.Choreographer
import android.view.View
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * The Recorder's and the Journal's hardware (ADR-0037), after teenage engineering's field recorders: flat
 * anthracite on black, hairlines, small lowercase labels, white glyphs, and red only for recording.
 */
object Deck {
    val KEY = Color.parseColor("#16171a")
    val KEY_TOP = Color.parseColor("#26282d")
    val HAIR = Color.parseColor("#2c2e33")
    val PACK = Color.parseColor("#1b1c20")
    val GLYPH = Color.parseColor("#d9dce1")
    val REC = Color.parseColor("#ff3b30")
    val DIM = Color.parseColor("#3a3c42")

    /** h.mm.ss, as the TP-7's display writes time. */
    fun dotted(ms: Long): String {
        val s = (ms / 1000).coerceAtLeast(0)
        return "%d.%02d.%02d".format(s / 3600, s / 60 % 60, s % 60)
    }

    /** h:mm:ss, a tape counter. */
    fun counter(ms: Long): String {
        val s = (ms / 1000).coerceAtLeast(0)
        return "%d:%02d:%02d".format(s / 3600, s / 60 % 60, s % 60)
    }
}

/**
 * Calls `frame(seconds since the last one)` on every display frame while started, and nothing at all while
 * stopped: the screens start it only while something moves and they're on screen (battery comes first).
 */
class Ticker(private val frame: (Float) -> Unit) : Choreographer.FrameCallback {
    private var last = 0L
    var running = false
        private set

    fun start() {
        if (running) return
        running = true
        last = 0L
        Choreographer.getInstance().postFrameCallback(this)
    }

    fun stop() {
        running = false
        Choreographer.getInstance().removeFrameCallback(this)
    }

    override fun doFrame(now: Long) {
        if (!running) return
        val dt = if (last == 0L) 0f else ((now - last) / 1e9f).coerceAtMost(0.1f)
        last = now
        frame(dt)
        if (running) Choreographer.getInstance().postFrameCallback(this)
    }
}

/** The transport: square keys with a glyph each (and a word under it), the focused one filled with the accent. */
class TransportKeys(context: Context) : View(context) {
    enum class Glyph { REC, PLAY, PAUSE, STOP, MARK, PHOTO }

    var glyphs = listOf(Glyph.REC, Glyph.PLAY, Glyph.STOP)
        set(v) { field = v; invalidate() }
    var labels: List<String?> = emptyList()
        set(v) { field = v; invalidate() }
    /** The key the wheel is on (-1: none). */
    var focus = -1
        set(v) { field = v; invalidate() }
    /** The key that's running: its glyph lit (red for recording). */
    var lit = -1
        set(v) { field = v; invalidate() }
    /** A blinking record glyph shows dimmed on its off beat. */
    var blinkOff = false
        set(v) { if (field != v) { field = v; invalidate() } }
    var accent = Theme.accent

    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val glyph = Paint(Paint.ANTI_ALIAS_FLAG)
    private val label = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = Theme.font(600)
        textSize = context.dp(11f)
        textAlign = Paint.Align.CENTER
    }
    private val r = RectF()
    private val path = Path()

    override fun onDraw(c: Canvas) {
        val n = glyphs.size.coerceAtLeast(1)
        val gap = context.dp(6f)
        val w = (width - gap * (n - 1)) / n
        val radius = context.dp(10f)
        for (i in 0 until n) {
            val x = i * (w + gap)
            r.set(x, 0f, x + w, height.toFloat())
            val focused = i == focus
            fill.color = if (focused) accent else Deck.KEY
            c.drawRoundRect(r, radius, radius, fill)
            if (!focused) {
                // The key's top edge catches the light.
                fill.color = Deck.KEY_TOP
                c.drawRoundRect(RectF(r.left + radius, r.top, r.right - radius, r.top + context.dp(1f)), 1f, 1f, fill)
            }
            val g = glyphs[i]
            glyph.color = when {
                focused -> Theme.ON_ACCENT
                i == lit && g == Glyph.REC -> if (blinkOff) Theme.withAlpha(Deck.REC, 0.35f) else Deck.REC
                i == lit -> Theme.FG
                else -> Deck.GLYPH
            }
            val word = labels.getOrNull(i)
            val cy = if (word != null) r.centerY() - context.dp(7f) else r.centerY()
            c.glyph(g, r.centerX(), cy, context.dp(9f), glyph, path)
            if (word != null) {
                label.color = if (focused) Theme.withAlpha(Theme.ON_ACCENT, 0.7f) else Theme.MUTED
                c.drawText(word, r.centerX(), r.bottom - context.dp(11f), label)
            }
        }
    }

}

/** A transport glyph centred on (x, y), `s` its half size. */
fun Canvas.glyph(g: TransportKeys.Glyph, x: Float, y: Float, s: Float, paint: Paint, path: Path) {
    when (g) {
        TransportKeys.Glyph.REC -> drawCircle(x, y, s, paint)
        TransportKeys.Glyph.STOP -> drawRect(x - s * 0.85f, y - s * 0.85f, x + s * 0.85f, y + s * 0.85f, paint)
        TransportKeys.Glyph.PLAY -> {
            path.reset()
            path.moveTo(x - s * 0.7f, y - s)
            path.lineTo(x + s, y)
            path.lineTo(x - s * 0.7f, y + s)
            path.close()
            drawPath(path, paint)
        }
        TransportKeys.Glyph.PAUSE -> {
            val bw = s * 0.55f
            drawRect(x - s * 0.85f, y - s, x - s * 0.85f + bw, y + s, paint)
            drawRect(x + s * 0.85f - bw, y - s, x + s * 0.85f, y + s, paint)
        }
        // A ribbon with a notch: this moment, marked (T-068).
        TransportKeys.Glyph.MARK -> {
            path.reset()
            path.moveTo(x - s * 0.62f, y - s)
            path.lineTo(x + s * 0.62f, y - s)
            path.lineTo(x + s * 0.62f, y + s)
            path.lineTo(x, y + s * 0.42f)
            path.lineTo(x - s * 0.62f, y + s)
            path.close()
            drawPath(path, paint)
        }
        // A camera: its body and the bump on top, the lens cut out.
        TransportKeys.Glyph.PHOTO -> {
            path.reset()
            path.fillType = Path.FillType.EVEN_ODD
            path.addRoundRect(x - s, y - s * 0.62f, x + s, y + s * 0.82f, s * 0.22f, s * 0.22f, Path.Direction.CW)
            path.addRect(x - s * 0.36f, y - s * 0.9f, x + s * 0.36f, y - s * 0.62f, Path.Direction.CW)
            path.addCircle(x, y + s * 0.1f, s * 0.4f, Path.Direction.CW)
            drawPath(path, paint)
            path.fillType = Path.FillType.WINDING
        }
    }
}

/** One glyph on its own (a counter's state). */
class GlyphView(context: Context) : View(context) {
    var glyph = TransportKeys.Glyph.STOP
        set(v) { if (field != v) { field = v; invalidate() } }
    var color = Deck.DIM
        set(v) { if (field != v) { field = v; invalidate() } }
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val path = Path()

    override fun onDraw(c: Canvas) {
        paint.color = color
        c.glyph(glyph, width / 2f, height / 2f, min(width, height) * 0.42f, paint, path)
    }
}

/** A round lamp, as on the TP-7 beside its keys: dark at rest, a colour with a glow when on. */
class Led(context: Context) : View(context) {
    var color = Deck.DIM
        set(v) { if (field != v) { field = v; invalidate() } }
    /** 0..1: how far the glow reaches (the voice level while recording). */
    var glow = 0f
        set(v) { if (kotlin.math.abs(field - v) > 0.02f) { field = v; invalidate() } }
    private val p = Paint(Paint.ANTI_ALIAS_FLAG)

    override fun onDraw(c: Canvas) {
        val cx = width / 2f
        val cy = height / 2f
        val r = min(width, height) * 0.22f
        if (glow > 0f) {
            p.shader = RadialGradient(cx, cy, r * (1.4f + 1.6f * glow), Theme.withAlpha(color, 0.55f), Color.TRANSPARENT, Shader.TileMode.CLAMP)
            c.drawCircle(cx, cy, r * (1.4f + 1.6f * glow), p)
            p.shader = null
        }
        p.color = color
        c.drawCircle(cx, cy, r, p)
    }
}

/** A row of ticks lit from the left with the level, like a tape deck's meter. */
class TickMeter(context: Context) : View(context) {
    var level = 0f
        set(v) {
            val l = v.coerceIn(0f, 1f)
            if ((l * TICKS).roundToInt() != (field * TICKS).roundToInt()) { field = l; invalidate() } else field = l
        }
    var color = Deck.REC
    private val p = Paint(Paint.ANTI_ALIAS_FLAG)

    override fun onDraw(c: Canvas) {
        val tw = context.dp(2f)
        val step = (width - tw) / (TICKS - 1).toFloat()
        val lit = (level * TICKS).roundToInt()
        for (i in 0 until TICKS) {
            p.color = if (i < lit) color else Deck.DIM
            val x = i * step
            val h = if (i % 8 == 0) height.toFloat() else height * 0.62f
            c.drawRoundRect(x, height - h, x + tw, height.toFloat(), tw, tw, p)
        }
    }

    private companion object {
        const val TICKS = 41
    }
}

/** The small display: a black window with a title, a line under it, and a boxed number. */
class Oled(context: Context) : View(context) {
    var title = ""
        set(v) { if (field != v) { field = v; invalidate() } }
    var line = ""
        set(v) { if (field != v) { field = v; invalidate() } }
    var box: String? = null
        set(v) { if (field != v) { field = v; invalidate() } }
    /** Recording: the title in red. */
    var hot = false
        set(v) { if (field != v) { field = v; invalidate() } }

    private val frame = Paint(Paint.ANTI_ALIAS_FLAG)
    private val big = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD); textSize = context.dp(17f) }
    private val small = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Typeface.MONOSPACE; textSize = context.dp(12.5f) }
    private val r = RectF()

    override fun onDraw(c: Canvas) {
        val radius = context.dp(8f)
        r.set(0f, 0f, width.toFloat(), height.toFloat())
        frame.style = Paint.Style.FILL
        frame.color = Color.BLACK
        c.drawRoundRect(r, radius, radius, frame)
        frame.style = Paint.Style.STROKE
        frame.strokeWidth = context.dp(1.2f)
        frame.color = Deck.DIM
        r.inset(frame.strokeWidth / 2, frame.strokeWidth / 2)
        c.drawRoundRect(r, radius, radius, frame)
        val pad = context.dp(9f)
        big.color = if (hot) Deck.REC else Theme.FG
        c.drawText(title, pad, pad + big.textSize * 0.82f, big)
        small.color = Theme.FG
        val baseline = height - pad
        c.drawText(line, pad, baseline, small)
        box?.let { b ->
            val tw = small.measureText(b)
            val bx = width - pad - tw - context.dp(6f)
            frame.style = Paint.Style.FILL
            frame.color = Theme.FG
            c.drawRoundRect(bx, baseline - small.textSize, width - pad, baseline + context.dp(3f), context.dp(3f), context.dp(3f), frame)
            small.color = Color.BLACK
            c.drawText(b, bx + context.dp(3f), baseline, small)
        }
    }
}

/**
 * The Journal's platter, after the TP-7's: a dark disc with a hairline across, two small labels and a hub.
 * It turns with [angle]: a notch per note on the wheel, and steadily while recording or playing.
 */
class Platter(context: Context) : View(context) {
    var angle = 0f
        set(v) { field = v; invalidate() }
    var top = "16 / 16"
    var bottom = ""
        set(v) { if (field != v) { field = v; invalidate() } }

    private val disc = Paint(Paint.ANTI_ALIAS_FLAG)
    private val hub = Paint(Paint.ANTI_ALIAS_FLAG)
    private val line = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = Theme.font(500)
        textSize = context.dp(12f)
        textAlign = Paint.Align.CENTER
        color = Color.parseColor("#9aa0a8")
    }
    private var shaded = 0f

    override fun onDraw(c: Canvas) {
        val cx = width / 2f
        val cy = height / 2f
        val rr = min(width, height) / 2f - context.dp(2f)
        if (shaded != rr) {
            shaded = rr
            disc.shader = RadialGradient(cx - rr * 0.25f, cy - rr * 0.3f, rr * 1.3f, Color.parseColor("#1d1e22"), Color.parseColor("#0c0d0f"), Shader.TileMode.CLAMP)
        }
        c.drawCircle(cx, cy, rr, disc)
        line.strokeWidth = context.dp(1.4f)
        line.color = Deck.HAIR
        c.drawCircle(cx, cy, rr, line)
        c.save()
        c.rotate(angle, cx, cy)
        // The hairline, from upper right to lower left, broken by the hub.
        line.strokeWidth = context.dp(1f)
        line.color = Color.parseColor("#5a5e66")
        val d = (sqrt(2f) / 2f)
        for (sign in listOf(1f, -1f)) {
            val a = rr * 0.33f
            val b = rr * 0.9f
            c.drawLine(cx + sign * a * d, cy - sign * a * d, cx + sign * b * d, cy - sign * b * d, line)
        }
        // The labels, across the line.
        for ((label, sign) in listOf(top to -1f, bottom to 1f)) {
            if (label.isEmpty()) continue
            val x = cx + sign * rr * 0.42f
            val y = cy + sign * rr * 0.42f
            c.save()
            c.rotate(45f, x, y)
            c.drawText(label, x, y + text.textSize / 3, text)
            c.restore()
        }
        // The hub, with its three screws.
        hub.color = Color.parseColor("#050506")
        c.drawCircle(cx, cy, rr * 0.27f, hub)
        hub.color = Deck.DIM
        for (k in 0 until 3) {
            val a = Math.toRadians((k * 120 - 90).toDouble())
            c.drawCircle(cx + (rr * 0.15f * cos(a)).toFloat(), cy + (rr * 0.15f * sin(a)).toFloat(), context.dp(2.2f), hub)
        }
        c.restore()
    }
}

/**
 * The Recorder's reels: two open reels with the tape between them over a head. [fraction] of the tape has
 * wound from the left reel to the right; [advance] turns them at tape speed (the fuller reel slower).
 */
class Reels(context: Context) : View(context) {
    var fraction = 0f
        set(v) { field = v.coerceIn(0f, 1f); invalidate() }
    /** Recording: the hubs' spokes in red. */
    var hot = false
        set(v) { if (field != v) { field = v; invalidate() } }
    private var angleL = 0f
    private var angleR = 0f

    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val spoke = Paint(Paint.ANTI_ALIAS_FLAG).apply { strokeCap = Paint.Cap.ROUND }
    private val tape = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }

    private fun geometry(): FloatArray {
        val rf = min(width * 0.215f, height * 0.44f)
        return floatArrayOf(width * 0.265f, width * 0.735f, height * 0.44f, rf)
    }

    private fun packs(rf: Float): Pair<Float, Float> {
        val hub = rf * HUB
        val max = rf * 0.95f
        val area = max * max - hub * hub
        return sqrt(hub * hub + area * (1 - fraction)) to sqrt(hub * hub + area * fraction)
    }

    /** Turns the reels for `dt` seconds of tape running. */
    fun advance(dt: Float) {
        if (width == 0) return
        val rf = geometry()[3]
        val (pl, pr) = packs(rf)
        val v = rf * 1.6f // tape speed: about a turn a second at mid pack
        angleL += Math.toDegrees((v * dt / pl).toDouble()).toFloat()
        angleR += Math.toDegrees((v * dt / pr).toDouble()).toFloat()
        invalidate()
    }

    override fun onDraw(c: Canvas) {
        val g = geometry()
        val (lx, rx, cy, rf) = listOf(g[0], g[1], g[2], g[3])
        val (pl, pr) = packs(rf)
        // The tape: off the bottom of each pack, round two guides, across the head.
        val headY = cy + rf + context.dp(16f)
        val gl = width * 0.42f
        val gr = width * 0.58f
        tape.strokeWidth = context.dp(1.6f)
        tape.color = Color.parseColor("#4a4d54")
        c.drawLine(lx, cy + pl, gl, headY, tape)
        c.drawLine(gl, headY, gr, headY, tape)
        c.drawLine(gr, headY, rx, cy + pr, tape)
        p.style = Paint.Style.FILL
        p.color = Deck.DIM
        c.drawCircle(gl, headY, context.dp(3.5f), p)
        c.drawCircle(gr, headY, context.dp(3.5f), p)
        p.color = Color.parseColor("#c9ced6")
        c.drawRoundRect(width / 2f - context.dp(11f), headY - context.dp(2f), width / 2f + context.dp(11f), headY + context.dp(8f), context.dp(2.5f), context.dp(2.5f), p)
        reel(c, lx, cy, rf, pl, angleL)
        reel(c, rx, cy, rf, pr, angleR)
    }

    private fun reel(c: Canvas, x: Float, y: Float, rf: Float, pack: Float, angle: Float) {
        // The flange: an outline, as on a reel you can see through.
        p.style = Paint.Style.STROKE
        p.strokeWidth = context.dp(1.5f)
        p.color = Deck.HAIR
        c.drawCircle(x, y, rf, p)
        // The tape wound on it, with a few windings showing.
        p.style = Paint.Style.FILL
        p.color = Deck.PACK
        c.drawCircle(x, y, pack, p)
        p.style = Paint.Style.STROKE
        p.strokeWidth = context.dp(0.8f)
        p.color = Color.parseColor("#24262a")
        val hub = rf * HUB
        var w = hub + (pack - hub) * 0.33f
        while (w < pack - context.dp(2f)) {
            c.drawCircle(x, y, w, p)
            w += (pack - hub) * 0.33f
        }
        // The hub, its three spokes turning.
        p.style = Paint.Style.FILL
        p.color = Color.BLACK
        c.drawCircle(x, y, hub, p)
        p.style = Paint.Style.STROKE
        p.strokeWidth = context.dp(1.2f)
        p.color = Deck.DIM
        c.drawCircle(x, y, hub, p)
        spoke.color = if (hot) Deck.REC else Theme.FG
        spoke.strokeWidth = context.dp(5f)
        for (k in 0 until 3) {
            val a = Math.toRadians((angle + k * 120).toDouble())
            val ca = cos(a).toFloat()
            val sa = sin(a).toFloat()
            c.drawLine(x + ca * hub * 0.38f, y + sa * hub * 0.38f, x + ca * hub * 0.8f, y + sa * hub * 0.8f, spoke)
        }
        p.style = Paint.Style.FILL
        p.color = spoke.color
        c.drawCircle(x, y, context.dp(3f), p)
    }

    private companion object {
        const val HUB = 0.42f
    }
}
