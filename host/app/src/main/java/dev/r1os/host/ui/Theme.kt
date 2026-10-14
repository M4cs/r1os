package dev.r1os.host.ui

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.util.TypedValue
import android.view.animation.PathInterpolator
import android.widget.TextView

/**
 * The shell's look, ported from shell/src/styles.css (ADR-0017). On the R1 (480×640 at density 180)
 * one CSS px is one dp, so the web shell's sizes carry over as dp.
 *
 * Yours to change (ADR-0043): dark or light, the accent, the font ([setLook], from r1d's look). Views take their
 * colors when they're made, so a change builds the shell again (Shell's rebuild).
 */
object Theme {
    /** Light mode: the shell's colors are the light set. */
    var light = false
        private set
    var BG = Color.BLACK
        private set
    var FG = Color.parseColor("#f2f2f2")
        private set
    var MUTED = Color.parseColor("#8a8f98")
        private set
    var LINE = Color.parseColor("#22252b")
        private set
    val BAD = Color.parseColor("#ff5a5a")
    val OK = Color.parseColor("#3ddc84")
    var CARD = Color.parseColor("#1a1c21")
        private set
    var FOCUS = Color.parseColor("#15171b")
        private set
    /** Raised over everything: a toast, a banner, an agent's reply over an app. */
    var SURFACE = Color.argb(245, 24, 26, 31)
        private set
    /** Set in: fields, the designer's boxes, a tile. */
    var PANEL = Color.parseColor("#121418")
        private set
    /** The launcher's sheet over home. */
    var SHEET = Color.parseColor("#0e0f12")
        private set
    /** What isn't chosen or on: dots, swatches, steps to come. */
    var SUBTLE = Color.parseColor("#2a2d33")
        private set
    /** A plain row's icon. */
    var ICON = Color.parseColor("#c9ced6")
        private set
    /** Words that went wrong, in a sentence (heard words, on the voice overlay). */
    var BAD_TEXT = Color.parseColor("#ffb4b4")
        private set

    /** The app palette's named accents (packages/runtime styles.css). */
    val COLORS: Map<String, Int> = mapOf(
        "orange" to "#ff8a3d", "amber" to "#f5b83d", "yellow" to "#f0d04a", "lime" to "#a3d65c",
        "green" to "#3ddc84", "teal" to "#2fc4a8", "cyan" to "#3cc8e0", "blue" to "#4ea1ff",
        "indigo" to "#8b8cf8", "purple" to "#b07cf6", "pink" to "#f06aa8", "red" to "#f2555a",
    ).mapValues { Color.parseColor(it.value) }

    /** A named accent, or a colour as `#rrggbb` (ADR-0043: any colour); else orange. */
    fun colorOf(name: String?): Int {
        val n = name.orEmpty()
        if (n.startsWith("#")) runCatching { return Color.parseColor(n) }
        return COLORS[n] ?: COLORS.getValue("orange")
    }

    /** A colour as `#rrggbb`. */
    fun hex(c: Int): String = String.format(java.util.Locale.ROOT, "#%06x", c and 0xffffff)

    /** A colour from hue (0–360), saturation and lightness (0–1). */
    fun hsl(h: Float, s: Float, l: Float): Int = hslToRgb(floatArrayOf(((h % 360f) + 360f) % 360f, s.coerceIn(0f, 1f), l.coerceIn(0f, 1f)))

    /** A colour's hue (0–360), saturation and lightness (0–1). */
    fun toHsl(c: Int): FloatArray = FloatArray(3).also { androidRgbToHsl(c, it) }

    /** The device's accent (the launcher, settings, the system's own screens); agents' screens keep their colour. */
    var accentName = "orange"
        private set
    var accent = colorOf("orange")
        private set

    /** The web shell's cubic-bezier(0.2, 0.8, 0.2, 1): quick out, gentle settle. */
    val SETTLE = PathInterpolator(0.2f, 0.8f, 0.2f, 1f)
    /** cubic-bezier(0.2, 1.4, 0.5, 1): a small overshoot, for things popping in. */
    val POP = PathInterpolator(0.2f, 1.4f, 0.5f, 1f)
    val EASE_IN_OUT = PathInterpolator(0.42f, 0f, 0.58f, 1f)
    val EASE_OUT = PathInterpolator(0f, 0f, 0.58f, 1f)

