package dev.r1os.host.ui

import android.graphics.Canvas
import android.graphics.ColorFilter
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PixelFormat
import android.graphics.drawable.Drawable
import dev.r1os.host.net.R1d

/**
 * Phosphor icons, drawn natively (ADR-0017). r1d serves the shell's icon set at
 * `/icons/<weight>/<name>.svg`: one filled path on a 256 grid. Each is fetched once and cached.
 */
object Icons {
    private val cache = HashMap<String, Path>()
    private val waiting = HashMap<String, MutableList<IconDrawable>>()

    fun drawable(name: String, color: Int, weight: String = "bold"): IconDrawable {
        val d = IconDrawable(color)
        val key = "$weight/$name"
        val hit = cache[key]
        if (hit != null) {
            d.path = hit
            return d
        }
        val list = waiting.getOrPut(key) { mutableListOf() }
        list += d
        if (list.size == 1) {
            R1d.bytes("/icons/$key.svg") { bytes ->
                val path = bytes?.let { parse(String(it)) }
                if (path != null) cache[key] = path
                waiting.remove(key)?.forEach { it.path = path }
            }
        }
        return d
    }

    /** Every `<path d="…">` of an icon, joined. */
    fun parse(svg: String): Path? {
        val out = Path()
        var found = false
        Regex("""<path[^>]*\sd="([^"]+)"""").findAll(svg).forEach {
            out.addPath(SvgPath.parse(it.groupValues[1]))
            found = true
        }
        return if (found) out else null
    }
}

/** A Phosphor icon (256-unit grid) scaled to its bounds. Blank until it has loaded. */
class IconDrawable(color: Int) : Drawable() {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { this.color = color; style = Paint.Style.FILL }
    private val scaled = Path()
    private val matrix = Matrix()

    var path: Path? = null
        set(value) {
            field = value
            rescale()
            invalidateSelf()
        }

    override fun onBoundsChange(bounds: android.graphics.Rect) = rescale()

    private fun rescale() {
        val p = path ?: return
        val b = bounds
        if (b.isEmpty) return
        matrix.setScale(b.width() / 256f, b.height() / 256f)
        matrix.postTranslate(b.left.toFloat(), b.top.toFloat())
        p.transform(matrix, scaled)
    }

    override fun draw(canvas: Canvas) {
        if (path != null) canvas.drawPath(scaled, paint)
    }

    /** Its colour now (a focused row swaps it for ink and back). */
    val color: Int get() = paint.color

    fun setColor(c: Int) {
        paint.color = c
        invalidateSelf()
    }

    override fun setAlpha(alpha: Int) { paint.alpha = alpha; invalidateSelf() }
    override fun setColorFilter(cf: ColorFilter?) { paint.colorFilter = cf; invalidateSelf() }
    @Deprecated("Deprecated in Java")
    override fun getOpacity(): Int = PixelFormat.TRANSLUCENT
}
