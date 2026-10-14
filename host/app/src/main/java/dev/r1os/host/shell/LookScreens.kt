package dev.r1os.host.shell

import android.content.Context
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Shader
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.camera.Photos
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Backdrop
import dev.r1os.host.ui.Look
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject

// Settings › Look (ADR-0043): the R1's look, picked on the wheel, a page for each part. Each pick sends the change to
// r1d (crate::look), which keeps it and sends it back as a `look` event; the shell then builds itself again in it and
// reopens these pages on the same row, so what you picked shows at once, everywhere.

/** Sends a change to the look (only what changes). */
private fun change(c: JSONObject) = R1d.put("/api/look", c)

private fun glanceChange(key: String, value: Any) = change(JSONObject().put("glance", JSONObject().put(key, value)))

/** A choice among several: a ticked circle on the chosen one, tinted in [tint] (the colour itself, for colours). */
private fun FocusList.choice(label: String, on: Boolean, sub: String? = null, tint: Int? = null, font: android.graphics.Typeface? = null, pick: () -> Unit) =
    item(if (on) "check-circle" else "circle", label, tint ?: (if (on) accent else null), sub = sub, font = font) { if (!on) pick() }

/** A row into a page: an icon, a name, what's chosen there now. */
private fun FocusList.page(icon: String, label: String, sub: String, target: String, shell: Shell) =
    item(icon, label, sub = sub, trailing = "›") { shell.open(Target.of("settings", "page" to target)) }

/** Colours in a row, as small round swatches (a palette, a soft gradient's colours). */
private fun swatches(context: Context, colors: List<Int>, size: Float = 18f) = LinearLayout(context).apply {
    orientation = LinearLayout.HORIZONTAL
    for (c in colors) {
        addView(View(context).apply { background = roundRect(c, context.dp(size)) },
            LinearLayout.LayoutParams(context.dpi(size), context.dpi(size)).apply { leftMargin = context.dpi(4f) })
    }
}

/** Settings › Look: a page for each part of it. */
class LookScreen(shell: Shell) : ListPage(shell, "Look") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        page("palette", "colours", "${Look.json.optString("mode", "dark")} · ${Theme.accentName} accent", "look-colors", shell)
        page("text-aa", "font", FONT_LABEL[Theme.fontName] ?: Theme.fontName, "look-font", shell)
        section("screens")
        page("clock", "glance screen", "${CLOCK_LABEL[Look.clock] ?: Look.clock} clock · agent ${PLACE_LABEL[Look.agentPlace]}", "look-glance", shell)
        page("image", "glance background", describe(Look.background("glance")), "look-bg-glance", shell)
        page("image", "home background", describe(Look.background("home")), "look-bg-home", shell)
    }

    companion object {
        val FONTS = listOf("rubik", "roboto", "flex", "source", "serif", "mono", "typewriter", "casual", "script", "smallcaps")
        val FONT_LABEL = mapOf(
            "rubik" to "rubik", "roboto" to "roboto", "flex" to "roboto flex", "source" to "source sans", "serif" to "noto serif",
            "mono" to "mono", "typewriter" to "typewriter", "casual" to "handwritten", "script" to "script", "smallcaps" to "small caps",
        )
        val CLOCK_LABEL = mapOf(
            "bold" to "bold", "light" to "light", "condensed" to "tall", "stacked" to "stacked", "serif" to "serif",
            "mono" to "mono", "words" to "in words", "dial" to "a dial",
        )
        val PLACE_LABEL = mapOf("center" to "in the middle", "left" to "on the left", "right" to "on the right", "hidden" to "hidden")
        /** Soft gradients' ready-made colours (crate::look PALETTES). */
        val PALETTES = listOf(
            "sunset" to listOf("#ff8a3d", "#f06aa8", "#6b4cff"), "lagoon" to listOf("#2fc4a8", "#3cc8e0", "#4e6bff"),
            "aurora" to listOf("#3ddc84", "#8b8cf8", "#1d4ed8"), "peach" to listOf("#ffb38a", "#ff6f91", "#ffc75f"),
            "dusk" to listOf("#4ea1ff", "#b07cf6", "#f06aa8"), "forest" to listOf("#a3d65c", "#2f8f5b", "#f5b83d"),
            "berry" to listOf("#b07cf6", "#f2555a", "#ff8a3d"), "ink" to listOf("#3a4a6b", "#7a5c99", "#1f2a44"),
        )

        /** A background in a few words. */
        fun describe(b: JSONObject): String {
            val color = b.optString("color").ifEmpty { "the accent" }
            val flow = if (b.optBoolean("flow")) ", moving" else ""
            return when (b.optString("kind", "plain")) {
                "tint" -> "a tint of $color"
                "gradient" -> "${b.optString("style", "glow")} gradient, $color$flow"
                "mesh" -> {
                    val cs = colorsOf(b)
                    val named = PALETTES.firstOrNull { it.second == cs }?.first
                    "soft gradient, ${named ?: "${cs.size} colours"}$flow"
                }
                "texture" -> "${b.optString("style", "grain")}, $color"
                "photo" -> "a photo"
                else -> "plain"
            }
        }

        /** A soft gradient's colours, as the look has them. */
        fun colorsOf(b: JSONObject): List<String> = b.optJSONArray("colors")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
    }
}

