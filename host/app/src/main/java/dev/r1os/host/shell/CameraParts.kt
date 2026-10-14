package dev.r1os.host.shell

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.view.View
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin

/**
 * The camera's parts (ADR-0046), after teenage engineering's instruments (the OP-1's coloured encoders, the TP-7's
 * keys): coloured knobs for the dials, flat keys and the viewfinder's marks. They sit on dark bands over the picture,
 * so their colours are the object's own, light on dark, whatever the look.
 */
object Cam {
    /** The dials' colours, as on the OP-1: one per knob. */
    val FILM = Color.parseColor("#ff6b1a")
    val EXP = Color.parseColor("#3f7cff")
    val WB = Color.parseColor("#20c472")
    val ZOOM = Color.parseColor("#e9e9e6")
    /** The motor turning: the lamp, and the flip key while it turns. */
    val TURN = Color.parseColor("#ffc933")
    /** Words on the bands. */
    val INK = Color.parseColor("#f2f2f2")
    val QUIET = Color.parseColor("#9aa0a8")

    val MONO: Typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
    val MONO_REG: Typeface = Typeface.MONOSPACE
}

/**
 * A dial: its name above, a knob (a coloured cap with a notch, in a ring of ticks lit up to its value), its value
 * below in a monospaced face. The one the wheel turns ([on]) has its ticks lit and its value in its colour.
 */
class Knob(context: Context, private val name: String, private val color: Int) : View(context) {
    var value = ""
        set(v) { if (field != v) { field = v; invalidate() } }
    /** 0..1: where the notch points, from lower left round to lower right. */
    var at = 0f
        set(v) { if (field != v) { field = v.coerceIn(0f, 1f); invalidate() } }
    var on = false
        set(v) { if (field != v) { field = v; invalidate() } }

    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val label = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = Theme.font(600)
        textSize = context.dp(11f)
        textAlign = Paint.Align.CENTER
    }
    private val mono = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = Cam.MONO
        textSize = context.dp(13f)
        textAlign = Paint.Align.CENTER
    }

    override fun onDraw(c: Canvas) {
        val cx = width / 2f
        label.color = if (on) Cam.INK else Cam.QUIET
        c.drawText(name, cx, context.dp(11f), label)
        val cy = context.dp(14f) + (height - context.dp(14f) - context.dp(18f)) / 2f
        val r = min(width * 0.5f, (height - context.dp(32f)) / 2f) - context.dp(6f)
        // The ticks round it: 270°, from lower left, lit up to the value.
        val n = 11
        for (i in 0 until n) {
            val f = i / (n - 1f)
            val a = Math.toRadians((135 + f * 270).toDouble())
            val lit = f <= at + 0.001f
            p.color = when {
                on && lit -> color
                lit -> Cam.QUIET
                else -> Theme.withAlpha(Cam.QUIET, 0.35f)
            }
            c.drawCircle(cx + (r + context.dp(4.5f)) * cos(a).toFloat(), cy + (r + context.dp(4.5f)) * sin(a).toFloat(), context.dp(1.3f), p)
        }
        // The knob: a dark body, the coloured cap, the notch.
        val body = r - context.dp(1f)
        p.color = Deck.KEY
        c.drawCircle(cx, cy, body, p)
        val cap = body * 0.74f
        p.color = if (on) color else Theme.withAlpha(color, 0.78f)
        c.drawCircle(cx, cy, cap, p)
        val a = Math.toRadians((135 + at * 270).toDouble())
        p.color = Color.parseColor("#101114")
        p.strokeWidth = context.dp(2.4f)
        p.strokeCap = Paint.Cap.ROUND
        c.drawLine(cx + cap * 0.25f * cos(a).toFloat(), cy + cap * 0.25f * sin(a).toFloat(), cx + cap * 0.86f * cos(a).toFloat(), cy + cap * 0.86f * sin(a).toFloat(), p)
        mono.color = if (on) color else Cam.INK
        c.drawText(value, cx, height - context.dp(3f), mono)
    }
}

/** A flat key, as on the TP-7: dark, its top edge catching the light, a glyph (or a word) and a word under it. */
class CamKey(context: Context, private val word: String, private val glyph: Glyph? = null) : View(context) {
    enum class Glyph { FLIP, SET }

