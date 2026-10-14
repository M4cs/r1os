package dev.r1os.host.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.view.View
import java.io.ByteArrayOutputStream
import java.util.zip.CRC32
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.math.PI
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/**
 * The boot animation (T-029): your agents pop in one after another, breathe and blink while the R1 starts, and hop
 * away when it's up. Drawn here with the characters' own drawing (CharacterView, offscreen), into Android's
 * `bootanimation.zip`: `desc.txt` and PNG frames in three parts, stored, not compressed (Android reads it in place).
 * r1d keeps it in /data/r1os/boot, and the image puts it where Android's boot animation looks once /data is up.
 */
object BootAnimation {
    /** One character to draw: a look, as an agent's. */
    data class Look(val form: String, val color: String, val finish: String, val eyes: String, val wear: List<String>)

    /** A fresh R1's (the image's own): r1os's four first forms. */
    val DEFAULT = listOf(
        Look("orb", "orange", "soft", "classic", emptyList()),
        Look("blob", "teal", "soft", "dots", emptyList()),
        Look("bot", "blue", "soft", "classic", emptyList()),
        Look("cat", "pink", "soft", "happy", emptyList()),
    )

    /** Changes when the drawing does, so a cast drawn by an older one is drawn again. */
    const val VERSION = 2
    private const val W = 480
    private const val H = 640
    private const val FPS = 20
    private const val INTRO = 30
    private const val LOOP = 48
    private const val OUTRO = 12
    private val BG = Color.parseColor("#0e1013")
    private val MUTED = Color.parseColor("#7d8590")

    /** What a cast is, to know whether it's been drawn. */
    fun signature(cast: List<Look>): String = "$VERSION:" + cast.joinToString("|") { "${it.form},${it.color},${it.finish},${it.eyes},${it.wear.joinToString("+")}" }

    /** The zip for `cast` (at most four drawn). Slow (a few seconds): not on the main thread. */
    fun render(context: Context, cast: List<Look>): ByteArray {
        val looks = cast.ifEmpty { DEFAULT }.take(4)
        // A character's box leaves room for its halo and what it wears, so the box is drawn big.
        val size = when (looks.size) { 1 -> 300; 2 -> 228; 3 -> 150; else -> 200 }
        val views = looks.map { l ->
            CharacterView(context, size).apply {
                set(l.form, l.color, l.finish, "idle", eyes = l.eyes, wear = l.wear)
                setActive(false)
                measure(View.MeasureSpec.makeMeasureSpec(size, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(size, View.MeasureSpec.EXACTLY))
                layout(0, 0, size, size)
            }
        }
        val spots = spots(looks.size, size)
        val out = ByteArrayOutputStream()
        ZipOutputStream(out).use { zip ->
            stored(zip, "desc.txt", "$W $H $FPS\nc 1 0 part0\np 0 0 part1\nc 1 0 part2\n".toByteArray())
            val bmp = Bitmap.createBitmap(W, H, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bmp)
            val png = ByteArrayOutputStream()
            for ((part, frames) in listOf("part0" to INTRO, "part1" to LOOP, "part2" to OUTRO)) {
                for (f in 0 until frames) {
                    frame(canvas, views, spots, size, part, f)
                    png.reset()
                    bmp.compress(Bitmap.CompressFormat.PNG, 100, png)
                    stored(zip, "$part/%05d.png".format(f), png.toByteArray())
                }
            }
            bmp.recycle()
        }
        return out.toByteArray()
    }

    /** Where each character's top-left goes: a row, or two rows for four. */
    private fun spots(n: Int, size: Int): List<Pair<Float, Float>> {
        val gap = 14f
        val cy = H * 0.42f
        return if (n <= 3) {
            val total = n * size + (n - 1) * gap
            (0 until n).map { i -> (W - total) / 2f + i * (size + gap) to cy - size / 2f }
        } else {
            val total = 2 * size + gap
            (0 until n).map { i -> (W - total) / 2f + (i % 2) * (size + gap) to cy - total / 2f + (i / 2) * (size + gap) }
        }
    }

    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = MUTED; textAlign = Paint.Align.CENTER; textSize = 34f }
    private val dot = Paint(Paint.ANTI_ALIAS_FLAG)

    private fun frame(c: Canvas, views: List<CharacterView>, spots: List<Pair<Float, Float>>, size: Int, part: String, f: Int) {
        c.drawColor(BG)
        text.typeface = Theme.font(700)
        val bottom = spots.maxOf { it.second } + size
        for ((i, v) in views.withIndex()) {
            val (x, y) = spots[i]
            var dy = 0f
            var scale = 1f
            var alpha = 1f
            var open = 1f
            when (part) {
                // One after another, each popping up from nothing with a little overshoot.
                "part0" -> {
                    val t = ((f - i * 5) / 12f).coerceIn(0f, 1f)
                    scale = pop(t)
                    dy = (1 - t) * 40f
                    alpha = min(1f, t * 2f)
                }
                // Breathing, each a little after the one before; a blink each, at its own moment.
                "part1" -> {
                    val phase = 2 * PI * (f / LOOP.toDouble() + i * 0.18)
                    dy = (-6 * (0.5 - 0.5 * kotlin.math.cos(phase))).toFloat()
                    scale = 1f + 0.015f * (0.5f - 0.5f * kotlin.math.cos(phase).toFloat())
                    val b = (f - (10 + i * 9)).let { if (it < 0) it + LOOP else it }
                    open = when (b) { 0 -> 0.45f; 1 -> 0.1f; 2 -> 0.5f; else -> 1f }
                }
                // Up and away: a hop, smaller and fading.
                "part2" -> {
                    val t = f / (OUTRO - 1f)
                    dy = -sin(t * PI).toFloat() * 26f - t * t * 30f
                    scale = 1f - 0.25f * t
                    alpha = 1f - t
                }
            }
            v.blinkPose(open)
            c.save()
            c.translate(x, y + dy)
            c.scale(scale, scale, size / 2f, size.toFloat())
            if (alpha < 1f) c.saveLayerAlpha(0f, 0f, size.toFloat(), size.toFloat(), (alpha * 255).toInt())
            v.draw(c)
            if (alpha < 1f) c.restore()
            c.restore()
        }
        // The name, and three dots taking turns while it loads.
        val wordAlpha = when (part) { "part0" -> (f / (INTRO - 1f)); "part2" -> 1f - f / (OUTRO - 1f); else -> 1f }
        text.alpha = (wordAlpha * 255).toInt().coerceIn(0, 255)
        c.drawText("r1os", W / 2f, bottom + 70f, text)
        if (part == "part1") {
            for (d in 0 until 3) {
                val on = ((f / (LOOP / 6)) % 3) == d
                dot.color = if (on) Theme.colorOf("orange") else MUTED
                dot.alpha = if (on) 255 else 110
                c.drawCircle(W / 2f + (d - 1) * 22f, bottom + 104f, if (on) 6f else 4.5f, dot)
            }
        }
    }

    /** 0→1 with a small overshoot (easeOutBack). */
    private fun pop(t: Float): Float {
        val s = 1.7f
        val u = t - 1f
        return max(0f, 1f + (s + 1) * u * u * u + s * u * u)
    }

    private fun stored(zip: ZipOutputStream, name: String, bytes: ByteArray) {
        val e = ZipEntry(name)
        e.method = ZipEntry.STORED
        e.size = bytes.size.toLong()
        e.compressedSize = bytes.size.toLong()
        e.crc = CRC32().apply { update(bytes) }.value
        zip.putNextEntry(e)
        zip.write(bytes)
        zip.closeEntry()
    }
}