/** Settings › Look › Colours: dark or light, and the accent (a named one, or your own). */
class ColorsScreen(shell: Shell) : ListPage(shell, "Colours") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        val mode = Look.json.optString("mode", "dark")
        section("mode")
        choice("dark", mode == "dark") { change(JSONObject().put("mode", "dark")) }
        choice("light", mode == "light") { change(JSONObject().put("mode", "light")) }
        section("accent")
        val custom = Theme.accentName.startsWith("#")
        item(if (custom) "check-circle" else "eyedropper", "your own colour", if (custom) Theme.accent else null,
            sub = if (custom) Theme.accentName else "pick any colour", trailing = "›") {
            shell.open(Target.of("settings", "page" to "look-color-accent"))
        }
        for (name in Theme.COLORS.keys) {
            choice(name, name == Theme.accentName, tint = Theme.colorOf(name)) { change(JSONObject().put("accent", name)) }
        }
    }
}

/** Settings › Look › Font: each in itself. */
class FontScreen(shell: Shell) : ListPage(shell, "Font") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        for (id in LookScreen.FONTS) {
            choice(LookScreen.FONT_LABEL.getValue(id), id == Theme.fontName, font = Theme.fontOf(id, 700)) { change(JSONObject().put("font", id)) }
        }
    }
}

/** Settings › Look › Glance screen: its clock, where the time and the agent sit, and what it shows. */
class GlanceLookScreen(shell: Shell) : ListPage(shell, "Glance screen") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        page("clock", "clock face", LookScreen.CLOCK_LABEL[Look.clock] ?: Look.clock, "look-clock", shell)
        section("time")
        choice("on the left", Look.align == "left") { glanceChange("align", "left") }
        choice("in the middle", Look.align == "center") { glanceChange("align", "center") }
        section("agent")
        for ((id, label) in LookScreen.PLACE_LABEL) choice(label, id == Look.agentPlace) { glanceChange("agent", id) }
        section("show")
        for ((part, label, icon) in PARTS) {
            val shown = !Look.hides(part)
            item(icon, label, if (shown) accent else null, trailing = if (shown) "on" else "off") {
                val hide = JSONArray()
                for ((p, _, _) in PARTS) if (if (p == part) shown else Look.hides(p)) hide.put(p)
                glanceChange("hide", hide)
            }
        }
    }

    private companion object {
        val PARTS = listOf(
            Triple("date", "the date", "calendar"), Triple("status", "battery and wi-fi", "battery-high"),
            Triple("alarm", "alarms and timers", "alarm"), Triple("notifications", "notifications", "bell"),
            Triple("playing", "what's playing", "music-notes"), Triple("widgets", "widgets", "squares-four"),
            Triple("greeting", "a greeting on wake", "chat-circle-dots"),
        )
    }
}

/** Settings › Look › Glance screen › Clock face: each in its own face, near enough. */
class ClockFaceScreen(shell: Shell) : ListPage(shell, "Clock face") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        for ((id, label) in LookScreen.CLOCK_LABEL) choice(label, id == Look.clock, font = clockFont(id)) { glanceChange("clock", id) }
    }

    private fun clockFont(id: String) = when (id) {
        "light" -> Clocks.flex("'wght' 250")
        "condensed" -> Clocks.flex("'wdth' 25, 'wght' 700")
        "serif" -> Theme.fontOf("serif", 400)
        "mono" -> Theme.fontOf("mono", 400)
        else -> null
    }
}

/** Settings › Look › a screen's background: its style, the colours it's made of, and its finish. */
class BackgroundScreen(shell: Shell, private val screen: String) : ListPage(shell, if (screen == "home") "Home background" else "Glance background") {
    init {
        render()
    }

    private fun set(b: JSONObject) = change(JSONObject().put(screen, JSONObject().put("background", b)))

