package dev.r1os.host.ui

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.DashPathEffect
import android.graphics.LinearGradient
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RadialGradient
import android.graphics.Shader
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * What agents look like and wear, drawn from one catalog (T-062): `shell/src/ui/wardrobe/wardrobe.json`, packed into
 * the APK as a Java resource. It holds each form's body, its head and where things sit on it (pins), every kind of
 * eyes, and every thing to wear, as layers of vector shapes. The shell's `ui/wardrobe.tsx` draws the same file as SVG,
 * step for step as this does on a Canvas, and r1d checks agents' eyes and wear against it. The format is in
 * docs/specs/experience.md §3.1.
 *
 * All geometry is in the forms' 200-unit box. Drawing happens when a character's layers draw, once per change of look,
 * never per frame.
 */
object Wardrobe {
    val catalog: Catalog by lazy {
        val text = Wardrobe::class.java.getResourceAsStream("/wardrobe.json")!!.bufferedReader().use { it.readText() }
        Catalog(JSONObject(text))
    }

    /** x, y, a turn in degrees and a scale: a frame in the form's box. */
    class Pin(val x: Float, val y: Float, val r: Float, val s: Float) {
        val m: Aff get() = Aff.move(x, y) * Aff.turn(r) * Aff.size(s, s)
    }

    class Form(val id: String, o: JSONObject) {
        val label: String = o.getString("label")
        val bodyD: String = o.getString("body")
        val body: Path = SvgPath.parse(bodyD)
        val head: Path = o.optString("head").ifEmpty { null }?.let(SvgPath::parse) ?: body
        val ears: Path? = o.optString("ears").ifEmpty { null }?.let(SvgPath::parse)
        /** One pin, or a pair (its sides, its cheeks: the second drawn mirrored). */
        val pins: Map<String, List<Pin>> = o.getJSONObject("pins").let { p ->
            p.keys().asSequence().associateWith { k ->
                val a = p.getJSONArray(k)
                if (a.get(0) is JSONArray) (0 until a.length()).map { pin(a.getJSONArray(it)) } else listOf(pin(a))
            }
        }
        val pairs: Set<String> = o.getJSONObject("pins").let { p -> p.keys().asSequence().filter { p.getJSONArray(it).get(0) is JSONArray }.toSet() }

        fun pin(name: String): Aff = pins[name]?.takeIf { name !in pairs }?.first()?.m ?: Aff.ID
        /** A pin's point in the box. */
        fun point(name: String): Pair<Float, Float> = pin(name).let { it.e to it.f }
        fun path(shape: String): Path = when (shape) {
            "head" -> head
            "ears" -> ears ?: Path()
            else -> body
        }

        private fun pin(a: JSONArray) = Pin(a.f(0), a.f(1), a.f(2), a.f(3))
    }

    class Slot(o: JSONObject) {
        val id: String = o.getString("id")
        val label: String = o.getString("label")
        /** Where its items sit unless a layer says otherwise. */
        val pin: String = o.getString("pin")
    }

    class Eyes(o: JSONObject) {
        val id: String = o.getString("id")
        val label: String = o.getString("label")
        /** False: already half shut, so it doesn't blink. */
        val blink: Boolean = o.optBoolean("blink", true)
        /** A reaction's face, not offered (ADR-0043). */
        val hidden: Boolean = o.optBoolean("hidden", false)
        /** Where its eyes are in the face's frame (x, y, size); what's worn on the face sits on them. */
        val spots: List<FloatArray> = o.optJSONArray("spots")?.let { s -> (0 until s.length()).map { s.getJSONArray(it).floats() } }
            ?: listOf(floatArrayOf(-22f, 0f, 1f), floatArrayOf(22f, 0f, 1f))
        val layers: List<Layer> = layers(o.optJSONArray("layers"))
    }

    class Item(o: JSONObject) {
        val id: String = o.getString("id")
        val slot: String = o.getString("slot")
        val label: String = o.getString("label")
        val layers: List<Layer> = layers(o.optJSONArray("layers"))
        /** Drawn behind the body (wings, a cape, the back of a crown). */
        val behind: List<Layer> = layers(o.optJSONArray("behind"))
        /** The form's ears drawn again over it. */
        val ears: Boolean = o.optBoolean("ears", false)
    }

