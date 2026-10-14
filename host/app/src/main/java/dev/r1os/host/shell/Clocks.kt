package dev.r1os.host.shell

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Canvas
import android.graphics.Paint
import android.util.TypedValue
import android.view.View
import android.widget.TextClock
import android.widget.TextView
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.text
import java.util.Calendar
import kotlin.math.cos
import kotlin.math.sin

/**
 * The glance screen's clock faces (ADR-0043): the look's `glance.clock`. The text ones are TextClocks in a style (the
 * morph to home scales them onto home's clock); the time in words and the dial draw themselves, once a minute, from
 * the system's minute tick (which stops while the R1 sleeps).
 */
object Clocks {
    /** Faces that are one line of digits: the morph scales them onto home's clock (the others fade). */
    fun textual(style: String) = style in listOf("bold", "light", "condensed", "serif", "mono")

    fun face(context: Context, style: String, size: Float = 1f): View = when (style) {
        "words" -> WordsClock(context, size)
        "dial" -> DialClock(context)
        else -> TextClock(context).apply {
            val stacked = style == "stacked"
            format12Hour = if (stacked) "hh\nmm" else "h:mm"
            format24Hour = if (stacked) "HH\nmm" else "H:mm"
            setTextColor(Theme.FG)
            includeFontPadding = false
            fontFeatureSettings = "tnum"
            val sp = when (style) {
                "light" -> { typeface = flex("'wght' 200"); 78f }
                "condensed" -> { typeface = flex("'wdth' 25, 'wght' 720"); letterSpacing = -0.01f; 100f }
                "stacked" -> { typeface = Theme.font(700); setLineSpacing(0f, 0.84f); 82f }
                "serif" -> { typeface = Theme.fontOf("serif", 400); 70f }
                "mono" -> { typeface = Theme.fontOf("mono", 400); 62f }
                else -> { typeface = Theme.font(650); letterSpacing = -0.01f; 64f }
            }
            setTextSize(TypedValue.COMPLEX_UNIT_DIP, sp * size)
        }
    }

    /** Roboto Flex at these axes, from the system's variable font file (a family's Typeface keeps only its weight). */
    private val flexes = HashMap<String, android.graphics.Typeface>()

    fun flex(settings: String): android.graphics.Typeface = flexes.getOrPut(settings) {
        runCatching {
            android.graphics.Typeface.Builder(java.io.File("/system/fonts/RobotoFlex-Regular.ttf")).setFontVariationSettings(settings).build()
        }.getOrNull() ?: Theme.fontOf("flex", 700)
    }

    /** "ten past four", "quarter to five", "noon": the nearest five minutes. */
    fun words(hour: Int, minute: Int): String {
        val names = listOf("twelve", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven")
        var m = Math.round(minute / 5f) * 5
        var h = hour
        if (m == 60) { m = 0; h = (h + 1) % 24 }
        fun name(h24: Int) = when (h24) {
            0 -> "midnight"
            12 -> "noon"
            else -> names[h24 % 12]
        }
        val next = (h + 1) % 24
        return when (m) {
            0 -> if (h == 0 || h == 12) name(h) else "${names[h % 12]} o'clock"
            15 -> "quarter past ${name(h)}"
            30 -> "half past ${name(h)}"
            45 -> "quarter to ${name(next)}"
            in 5..25 -> "${number(m)} past ${name(h)}"
            else -> "${number(60 - m)} to ${name(next)}"
        }
    }

    private fun number(n: Int) = when (n) {
        5 -> "five"
        10 -> "ten"
        20 -> "twenty"
        25 -> "twenty-five"
        else -> n.toString()
    }
}

/** Something that changes with the minute: the system's tick, while it's on screen. */
private class MinuteTick(private val view: View, private val onTick: () -> Unit) : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) = onTick()

    fun attach() {
        view.context.registerReceiver(this, IntentFilter().apply {
            addAction(Intent.ACTION_TIME_TICK); addAction(Intent.ACTION_TIME_CHANGED); addAction(Intent.ACTION_TIMEZONE_CHANGED)
        })
        onTick()
    }

    fun detach() {
        runCatching { view.context.unregisterReceiver(this) }
    }
}

/** The time in words, big and bold. */
private class WordsClock(context: Context, size: Float) : TextView(context) {
    private val tick = MinuteTick(this) {
        val now = Calendar.getInstance()
        text = Clocks.words(now.get(Calendar.HOUR_OF_DAY), now.get(Calendar.MINUTE))
    }

    init {
        val t = context.text(40f * size, 700)
        typeface = t.typeface
        transformationMethod = t.transformationMethod
        setTextSize(TypedValue.COMPLEX_UNIT_DIP, 40f * size)
        setTextColor(Theme.FG)
        includeFontPadding = false
        setLineSpacing(0f, 0.92f)
        maxWidth = context.resources.displayMetrics.widthPixels - context.dp(40f).toInt()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        tick.attach()
    }

    override fun onDetachedFromWindow() {
        tick.detach()
        super.onDetachedFromWindow()
    }
}

/** A clock face: twelve marks, the hour hand in the text's colour and the minute hand in the accent. */
private class DialClock(context: Context) : View(context) {
    private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { strokeCap = Paint.Cap.ROUND }
    private var hour = 0
    private var minute = 0
    private val tick = MinuteTick(this) {
        val now = Calendar.getInstance()
        hour = now.get(Calendar.HOUR)
        minute = now.get(Calendar.MINUTE)
        invalidate()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val s = context.dp(132f).toInt()
        setMeasuredDimension(s, s)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        tick.attach()
    }

    override fun onDetachedFromWindow() {
        tick.detach()
        super.onDetachedFromWindow()
    }

    override fun onDraw(c: Canvas) {
        val r = width / 2f
        val k = context.dp(1f)
        p.style = Paint.Style.STROKE
        for (i in 0 until 12) {
            val a = Math.toRadians(i * 30.0)
            val major = i % 3 == 0
            p.color = if (major) Theme.FG else Theme.MUTED
            p.strokeWidth = (if (major) 4f else 2.5f) * k
            val r0 = r - (if (major) 14f else 9f) * k
            val r1 = r - 3f * k
            c.drawLine(r + sin(a).toFloat() * r0, r - cos(a).toFloat() * r0, r + sin(a).toFloat() * r1, r - cos(a).toFloat() * r1, p)
        }
        fun hand(turns: Double, length: Float, width: Float, color: Int) {
            val a = turns * 2 * Math.PI
            p.color = color
            p.strokeWidth = width * k
            c.drawLine(r, r, r + sin(a).toFloat() * length, r - cos(a).toFloat() * length, p)
        }
        hand((hour + minute / 60.0) / 12.0, r * 0.5f, 7f, Theme.FG)
        hand(minute / 60.0, r * 0.78f, 4.5f, Theme.accent)
        p.style = Paint.Style.FILL
        p.color = Theme.accent
        c.drawCircle(r, r, 5f * k, p)
    }
}