    private fun render(): Unit = list.rebuild {
        val b = Look.background(screen)
        val kind = b.optString("kind", "plain")
        val style = b.optString("style")
        if (screen == "home") {
            item("copy", "same as the glance screen", sub = LookScreen.describe(Look.background("glance"))) {
                set(JSONObject(Look.background("glance").toString()))
            }
        }
        section("style")
        choice("plain", kind == "plain") { set(JSONObject().put("kind", "plain")) }
        choice("soft gradient", kind == "mesh", sub = "your colours, with fine grain") { set(JSONObject().put("kind", "mesh")) }
        choice("a tint", kind == "tint") { set(JSONObject().put("kind", "tint")) }
        for (g in listOf("glow", "dusk", "diagonal", "horizon")) choice("$g gradient", kind == "gradient" && style == g) { set(JSONObject().put("kind", "gradient").put("style", g)) }
        for (t in listOf("grain", "noise")) choice("$t texture", kind == "texture" && style == t) { set(JSONObject().put("kind", "texture").put("style", t)) }
        choice("a photo", kind == "photo") { shell.open(Target.of("settings", "page" to "look-photo-$screen")) }
        if (kind == "photo") item("image", "pick another photo", trailing = "›") { shell.open(Target.of("settings", "page" to "look-photo-$screen")) }

        if (kind == "mesh") {
            section("colours")
            val cs = LookScreen.colorsOf(b)
            for ((i, c) in cs.withIndex()) {
                item(null, "colour ${i + 1}", sub = c, trailingView = swatches(context, listOf(Theme.colorOf(c)), 26f)) {
                    shell.open(Target.of("settings", "page" to "look-color-$screen-$i"))
                }
            }
            if (cs.size < 3) item("plus", "add a colour") {
                val next = Theme.hue(Theme.colorOf(cs.lastOrNull() ?: Theme.accentName), 40f)
                set(JSONObject().put("colors", JSONArray(cs + Theme.hex(next))))
            }
            if (cs.size > 1) item("minus", "take the last one off") { set(JSONObject().put("colors", JSONArray(cs.dropLast(1)))) }
            page("swatches", "palettes", "ready-made colours", "look-palette-$screen", shell)
        }
        if (kind in listOf("tint", "gradient", "texture")) {
            section("colour")
            val color = b.optString("color")
            choice("the accent", color.isEmpty(), tint = Theme.accent) { set(JSONObject().put("color", JSONObject.NULL)) }
            val custom = color.startsWith("#")
            item(if (custom) "check-circle" else "eyedropper", "your own colour", if (custom) Theme.colorOf(color) else null,
                sub = if (custom) color else "pick any colour", trailing = "›") {
                shell.open(Target.of("settings", "page" to "look-color-$screen-c"))
            }
            for (name in Theme.COLORS.keys) choice(name, name == color, tint = Theme.colorOf(name)) { set(JSONObject().put("color", name)) }
        }
        if (kind in listOf("mesh", "gradient", "tint", "texture")) {
            section("grain")
            val grain = b.optDouble("grain", Backdrop.GRAIN_DEFAULT[kind] ?: 0.0)
            for ((g, label) in GRAINS) choice(label, Math.abs(grain - g) < 0.01) { set(JSONObject().put("grain", g)) }
        }
        if (kind in listOf("mesh", "gradient")) {
            section("motion")
            val flow = b.optBoolean("flow")
            item("waves", "moves with the wheel", if (flow) accent else null, sub = "it drifts as you turn, and settles", trailing = if (flow) "on" else "off") {
                set(JSONObject().put("flow", !flow))
            }
        }
        if (kind in listOf("photo", "mesh")) {
            section("dim it")
            val dim = b.optDouble("dim", if (kind == "photo") 0.45 else 0.4)
            for ((d, label) in DIMS) choice(label, Math.abs(dim - d) < 0.01) { set(JSONObject().put("dim", d)) }
        }
    }

    private companion object {
        val DIMS = listOf(0.2 to "a little", 0.4 to "some", 0.6 to "a lot", 0.8 to "most")
        val GRAINS = listOf(0.0 to "none", 0.3 to "fine", 0.5 to "some", 0.8 to "lots")
    }
}