    /** Text on an accent fill (a focused row, a primary button): near-black, like ink on orange. */
    val ON_ACCENT = Color.parseColor("#0b0b0c")

    private var regular: Typeface? = null
    private var medium: Typeface? = null
    private var bold: Typeface? = null

    /** The shell's typeface (ADR-0043): Rubik, r1os's own (ADR-0021), or one of the R1's system fonts. */
    var fontName = "rubik"
        private set
    /** The system font families a [fontName] stands for (the image's fonts.xml). */
    val FAMILIES = mapOf(
        "roboto" to "sans-serif", "flex" to "roboto-flex", "source" to "source-sans-pro", "serif" to "serif",
        "mono" to "monospace", "typewriter" to "serif-monospace", "casual" to "casual", "script" to "cursive",
        "smallcaps" to "sans-serif-smallcaps",
    )
    private val faces = HashMap<String, Typeface>()

    /** Loads Rubik, r1os's typeface (ADR-0021). Before any view is made. */
    fun init(context: Context) {
        regular = context.resources.getFont(dev.r1os.host.R.font.rubik_regular)
        medium = context.resources.getFont(dev.r1os.host.R.font.rubik_medium)
        bold = context.resources.getFont(dev.r1os.host.R.font.rubik_bold)
    }

    /** Dark or light, the accent and the font: the look's (crate::look). Before views are made. */
    fun setLook(lightMode: Boolean, accentColor: String, font: String) {
        light = lightMode
        accentName = if (accentColor in COLORS || accentColor.matches(Regex("#[0-9a-fA-F]{6}"))) accentColor else "orange"
        accent = colorOf(accentName)
        fontName = if (font == "rubik" || font in FAMILIES) font else "rubik"
        if (light) {
            BG = Color.parseColor("#f3f2ef"); FG = Color.parseColor("#16171a"); MUTED = Color.parseColor("#6c717a")
            LINE = Color.parseColor("#dcdcd8"); CARD = Color.WHITE; FOCUS = Color.parseColor("#e7e6e2")
            SURFACE = Color.argb(250, 255, 255, 255); PANEL = Color.parseColor("#e8e7e3"); SHEET = Color.parseColor("#fbfaf8")
            SUBTLE = Color.parseColor("#c9c8c3"); ICON = Color.parseColor("#4b5059"); BAD_TEXT = BAD
        } else {
            BG = Color.BLACK; FG = Color.parseColor("#f2f2f2"); MUTED = Color.parseColor("#8a8f98")
            LINE = Color.parseColor("#22252b"); CARD = Color.parseColor("#1a1c21"); FOCUS = Color.parseColor("#15171b")
            SURFACE = Color.argb(245, 24, 26, 31); PANEL = Color.parseColor("#121418"); SHEET = Color.parseColor("#0e0f12")
            SUBTLE = Color.parseColor("#2a2d33"); ICON = Color.parseColor("#c9ced6"); BAD_TEXT = Color.parseColor("#ffb4b4")
        }
    }

    /** The shell's typeface at a weight: bold from 650, medium from 500, else regular. */
    fun font(weight: Int): Typeface = fontOf(fontName, weight)

    /** A typeface by its look name (Settings › Look shows each in its own). */
    fun fontOf(name: String, weight: Int): Typeface {
        if (name !in FAMILIES) {
            return when {
                weight >= 650 -> bold
                weight >= 500 -> medium
                else -> regular
            } ?: Typeface.create(Typeface.DEFAULT, weight, false)
        }
        val w = when {
            weight >= 650 -> 700
            weight >= 500 -> 500
            else -> 400
        }
        return faces.getOrPut("$name/$w") { Typeface.create(Typeface.create(FAMILIES.getValue(name), Typeface.NORMAL), w, false) }
    }

