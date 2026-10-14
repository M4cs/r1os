package dev.r1os.host.ui

import android.graphics.Path
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt
import kotlin.math.tan

/**
 * SVG path data (`d`) to an android.graphics.Path: M L H V C S Q T A Z, absolute and relative.
 * Enough for the characters' forms (ported from shell/src/ui/Character.tsx) and the Phosphor icons
 * apps and the launcher name, which are single filled paths with arcs (ADR-0017).
 */
object SvgPath {
    fun parse(d: String): Path {
        val path = Path()
        val t = Tokens(d)
        var cmd = ' '
        var x = 0f
        var y = 0f
        var startX = 0f
        var startY = 0f
        // The last control point, for S and T reflections.
        var cx = 0f
        var cy = 0f
        var prev = ' '
        while (true) {
            t.skip()
            if (t.done()) break
            if (t.isCommand()) cmd = t.command() else if (cmd == ' ') break
            val rel = cmd.isLowerCase()
            val ox = if (rel) x else 0f
            val oy = if (rel) y else 0f
            when (cmd.uppercaseChar()) {
                'M' -> {
                    x = ox + t.num(); y = oy + t.num()
                    path.moveTo(x, y)
                    startX = x; startY = y
                    cmd = if (rel) 'l' else 'L' // further pairs are implicit line-tos
                }
                'L' -> { x = ox + t.num(); y = oy + t.num(); path.lineTo(x, y) }
                'H' -> { x = ox + t.num(); path.lineTo(x, y) }
                'V' -> { y = oy + t.num(); path.lineTo(x, y) }
                'C' -> {
                    val x1 = ox + t.num(); val y1 = oy + t.num()
                    val x2 = ox + t.num(); val y2 = oy + t.num()
                    x = ox + t.num(); y = oy + t.num()
                    path.cubicTo(x1, y1, x2, y2, x, y)
                    cx = x2; cy = y2
                }
                'S' -> {
                    val (x1, y1) = if (prev.uppercaseChar() in "CS") (2 * x - cx) to (2 * y - cy) else x to y
                    val x2 = ox + t.num(); val y2 = oy + t.num()
                    x = ox + t.num(); y = oy + t.num()
                    path.cubicTo(x1, y1, x2, y2, x, y)
                    cx = x2; cy = y2
                }
                'Q' -> {
                    val x1 = ox + t.num(); val y1 = oy + t.num()
                    x = ox + t.num(); y = oy + t.num()
                    path.quadTo(x1, y1, x, y)
                    cx = x1; cy = y1
                }
                'T' -> {
                    val (x1, y1) = if (prev.uppercaseChar() in "QT") (2 * x - cx) to (2 * y - cy) else x to y
                    x = ox + t.num(); y = oy + t.num()
                    path.quadTo(x1, y1, x, y)
                    cx = x1; cy = y1
                }
                'A' -> {
                    val rx = t.num(); val ry = t.num(); val rot = t.num()
                    val large = t.flag(); val sweep = t.flag()
                    val ex = ox + t.num(); val ey = oy + t.num()
                    arc(path, x, y, ex, ey, rx, ry, rot, large, sweep)
                    x = ex; y = ey
                }
                'Z' -> { path.close(); x = startX; y = startY }
                else -> break
            }
            prev = cmd
        }
        return path
    }

    /** An SVG elliptical arc as cubic Béziers (the W3C endpoint-to-centre conversion). */
    private fun arc(path: Path, x1: Float, y1: Float, x2: Float, y2: Float, rx0: Float, ry0: Float, rotDeg: Float, large: Boolean, sweep: Boolean) {
        if (x1 == x2 && y1 == y2) return
        var rx = abs(rx0).toDouble()
        var ry = abs(ry0).toDouble()
        if (rx == 0.0 || ry == 0.0) { path.lineTo(x2, y2); return }
        val phi = Math.toRadians(rotDeg.toDouble())
        val cosP = cos(phi); val sinP = sin(phi)
        val dx = (x1 - x2) / 2.0; val dy = (y1 - y2) / 2.0
        val x1p = cosP * dx + sinP * dy
        val y1p = -sinP * dx + cosP * dy
        val lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
        if (lambda > 1) { val s = sqrt(lambda); rx *= s; ry *= s }
        val num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
        val den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
        var coef = sqrt((num / den).coerceAtLeast(0.0))
        if (large == sweep) coef = -coef
        val cxp = coef * rx * y1p / ry
        val cyp = -coef * ry * x1p / rx
        val ccx = cosP * cxp - sinP * cyp + (x1 + x2) / 2.0
        val ccy = sinP * cxp + cosP * cyp + (y1 + y2) / 2.0
        fun angle(ux: Double, uy: Double, vx: Double, vy: Double): Double {
            val a = atan2(ux * vy - uy * vx, ux * vx + uy * vy)
            return a
        }
        val theta1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry)
        var delta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
        if (!sweep && delta > 0) delta -= 2 * PI else if (sweep && delta < 0) delta += 2 * PI
        val segments = ceil(abs(delta) / (PI / 2)).toInt().coerceAtLeast(1)
        val step = delta / segments
        val k = 4.0 / 3.0 * tan(step / 4)
        var t1 = theta1
        repeat(segments) {
            val t2 = t1 + step
            val c1 = cos(t1); val s1 = sin(t1); val c2 = cos(t2); val s2 = sin(t2)
            fun px(ex: Double, ey: Double) = (cosP * rx * ex - sinP * ry * ey + ccx).toFloat()
            fun py(ex: Double, ey: Double) = (sinP * rx * ex + cosP * ry * ey + ccy).toFloat()
            path.cubicTo(
                px(c1 - k * s1, s1 + k * c1), py(c1 - k * s1, s1 + k * c1),
                px(c2 + k * s2, s2 - k * c2), py(c2 + k * s2, s2 - k * c2),
                px(c2, s2), py(c2, s2),
            )
            t1 = t2
        }
    }

    private class Tokens(private val s: String) {
        private var i = 0
        fun done() = i >= s.length
        fun skip() { while (i < s.length && (s[i].isWhitespace() || s[i] == ',')) i++ }
        fun isCommand() = i < s.length && s[i].isLetter() && s[i] != 'e' && s[i] != 'E'
        fun command(): Char = s[i++]
        fun flag(): Boolean { skip(); val c = s[i++]; return c == '1' }
        fun num(): Float {
            skip()
            val start = i
            if (i < s.length && (s[i] == '-' || s[i] == '+')) i++
            var dot = false
            while (i < s.length) {
                val c = s[i]
                when {
                    c.isDigit() -> i++
                    c == '.' && !dot -> { dot = true; i++ }
                    (c == 'e' || c == 'E') && i + 1 < s.length -> {
                        i++
                        if (s[i] == '-' || s[i] == '+') i++
                    }
                    else -> break
                }
            }
            return s.substring(start, i).toFloatOrNull() ?: 0f
        }
    }
}