/** Settings › Look › a soft gradient's palettes: ready-made colours, each shown as its swatches. */
class PaletteScreen(shell: Shell, private val screen: String) : ListPage(shell, "Palettes") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        val now = LookScreen.colorsOf(Look.background(screen))
        for ((name, cs) in LookScreen.PALETTES) {
            val on = cs == now
            item(if (on) "check-circle" else "circle", name, if (on) accent else null, trailingView = swatches(context, cs.map { Theme.colorOf(it) })) {
                // Back first: the change builds the shell again on the screens open then.
                shell.back()
                change(JSONObject().put(screen, JSONObject().put("background", JSONObject().put("kind", "mesh").put("colors", JSONArray(cs)))))
            }
        }
    }
}

/**
 * Settings › Look › any colour: hue, vibrance and brightness on the wheel. A click on a row starts turning it, another
 * stops; a preview shows it (for a soft gradient, the gradient with it). Done keeps it.
 *
 * `field`: `accent`, `<screen>-c` (a background's colour) or `<screen>-<n>` (a soft gradient's nth colour).
 */
class ColorPickScreen(
    shell: Shell,
    private val field: String,
    /** Any colour, handed back as `#rrggbb` rather than kept in the look (an agent's own, T-060), from `from`. */
    private val from: Int? = null,
    private val onPick: ((String) -> Unit)? = null,
) : ListPage(shell, "Your colour") {
    private val screen = field.substringBefore('-').takeIf { field != "accent" && onPick == null }
    private val stop = field.substringAfter('-', "").toIntOrNull()
    private val start: Int = when {
        from != null -> from
        field == "accent" -> Theme.accent
        stop != null -> Theme.colorOf(LookScreen.colorsOf(Look.background(screen!!)).getOrNull(stop))
        else -> Theme.colorOf(Look.background(screen!!).optString("color").ifEmpty { Theme.accentName })
    }
    private val hsl = Theme.toHsl(start)
    /** The row being turned: 0 hue, 1 vibrance, 2 brightness; -1 none. */
    private var turning = -1
    private var lastTick = 0L
    private val preview = FrameLayout(context).apply { clipToOutline = true; background = roundRect(Theme.PANEL, context.dp(18f)) }
    private val hexText = context.text(15f, 700, Theme.MUTED).apply { gravity = Gravity.CENTER }
    private val bars = Array(3) { Bar(context, it) }
    private val values = Array(3) { context.text(14f, 700, Theme.MUTED) }
    private val labels = Array(3) { context.text(19f, 700).apply { text = LABELS[it] } }

    init {
        list.place(preview)
        (preview.layoutParams as? LinearLayout.LayoutParams)?.height = context.dpi(PREVIEW_DP)
        list.place(hexText)
        for (i in 0 until 3) {
            val row = LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(context.dpi(12f), context.dpi(10f), context.dpi(12f), context.dpi(12f))
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    addView(labels[i], LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                    addView(values[i])
                })
                addView(bars[i], LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(16f)).apply { topMargin = context.dpi(8f) })
            }
            list.place(row)
            list.register(row, onFocus = { on ->
                labels[i].setTextColor(if (on) Theme.ON_ACCENT else Theme.FG)
                values[i].setTextColor(if (on) Theme.ON_ACCENT else Theme.MUTED)
            }) {
                turning = if (turning == i) -1 else i
                refresh()
            }
        }
        list.button("done") { save() }
        refresh()
        list.setFocus(0, animate = false)
    }

    private fun color() = Theme.hsl(hsl[0], hsl[1], hsl[2])

    private fun refresh() {
        val c = color()
        hexText.text = Theme.hex(c)
        values[0].text = if (turning == 0) "turning" else "${hsl[0].toInt()}°"
        values[1].text = if (turning == 1) "turning" else "${(hsl[1] * 100).toInt()}%"
        values[2].text = if (turning == 2) "turning" else "${(hsl[2] * 100).toInt()}%"
        bars.forEach { it.set(hsl) }
        // The preview: the colour itself, or the soft gradient with it in its place.
        preview.removeAllViews()
        val v: View = if (screen != null && stop != null) {
            val b = JSONObject(Look.background(screen).toString())
            val cs = LookScreen.colorsOf(b).toMutableList()
            if (stop < cs.size) cs[stop] = Theme.hex(c)
            Backdrop(context, b.put("kind", "mesh").put("colors", JSONArray(cs)).put("flow", false))
        } else {
            View(context).apply { setBackgroundColor(c) }
        }
        preview.addView(v, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    /** While a row is being turned, the wheel changes it (faster when it's spun fast). */
    override fun extra(ev: Ev): Boolean {
        if (turning < 0 || ev.type != "wheel") return false
        val now = android.os.SystemClock.uptimeMillis()
        val fast = now - lastTick < FAST_MS
        lastTick = now
        val d = if (ev.int("delta") > 0) 1 else -1
        when (turning) {
            0 -> hsl[0] = ((hsl[0] + d * (if (fast) 12f else 3f)) % 360f + 360f) % 360f
            1 -> hsl[1] = (hsl[1] + d * (if (fast) 0.06f else 0.02f)).coerceIn(0f, 1f)
            else -> hsl[2] = (hsl[2] + d * (if (fast) 0.06f else 0.02f)).coerceIn(0.08f, 0.92f)
        }
        refresh()
        return true
    }

    private fun save() {
        val hex = Theme.hex(color())
        onPick?.let { pick ->
            shell.back()
            pick(hex)
            return
        }
        val c = when {
            field == "accent" -> JSONObject().put("accent", hex)
            stop != null -> {
                val cs = LookScreen.colorsOf(Look.background(screen!!)).toMutableList()
                if (stop < cs.size) cs[stop] = hex else cs += hex
                JSONObject().put(screen, JSONObject().put("background", JSONObject().put("colors", JSONArray(cs))))
            }
            else -> JSONObject().put(screen, JSONObject().put("background", JSONObject().put("color", hex)))
        }
        // Back first: the change builds the shell again on the screens open then.
        shell.back()
        change(c)
    }

    /** A channel's bar: its range drawn in colour (the hues, grey to vivid, dark to light) and where it's at. */
    private class Bar(context: Context, private val ch: Int) : View(context) {
        private val p = Paint(Paint.ANTI_ALIAS_FLAG)
        private val mark = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = context.dp(3f); color = Theme.FG }
        private var hsl = floatArrayOf(0f, 0f, 0f)

        fun set(v: FloatArray) {
            hsl = v.copyOf()
            invalidate()
        }

        override fun onDraw(c: Canvas) {
            val w = width.toFloat()
            val h = height.toFloat()
            val stops = IntArray(13) { i ->
                val t = i / 12f
                when (ch) {
                    0 -> Theme.hsl(t * 360f, maxOf(hsl[1], 0.6f), 0.55f)
                    1 -> Theme.hsl(hsl[0], t, hsl[2])
                    else -> Theme.hsl(hsl[0], hsl[1], 0.08f + t * 0.84f)
                }
            }
            p.shader = LinearGradient(0f, 0f, w, 0f, stops, null, Shader.TileMode.CLAMP)
            c.drawRoundRect(0f, 0f, w, h, h / 2, h / 2, p)
            val at = when (ch) {
                0 -> hsl[0] / 360f
                1 -> hsl[1]
                else -> (hsl[2] - 0.08f) / 0.84f
            }.coerceIn(0f, 1f)
            val x = h / 2 + at * (w - h)
            c.drawCircle(x, h / 2, h / 2 - context.dp(1.5f), mark)
        }
    }

    private companion object {
        val LABELS = listOf("hue", "vibrance", "brightness")
        const val PREVIEW_DP = 110f
        const val FAST_MS = 70L
    }
}