    class Catalog(o: JSONObject) {
        val colors: Map<String, Int> = o.getJSONObject("colors").let { c -> c.keys().asSequence().associateWith { Color.parseColor(c.getString(it)) } }
        val forms: Map<String, Form> = o.getJSONObject("forms").let { f -> f.keys().asSequence().associateWith { Form(it, f.getJSONObject(it)) } }
        val slots: List<Slot> = o.getJSONArray("slots").let { s -> (0 until s.length()).map { Slot(s.getJSONObject(it)) } }
        val eyes: List<Eyes> = o.getJSONArray("eyes").let { e -> (0 until e.length()).map { Eyes(e.getJSONObject(it)) } }
        val items: List<Item> = o.getJSONArray("items").let { e -> (0 until e.length()).map { Item(e.getJSONObject(it)) } }
        private val eyesById = eyes.associateBy { it.id }
        private val itemsById = items.associateBy { it.id }

        val formIds: List<String> get() = forms.keys.toList()
        /** The eyes you can pick, in order. */
        val eyeIds: List<String> get() = eyes.filter { !it.hidden }.map { it.id }
        fun form(id: String): Form = forms[id] ?: forms.getValue("orb")
        fun eyes(id: String?): Eyes = eyesById[id] ?: eyesById.getValue("classic")
        fun item(id: String): Item? = itemsById[id]
        fun itemsIn(slot: String): List<Item> = items.filter { it.slot == slot }
        fun slot(id: String): Slot? = slots.firstOrNull { it.id == id }
        /** The worn items, one per slot (the first of each), in the slots' order. */
        fun worn(wear: List<String>): List<Item> = slots.mapNotNull { s -> wear.firstNotNullOfOrNull { w -> item(w)?.takeIf { it.slot == s.id } } }
    }

    /** One layer: a shape and its paint, or a group of layers, placed in its parent's frame or at a pin. */
    class Layer(o: JSONObject) {
        val d: String? = o.optString("d").ifEmpty { null }
        val circle: FloatArray? = o.optJSONArray("circle")?.floats()
        val ellipse: FloatArray? = o.optJSONArray("ellipse")?.floats()
        val rect: FloatArray? = o.optJSONArray("rect")?.floats()
        /** The form's own outline, wherever the layer is (in the form's box). */
        val shape: String? = o.optString("shape").ifEmpty { null }
        val g: List<Layer>? = o.optJSONArray("g")?.let(::layers)
        val at: String? = o.optString("at").ifEmpty { null }
        val t: FloatArray? = o.optJSONArray("t")?.floats()
        val r: Float = o.optDouble("r", 0.0).toFloat()
        val s: FloatArray? = when (val v = o.opt("s")) {
            is Number -> floatArrayOf(v.toFloat(), v.toFloat())
            is JSONArray -> v.floats()
            else -> null
        }
        val mirror: Boolean = o.optBoolean("mirror", false)
        val each: Boolean = o.optString("each") == "eye"
        val side: String? = o.optString("side").ifEmpty { null }
        val pair: Boolean = o.optBoolean("pair", false)
        val outer: Boolean = o.optBoolean("outer", false)
        val fixed: Boolean = o.optBoolean("fixed", false)
        val forms: List<String>? = o.optJSONArray("forms")?.strings()
        val except: List<String>? = o.optJSONArray("except")?.strings()
        /** Form outlines (`head`, `!head`, `head+3`) or shapes in the layer's frame, all of which it's drawn inside. */
        val clip: List<Any> = when (val c = o.opt("clip")) {
            null -> emptyList()
            is JSONArray -> (0 until c.length()).map { i -> c.get(i).let { if (it is JSONObject) Layer(it) else it.toString() } }
            is JSONObject -> listOf(Layer(c))
            else -> listOf(c.toString())
        }
        val fill: Any? = o.opt("fill")
        val stroke: Any? = o.opt("stroke")
        val width: Float = o.optDouble("width", 1.0).toFloat()
        val dash: FloatArray? = o.optJSONArray("dash")?.floats()
        val cap: String = o.optString("cap", "round")
        val join: String = o.optString("join", "round")
        val opacity: Float = o.optDouble("opacity", 1.0).toFloat()
        val grow: Float = o.optDouble("grow", 0.0).toFloat()