    /** `c` moved toward `to` by `t` (0–1). */
    fun mix(c: Int, to: Int, t: Float): Int = Color.rgb(
        (Color.red(c) + (Color.red(to) - Color.red(c)) * t).toInt(),
        (Color.green(c) + (Color.green(to) - Color.green(c)) * t).toInt(),
        (Color.blue(c) + (Color.blue(to) - Color.blue(c)) * t).toInt(),
    )

    /** `c` with its hue turned by `deg`, keeping saturation and lightness. */
    fun hue(c: Int, deg: Float): Int {
        val hsl = FloatArray(3)
        androidRgbToHsl(c, hsl)
        hsl[0] = ((hsl[0] + deg) % 360f + 360f) % 360f
        return hslToRgb(hsl)
    }

    fun withAlpha(c: Int, a: Float): Int = Color.argb((a * 255).toInt().coerceIn(0, 255), Color.red(c), Color.green(c), Color.blue(c))

    private fun androidRgbToHsl(c: Int, out: FloatArray) {
        val r = Color.red(c) / 255f
        val g = Color.green(c) / 255f
        val b = Color.blue(c) / 255f
        val max = maxOf(r, g, b)
        val min = minOf(r, g, b)
        val l = (max + min) / 2
        val d = max - min
        val s = if (d == 0f) 0f else d / (1 - Math.abs(2 * l - 1))
        var h = when {
            d == 0f -> 0f
            max == r -> ((g - b) / d) % 6
            max == g -> (b - r) / d + 2
            else -> (r - g) / d + 4
        } * 60
        if (h < 0) h += 360
        out[0] = h
        out[1] = s
        out[2] = l
    }

    private fun hslToRgb(hsl: FloatArray): Int {
        val (h, s, l) = Triple(hsl[0], hsl[1], hsl[2])
        val c = (1 - Math.abs(2 * l - 1)) * s
        val x = c * (1 - Math.abs((h / 60) % 2 - 1))
        val m = l - c / 2
        val (r, g, b) = when {
            h < 60 -> Triple(c, x, 0f)
            h < 120 -> Triple(x, c, 0f)
            h < 180 -> Triple(0f, c, x)
            h < 240 -> Triple(0f, x, c)
            h < 300 -> Triple(x, 0f, c)
            else -> Triple(c, 0f, x)
        }
        return Color.rgb(((r + m) * 255).toInt(), ((g + m) * 255).toInt(), ((b + m) * 255).toInt())
    }
}

/**
 * Everything the shell shows is lowercase (ADR-0021; Max, 2026-10-09: "lowercase everywhere, even on
 * app names … and agent names"). A display-only transformation: the text itself (an agent's name, an
 * app's) keeps its case wherever it's stored or sent. Chat messages and your spoken words opt out
 * ([keepCase]): they're what was said.
 */
object Lowercase : android.text.method.TransformationMethod {
    override fun getTransformation(source: CharSequence?, view: android.view.View?): CharSequence? = source?.toString()?.lowercase()
    override fun onFocusChanged(view: android.view.View?, sourceText: CharSequence?, focused: Boolean, direction: Int, previouslyFocusedRect: android.graphics.Rect?) {}
}

/** Shows this text as written (chat messages, spoken words). */
fun TextView.keepCase(): TextView = apply { transformationMethod = null }

fun Context.dp(v: Float): Float = v * resources.displayMetrics.density
fun Context.dpi(v: Float): Int = Math.round(dp(v))

/** A TextView in the shell's style: Rubik, lowercase. */
fun Context.text(size: Float, weight: Int = 500, color: Int = Theme.FG): TextView = TextView(this).apply {
    setTextSize(TypedValue.COMPLEX_UNIT_DIP, size)
    typeface = Theme.font(weight)
    setTextColor(color)
    includeFontPadding = false
    transformationMethod = Lowercase
}

/**
 * A trace section around [block], closed however it ends (a `return` too): r1os's own costs show by name in
 * `atrace -a dev.r1os.host` and Perfetto (T-041).
 */
inline fun <T> traced(name: String, block: () -> T): T {
    android.os.Trace.beginSection(name)
    try {
        return block()
    } finally {
        android.os.Trace.endSection()
    }
}