/** Settings › Look › a photo for a screen's background: the R1's photos, newest first, three to a row. */
class BackgroundPhotoScreen(shell: Shell, private val screen: String) : ListPage(shell, "Pick a photo") {
    private var photos: List<JSONObject> = emptyList()
    private var loaded = false

    init {
        list.gap(4f)
        R1d.get("/api/media?kind=photo&limit=120") { r ->
            val a = r.optJSONArray("media") ?: JSONArray()
            photos = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
            loaded = true
            render()
        }
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        if (photos.isEmpty()) {
            note("No photos yet: take one with the camera (launcher › Tools).", center = true)
            return@rebuild
        }
        val size = context.dpi(104f)
        val dim = Look.background(screen).takeIf { it.optString("kind") == "photo" }?.optDouble("dim", 0.45) ?: 0.45
        for (row in photos.chunked(3)) {
            val line = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
            for (p in row) {
                val id = p.optString("id")
                val v = ImageView(context).apply {
                    scaleType = ImageView.ScaleType.CENTER_CROP
                    val pad = context.dpi(4f)
                    setPadding(pad, pad, pad, pad)
                    Photos.show(this, id, size)
                }
                register(v, onFocus = {}) {
                    // Back first: the change builds the shell again on the screens that are open then.
                    shell.back()
                    change(JSONObject().put(screen, JSONObject().put("background", JSONObject().put("kind", "photo").put("photo", id).put("dim", dim))))
                }
                line.addView(v, LinearLayout.LayoutParams(size, size))
            }
            place(line)
        }
    }
}