        /** The shape's path in the layer's frame (a form's outline is drawn per look instead). */
        val path: Path? by lazy {
            when {
                d != null -> SvgPath.parse(d)
                circle != null -> Path().apply { addCircle(circle[0], circle[1], circle[2], Path.Direction.CW) }
                ellipse != null -> Path().apply { addOval(ellipse[0] - ellipse[2], ellipse[1] - ellipse[3], ellipse[0] + ellipse[2], ellipse[1] + ellipse[3], Path.Direction.CW) }
                rect != null -> Path().apply {
                    val rr = rect.getOrElse(4) { 0f }
                    addRoundRect(rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3], rr, rr, Path.Direction.CW)
                }
                else -> null
            }
        }
    }

    // ---- frames ----------------------------------------------------------------------------------

    /** An affine matrix as SVG's [a b c d e f]: x' = a x + c y + e, y' = b x + d y + f. */
    class Aff(val a: Float, val b: Float, val c: Float, val d: Float, val e: Float, val f: Float) {
        operator fun times(n: Aff) = Aff(
            a * n.a + c * n.b, b * n.a + d * n.b, a * n.c + c * n.d, b * n.c + d * n.d, a * n.e + c * n.f + e, b * n.e + d * n.f + f,
        )
        fun inv(): Aff {
            val det = a * d - b * c
            return Aff(d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det)
        }
        val scale: Float get() = sqrt(abs(a * d - b * c))
        val matrix: Matrix get() = Matrix().apply { setValues(floatArrayOf(a, c, e, b, d, f, 0f, 0f, 1f)) }

        companion object {
            val ID = Aff(1f, 0f, 0f, 1f, 0f, 0f)
            fun move(x: Float, y: Float) = Aff(1f, 0f, 0f, 1f, x, y)
            fun size(sx: Float, sy: Float) = Aff(sx, 0f, 0f, sy, 0f, 0f)
            fun turn(deg: Float): Aff {
                val r = Math.toRadians(deg.toDouble())
                return Aff(cos(r).toFloat(), sin(r).toFloat(), -sin(r).toFloat(), cos(r).toFloat(), 0f, 0f)
            }
        }
    }

    // ---- colours ---------------------------------------------------------------------------------

    /** The colours a look draws with: the agent's own (`c`), its opposite (`tone`) and the eyes' ink. */
    class Look(val formId: String, val form: Form, val c: Int, val spots: List<FloatArray>) {
        val tone: Int = mix(Theme.hue(c, 160f), Color.BLACK, 0.12f)
        val ink: Int = if (formId == "bot") c else catalog.colors.getValue("ink")
    }

    /** `c` moved toward `to` by `t` (0–1), rounded as the shell rounds. */
    fun mix(c: Int, to: Int, t: Float): Int = Color.rgb(
        (Color.red(c) + (Color.red(to) - Color.red(c)) * t).roundToInt(),
        (Color.green(c) + (Color.green(to) - Color.green(c)) * t).roundToInt(),
        (Color.blue(c) + (Color.blue(to) - Color.blue(c)) * t).roundToInt(),
    )

    private val SPEC = Regex("^(#[0-9a-fA-F]{6}|[a-z]+)(.*)$")
    private val OP = Regex("([+\\-~])(-?[\\d.]+)")

    /**
     * A catalog colour: `#rrggbb`, `c`, `tone`, `ink` or a named colour, then any of `+N` (N% toward white), `-N`
     * (toward black) and `~N` (the hue turned N degrees), in order: `gold-30`, `c~120+10`.
     */
    fun color(spec: String, look: Look): Int {
        val m = SPEC.matchEntire(spec) ?: return Color.MAGENTA
        val base = m.groupValues[1]
        var out = when {
            base.startsWith("#") -> Color.parseColor(base)
            base == "c" -> look.c
            base == "tone" -> look.tone
            base == "ink" -> look.ink
            else -> catalog.colors[base] ?: Color.MAGENTA
        }
        for (op in OP.findAll(m.groupValues[2])) {
            val v = op.groupValues[2].toFloat()
            out = when (op.groupValues[1]) {
                "+" -> mix(out, Color.WHITE, v / 100f)
                "-" -> mix(out, Color.BLACK, v / 100f)
                else -> Theme.hue(out, v)
            }
        }
        return out
    }

    // ---- drawing ---------------------------------------------------------------------------------

    /** Layers at a pin, on a canvas in the form's box. */
    fun drawAt(cv: Canvas, layers: List<Layer>, look: Look, pin: String) {
        if (layers.isEmpty()) return
        val m = look.form.pin(pin)
        cv.save()
        cv.concat(m.matrix)
        for (l in layers) drawLayer(cv, l, look, m, m)
        cv.restore()
    }

    /** The layer's own frame in its parent's: at a pin (unless `atPin` is false), then moved, turned and scaled. */
    private fun frameOf(l: Layer, parent: Aff, form: Form, atPin: Boolean = true): Aff {
        var m = parent
        if (atPin && l.at != null && l.at !in form.pairs && form.pins.containsKey(l.at)) m = form.pin(l.at)
        l.t?.let { m *= Aff.move(it[0], it[1]) }
        if (l.r != 0f) m *= Aff.turn(l.r)
        l.s?.let { m *= Aff.size(it[0], it[1]) }
        return m
    }

    /**
     * One layer, its placement composed onto `base`, on a canvas whose transform is `ctx` (the same, except where a
     * pair of pins moves the base).
     */
    private fun drawLayer(cv: Canvas, l: Layer, look: Look, base: Aff, ctx: Aff) {
        if ((l.forms != null && look.formId !in l.forms) || l.except?.contains(look.formId) == true) return
        if (l.each) {
            if (l.pair && look.spots.size < 2) return
            val placed = frameOf(l, base, look.form)
            for (spot in look.spots) {
                val (x, y, s) = Triple(spot[0], spot[1], spot[2])
                val right = x > 0
                if ((l.side == "R" && x < 0) || (l.side == "L" && right)) continue
                val at = placed * Aff.move(x, y) * Aff.size(if (right) -s else s, s)
                val flip = at * Aff.size(-1f, 1f)
                val kids = l.g.orEmpty()
                within(cv, ctx, at) { for (k in kids) drawLayer(cv, k, look, if (right && k.fixed) flip else at, at) }
                // One eye in the middle: what's on its outer side goes on both sides of it.
                if (x == 0f) {
                    val outer = kids.filter { it.outer }
                    if (outer.isNotEmpty()) within(cv, ctx, flip) { for (k in outer) drawLayer(cv, k, look, flip, flip) }
                }
            }
            return
        }
        // A pair of pins (its sides, its cheeks): once at each, the right one mirrored.
        if (l.at != null && l.at in look.form.pairs) {
            look.form.pins.getValue(l.at).forEachIndexed { i, p ->
                drawPlaced(cv, l, look, frameOf(l, p.m * Aff.size(if (i > 0) -1f else 1f, 1f), look.form, atPin = false), ctx)
            }
            return
        }
        drawPlaced(cv, l, look, frameOf(l, base, look.form), ctx)
    }

    private fun drawPlaced(cv: Canvas, l: Layer, look: Look, m: Aff, ctx: Aff) {
        val copies = if (l.mirror) listOf(m, m * Aff.size(-1f, 1f)) else listOf(m)
        for (f in copies) within(cv, ctx, f) {
            clip(cv, l, look, f)
            if (l.g != null) {
                val layered = l.opacity < 1f
                if (layered) cv.saveLayerAlpha(null, (l.opacity * 255).roundToInt())
                for (c in l.g) drawLayer(cv, c, look, f, f)
                if (layered) cv.restore()
            } else {
                shape(cv, l, look, f)
            }
        }
    }

    /** Runs `draw` on the canvas moved from `ctx` to `m`. */
    private inline fun within(cv: Canvas, ctx: Aff, m: Aff, draw: () -> Unit) {
        cv.save()
        cv.concat((ctx.inv() * m).matrix)
        draw()
        cv.restore()
    }

    /** A form's outline in the frame `m`. */
    private fun formPath(look: Look, shape: String, m: Aff): Path = Path(look.form.path(shape)).apply { transform(m.inv().matrix) }

    private val CLIP = Regex("^(!?)(head|body|ears)(?:\\+([\\d.]+))?$")

    /** Each of the layer's clips, in its frame `m`. */
    private fun clip(cv: Canvas, l: Layer, look: Look, m: Aff) {
        for (c in l.clip) {
            if (c is Layer) {
                // Its own placement, in the layer's frame (or at a pin).
                val local = if (c.at != null) m.inv() * frameOf(c, Aff.ID, look.form) else frameOf(c, Aff.ID, look.form)
                val p = Path(c.path ?: continue).apply { transform(local.matrix) }
                cv.clipPath(p)
                continue
            }
            val g = CLIP.matchEntire(c as String) ?: continue
            val path = formPath(look, g.groupValues[2], m)
            val grow = g.groupValues[3].toFloatOrNull()
            if (grow != null) {
                val out = Path()
                Paint().apply { style = Paint.Style.STROKE; strokeWidth = grow * 2; strokeJoin = Paint.Join.ROUND }.getFillPath(path, out)
                out.op(path, Path.Op.UNION)
                if (g.groupValues[1] == "!") cv.clipOutPath(out) else cv.clipPath(out)
            } else if (g.groupValues[1] == "!") {
                cv.clipOutPath(path)
            } else {
                cv.clipPath(path)
            }
        }
    }

    /** A shape, filled then stroked; paint stays in the layer's frame. */
    private fun shape(cv: Canvas, l: Layer, look: Look, m: Aff) {
        val path = l.shape?.let { formPath(look, it, m) } ?: l.path ?: return
        val p = Paint(Paint.ANTI_ALIAS_FLAG)
        val fill = l.fill?.takeIf { it != "none" }
        if (fill != null) {
            paint(p, fill, look, l.opacity)
            if (l.grow > 0f) {
                p.style = Paint.Style.FILL_AND_STROKE
                p.strokeWidth = l.grow * 2
                p.strokeJoin = Paint.Join.ROUND
            } else {
                p.style = Paint.Style.FILL
            }
            cv.drawPath(path, p)
        }
        val stroke = l.stroke?.takeIf { it != "none" }
        if (stroke != null && !(l.grow > 0f && fill != null)) {
            p.reset()
            p.isAntiAlias = true
            paint(p, stroke, look, l.opacity)
            p.style = Paint.Style.STROKE
            p.strokeWidth = l.width
            p.strokeCap = when (l.cap) { "butt" -> Paint.Cap.BUTT; "square" -> Paint.Cap.SQUARE; else -> Paint.Cap.ROUND }
            p.strokeJoin = when (l.join) { "miter" -> Paint.Join.MITER; "bevel" -> Paint.Join.BEVEL; else -> Paint.Join.ROUND }
            l.dash?.let { p.pathEffect = DashPathEffect(it, 0f) }
            cv.drawPath(path, p)
        }
    }

    /** A colour or a gradient (`{lin: [x1, y1, x2, y2]}` or `{rad: [cx, cy, r]}`, with stops) on the paint. */
    private fun paint(p: Paint, spec: Any, look: Look, opacity: Float) {
        if (spec is JSONObject) {
            val stops = spec.getJSONArray("stops")
            val n = stops.length()
            val colors = IntArray(n)
            val at = FloatArray(n)
            for (i in 0 until n) {
                val s = stops.get(i)
                if (s is JSONArray) {
                    at[i] = s.getDouble(0).toFloat(); colors[i] = color(s.getString(1), look)
                } else {
                    at[i] = if (n == 1) 0f else i / (n - 1f); colors[i] = color(s.toString(), look)
                }
            }
            p.shader = spec.optJSONArray("lin")?.floats()?.let { LinearGradient(it[0], it[1], it[2], it[3], colors, at, Shader.TileMode.CLAMP) }
                ?: spec.getJSONArray("rad").floats().let { RadialGradient(it[0], it[1], it[2], colors, at, Shader.TileMode.CLAMP) }
            p.alpha = (opacity * 255).roundToInt()
        } else {
            p.color = color(spec.toString(), look)
            p.alpha = (opacity * 255).roundToInt()
        }
    }

    // ---- reading ---------------------------------------------------------------------------------

    private fun layers(a: JSONArray?): List<Layer> = a?.let { (0 until it.length()).map { i -> Layer(it.getJSONObject(i)) } }.orEmpty()
    private fun JSONArray.f(i: Int) = getDouble(i).toFloat()
    private fun JSONArray.floats() = FloatArray(length()) { getDouble(it).toFloat() }
    private fun JSONArray.strings() = (0 until length()).map { getString(it) }
}