    var lit = false
        set(v) { if (field != v) { field = v; invalidate() } }
    var color = Theme.accent

    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Theme.font(600); textSize = context.dp(11f); textAlign = Paint.Align.CENTER }
    private val r = RectF()
    private val path = Path()

    override fun onDraw(c: Canvas) {
        val radius = context.dp(9f)
        r.set(0f, 0f, width.toFloat(), height.toFloat())
        p.style = Paint.Style.FILL
        p.color = if (lit) color else Deck.KEY
        c.drawRoundRect(r, radius, radius, p)
        if (!lit) {
            p.color = Deck.KEY_TOP
            c.drawRoundRect(RectF(r.left + radius, r.top, r.right - radius, r.top + context.dp(1f)), 1f, 1f, p)
        }
        val ink = if (lit) Theme.ON_ACCENT else Deck.GLYPH
        val g = glyph
        if (g == null) {
            text.color = ink
            text.textSize = context.dp(12f)
            c.drawText(word, r.centerX(), r.centerY() + context.dp(4f), text)
            return
        }
        val cy = r.centerY() - context.dp(6f)
        p.color = ink
        p.style = Paint.Style.STROKE
        p.strokeWidth = context.dp(1.8f)
        p.strokeCap = Paint.Cap.ROUND
        p.strokeJoin = Paint.Join.ROUND
        val s = context.dp(7f)
        when (g) {
            // Two arrows, up and down: the module turning over.
            Glyph.FLIP -> {
                val cx = r.centerX()
                for ((dx, up) in listOf(-s * 0.6f to true, s * 0.6f to false)) {
                    val x = cx + dx
                    c.drawLine(x, cy - s, x, cy + s, p)
                    path.reset()
                    val tip = if (up) cy - s else cy + s
                    val back = if (up) tip + s * 0.6f else tip - s * 0.6f
                    path.moveTo(x - s * 0.5f, back)
                    path.lineTo(x, tip)
                    path.lineTo(x + s * 0.5f, back)
                    c.drawPath(path, p)
                }
            }
            // Three sliders.
            Glyph.SET -> {
                val cx = r.centerX()
                for ((k, knob) in listOf(-1f to 0.3f, 0f to -0.4f, 1f to 0.1f)) {
                    val y = cy + k * s * 0.75f
                    c.drawLine(cx - s, y, cx + s, y, p)
                    p.style = Paint.Style.FILL
                    c.drawCircle(cx + knob * s, y, context.dp(2.4f), p)
                    p.style = Paint.Style.STROKE
                }
            }
        }
        p.style = Paint.Style.FILL
        text.color = if (lit) Theme.withAlpha(Theme.ON_ACCENT, 0.7f) else Cam.QUIET
        text.textSize = context.dp(11f)
        c.drawText(word, r.centerX(), r.bottom - context.dp(8f), text)
    }
}

/**
 * The viewfinder's marks: the thirds over the whole frame (if on), a centre cross, the timer's count, and corner
 * brackets round what's clear of the bands ([top], [bottom]: how much of the frame they cover).
 */
class FinderMarks(context: Context) : View(context) {
    var grid = false
        set(v) { if (field != v) { field = v; invalidate() } }
    var top = 0f
        set(v) { if (field != v) { field = v; invalidate() } }
    var bottom = 0f
        set(v) { if (field != v) { field = v; invalidate() } }
    /** The timer's seconds left, big in the middle; 0: none. */
    var count = 0
        set(v) { if (field != v) { field = v; invalidate() } }

    private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { strokeCap = Paint.Cap.SQUARE }
    private val big = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Cam.MONO; textAlign = Paint.Align.CENTER; color = Color.WHITE }

    override fun onDraw(c: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        if (grid) {
            p.color = Color.argb(70, 255, 255, 255)
            p.strokeWidth = context.dp(1f)
            for (k in 1..2) {
                c.drawLine(w * k / 3f, 0f, w * k / 3f, h, p)
                c.drawLine(0f, h * k / 3f, w, h * k / 3f, p)
            }
        }
        p.color = Color.argb(200, 255, 255, 255)
        p.strokeWidth = context.dp(1.6f)
        val m = context.dp(12f)
        val l = context.dp(16f)
        for ((x, sx) in listOf(m to 1f, w - m to -1f)) {
            for ((y, sy) in listOf(top + m to 1f, h - bottom - m to -1f)) {
                c.drawLine(x, y, x + sx * l, y, p)
                c.drawLine(x, y, x, y + sy * l, p)
            }
        }
        val cs = context.dp(6f)
        p.strokeWidth = context.dp(1.2f)
        p.color = Color.argb(150, 255, 255, 255)
        c.drawLine(w / 2 - cs, h / 2, w / 2 + cs, h / 2, p)
        c.drawLine(w / 2, h / 2 - cs, w / 2, h / 2 + cs, p)
        if (count > 0) {
            big.textSize = context.dp(96f)
            big.setShadowLayer(context.dp(12f), 0f, 0f, Color.argb(140, 0, 0, 0))
            c.drawText("$count", w / 2, h / 2 + big.textSize * 0.36f, big)
        }
    }
}
