package dev.r1os.host.shell

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.text.TextUtils
import android.util.LruCache
import android.view.Gravity
import android.view.View
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Cards in chats (ADR-0039): what an agent shows with its answer (`show_card`), drawn natively from the card r1d
 * checked (crate::cards): headlines, a link, the weather, a picture, stats, bars, a line, steps or a table. [cardView]
 * is the card in the chat; [CardScreen] is a card's links, where the wheel moves between them and a click opens one
 * in the R1's reader. Words from the web keep their case; the card's own captions are r1os's lowercase.
 */
fun cardView(context: Context, card: JSONObject, accent: Int): View {
    val box = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(14f), context.dpi(12f), context.dpi(14f), context.dpi(14f))
        background = roundRect(Theme.CARD, context.dp(18f))
    }
    val c = Cards(context, box, accent)
    when (card.optString("type")) {
        "headlines" -> c.headlines(card)
        "link" -> c.link(card)
        "weather" -> c.weather(card)
        "image" -> c.image(card)
        "stats" -> c.stats(card)
        "bars" -> c.bars(card)
        "line" -> c.line(card)
        "steps" -> c.steps(card)
        "table" -> c.table(card)
        else -> c.words(card.optString("summary"))
    }
    return box
}

/** The links on a card, in order: what a click in the chat opens ([CardScreen], or the one link itself). */
fun cardLinks(card: JSONObject): List<JSONObject> = when (card.optString("type")) {
    "headlines" -> card.optJSONArray("items").objects().filter { it.optString("url").isNotEmpty() }
    "link" -> listOf(card)
    "image" -> if (card.optString("link").isNotEmpty()) listOf(JSONObject().put("url", card.optString("link")).put("title", card.optString("caption"))) else emptyList()
    else -> emptyList()
}

private fun JSONArray?.objects(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }

/** Headlines in the chat show this many; the card's page has them all. */
private const val CHAT_HEADLINES = 3

private class Cards(val context: Context, val box: LinearLayout, val accent: Int) {
    private fun add(v: View, top: Float = 8f, width: Int = LinearLayout.LayoutParams.MATCH_PARENT, height: Int = LinearLayout.LayoutParams.WRAP_CONTENT) {
        box.addView(v, LinearLayout.LayoutParams(width, height).apply { if (box.childCount > 0) topMargin = context.dpi(top) })
    }

    /** The card's caption: its title, or what kind of card it is. */
    private fun caption(text: String) {
        if (text.isEmpty()) return
        add(context.text(13f, 700, accent).apply { this.text = text; maxLines = 1; ellipsize = TextUtils.TruncateAt.END }, 0f)
    }

    /** Words as written (from the web, or the agent). */
    private fun said(size: Float, weight: Int, color: Int = Theme.FG, lines: Int = 3): TextView =
        context.text(size, weight, color).keepCase().apply {
            maxLines = lines
            ellipsize = TextUtils.TruncateAt.END
            setLineSpacing(0f, 1.12f)
        }

    private fun meta(vararg parts: String): String = parts.filter { it.isNotBlank() }.joinToString(" · ")

    private fun picture(url: String, heightDp: Float, corner: Float = 12f): ImageView = ImageView(context).apply {
        scaleType = ImageView.ScaleType.CENTER_CROP
        background = roundRect(Theme.FOCUS, context.dp(corner))
        clipToOutline = true
        CardImages.show(this, url, context.dpi(heightDp * 2))
    }

    fun words(text: String) = add(said(16f, 500, lines = 6).apply { this.text = text }, 0f)

    fun headlines(card: JSONObject) {
        caption(card.optString("title").ifEmpty { "headlines" })
        val items = card.optJSONArray("items").objects()
        items.firstOrNull()?.let { first ->
            first.optString("image").takeIf { it.isNotEmpty() }?.let { add(picture(it, 150f), height = context.dpi(150f)) }
            add(said(18f, 700).apply { text = first.optString("title") })
            first.optString("tagline").takeIf { it.isNotEmpty() }?.let { add(said(15f, 500, Theme.MUTED).apply { text = it }, 4f) }
            meta(first.optString("source"), first.optString("time")).takeIf { it.isNotEmpty() }?.let {
                add(said(13f, 600, Theme.MUTED, 1).apply { text = it }, 4f)
            }
        }
        for (item in items.drop(1).take(CHAT_HEADLINES - 1)) {
            val row = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
            }
            item.optString("image").takeIf { it.isNotEmpty() }?.let {
                row.addView(picture(it, 48f, 10f), LinearLayout.LayoutParams(context.dpi(48f), context.dpi(48f)).apply { rightMargin = context.dpi(10f) })
            }
            row.addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(said(15f, 600, lines = 2).apply { text = item.optString("title") })
                meta(item.optString("source"), item.optString("time")).takeIf { it.isNotEmpty() }?.let {
                    addView(said(12f, 600, Theme.MUTED, 1).apply { text = it; setPadding(0, context.dpi(2f), 0, 0) })
                }
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            add(divider(), 10f, height = context.dpi(1f))
            add(row, 10f)
        }
        val more = items.size - CHAT_HEADLINES
        val linked = items.any { it.optString("url").isNotEmpty() }
        val hint = when {
            more > 0 -> "$more more · click to see them all"
            linked -> "click to read"
            else -> ""
        }
        if (hint.isNotEmpty()) add(context.text(13f, 600, Theme.MUTED).apply { text = hint }, 10f)
    }

    fun link(card: JSONObject) {
        caption(card.optString("source").ifEmpty { "link" })
        card.optString("image").takeIf { it.isNotEmpty() }?.let { add(picture(it, 140f), height = context.dpi(140f)) }
        add(said(18f, 700).apply { text = card.optString("title") })
        card.optString("summary").takeIf { it.isNotEmpty() }?.let { add(said(15f, 500, Theme.MUTED, 4).apply { text = it }, 4f) }
        add(context.text(13f, 600, Theme.MUTED).apply { text = "click to read" }, 8f)
    }

    fun weather(card: JSONObject) {
        val unit = card.optString("unit", "C")
        fun deg(v: Double) = "${v.roundToInt()}°"
        caption(card.optString("place"))
        val now = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(ImageView(context).apply { setImageDrawable(Icons.drawable(card.optString("icon", "sun"), accent)) },
                LinearLayout.LayoutParams(context.dpi(54f), context.dpi(54f)).apply { rightMargin = context.dpi(12f) })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(context.text(40f, 700).apply { text = deg(card.optDouble("temp")) + unit; includeFontPadding = false }.keepCase())
                addView(context.text(16f, 600, Theme.MUTED).apply { text = card.optString("condition"); maxLines = 1; ellipsize = TextUtils.TruncateAt.END })
            })
        }
        add(now, 6f)
        val range = listOfNotNull(
            card.optDouble("high").takeIf { !it.isNaN() }?.let { "h ${deg(it)}" },
            card.optDouble("low").takeIf { !it.isNaN() }?.let { "l ${deg(it)}" },
            card.optDouble("feels").takeIf { !it.isNaN() }?.let { "feels ${deg(it)}" },
        ).joinToString(" · ")
        if (range.isNotEmpty()) add(context.text(14f, 600, Theme.MUTED).apply { text = range }, 6f)
        val hours = card.optJSONArray("hours").objects().take(6)
        if (hours.isNotEmpty()) {
            val row = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
            for (h in hours) {
                row.addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    gravity = Gravity.CENTER_HORIZONTAL
                    addView(context.text(12f, 600, Theme.MUTED).apply { text = h.optString("time"); maxLines = 1 })
                    addView(ImageView(context).apply { setImageDrawable(Icons.drawable(h.optString("icon", "sun"), Theme.FG)) },
                        LinearLayout.LayoutParams(context.dpi(22f), context.dpi(22f)).apply { topMargin = context.dpi(4f); bottomMargin = context.dpi(4f) })
                    addView(context.text(15f, 700).apply { text = deg(h.optDouble("temp")) })
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            }
            add(divider(), 12f, height = context.dpi(1f))
            add(row, 10f)
        }
        val days = card.optJSONArray("days").objects()
        if (days.isNotEmpty()) add(divider(), 12f, height = context.dpi(1f))
        for (d in days) {
            add(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(context.text(15f, 600).apply { text = d.optString("day"); maxLines = 1 }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                addView(ImageView(context).apply { setImageDrawable(Icons.drawable(d.optString("icon", "sun"), Theme.FG)) },
                    LinearLayout.LayoutParams(context.dpi(20f), context.dpi(20f)).apply { rightMargin = context.dpi(14f) })
                addView(context.text(15f, 600, Theme.MUTED).apply { text = deg(d.optDouble("low")) }, LinearLayout.LayoutParams(context.dpi(38f), LinearLayout.LayoutParams.WRAP_CONTENT))
                addView(context.text(15f, 700).apply { text = deg(d.optDouble("high")); gravity = Gravity.END }, LinearLayout.LayoutParams(context.dpi(38f), LinearLayout.LayoutParams.WRAP_CONTENT))
            }, 8f)
        }
        card.optString("note").takeIf { it.isNotEmpty() }?.let { add(said(13f, 500, Theme.MUTED).apply { text = it }, 10f) }
    }

    fun image(card: JSONObject) {
        add(picture(card.optString("url"), 200f), 0f, height = context.dpi(200f))
        card.optString("caption").takeIf { it.isNotEmpty() }?.let { add(said(15f, 600).apply { text = it }) }
        meta(card.optString("source"), if (card.optString("link").isNotEmpty()) "click to open" else "").takeIf { it.isNotEmpty() }?.let {
            add(context.text(12f, 600, Theme.MUTED).apply { text = it }, 4f)
        }
    }

    fun stats(card: JSONObject) {
        caption(card.optString("title"))
        val items = card.optJSONArray("items").objects()
        for (pair in items.chunked(2)) {
            val row = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
            for (s in pair) {
                row.addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(context.text(13f, 600, Theme.MUTED).apply { text = s.optString("label"); maxLines = 1; ellipsize = TextUtils.TruncateAt.END })
                    addView(said(24f, 700, lines = 1).apply { text = s.opt("value")?.toString().orEmpty() })
                    val delta = s.optString("delta")
                    if (delta.isNotEmpty()) {
                        val trend = s.optString("trend")
                        val color = when (trend) { "up" -> Theme.OK; "down" -> Theme.BAD; else -> Theme.MUTED }
                        val arrow = when (trend) { "up" -> "▲ "; "down" -> "▼ "; else -> "" }
                        addView(said(13f, 700, color, 1).apply { text = arrow + delta })
                    }
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            }
            if (pair.size == 1 && items.size > 1) row.addView(View(context), LinearLayout.LayoutParams(0, 1, 1f))
            add(row, 10f)
        }
    }

    fun bars(card: JSONObject) {
        caption(card.optString("title"))
        val unit = card.optString("unit")
        val items = card.optJSONArray("items").objects()
        val max = items.maxOfOrNull { it.optDouble("value") }?.takeIf { it > 0 } ?: 1.0
        for (b in items) {
            val v = b.optDouble("value")
            add(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    addView(said(14f, 600, lines = 1).apply { text = b.optString("label") }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                    addView(context.text(14f, 700, Theme.MUTED).keepCase().apply { text = number(v) + unit })
                })
                addView(BarView(context, (v / max).toFloat(), accent), LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(8f)).apply { topMargin = context.dpi(4f) })
            }, 8f)
        }
    }

    fun line(card: JSONObject) {
        caption(card.optString("title"))
        val points = card.optJSONArray("points").objects()
        val values = points.map { it.optDouble("value") }
        val unit = card.optString("unit")
        val last = values.lastOrNull() ?: 0.0
        val first = values.firstOrNull() ?: 0.0
        val change = if (first != 0.0) (last - first) / abs(first) * 100 else 0.0
        add(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.BOTTOM
            addView(context.text(28f, 700).keepCase().apply { text = number(last) + unit })
            addView(context.text(14f, 700, if (change >= 0) Theme.OK else Theme.BAD).apply {
                text = (if (change >= 0) " ▲ " else " ▼ ") + "${abs(change).roundToInt()}%"
                setPadding(context.dpi(6f), 0, 0, context.dpi(4f))
            })
        }, 4f)
        add(LineView(context, values, accent), 8f, height = context.dpi(90f))
        add(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(said(12f, 600, Theme.MUTED, 1).apply { text = points.firstOrNull()?.optString("label").orEmpty() }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            addView(said(12f, 600, Theme.MUTED, 1).apply { text = points.lastOrNull()?.optString("label").orEmpty() })
        }, 4f)
    }

    fun steps(card: JSONObject) {
        caption(card.optString("title"))
        val items = card.optJSONArray("items").objects()
        items.forEachIndexed { i, s ->
            val row = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
            // The number in a dot, and a rule down to the next one: a small flow.
            row.addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER_HORIZONTAL
                addView(context.text(13f, 700, Theme.ON_ACCENT).apply {
                    text = "${i + 1}"
                    gravity = Gravity.CENTER
                    background = roundRect(accent, context.dp(999f))
                }, LinearLayout.LayoutParams(context.dpi(24f), context.dpi(24f)))
                if (i < items.size - 1) {
                    addView(View(context).apply { setBackgroundColor(Theme.withAlpha(accent, 0.35f)) },
                        LinearLayout.LayoutParams(context.dpi(2f), 0, 1f).apply { topMargin = context.dpi(4f) })
                }
            }, LinearLayout.LayoutParams(context.dpi(24f), LinearLayout.LayoutParams.MATCH_PARENT).apply { rightMargin = context.dpi(12f) })
            row.addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(0, context.dpi(2f), 0, context.dpi(if (i < items.size - 1) 12f else 0f))
                addView(said(16f, 700, lines = 2).apply { text = s.optString("title") })
                s.optString("detail").takeIf { it.isNotEmpty() }?.let { addView(said(14f, 500, Theme.MUTED, 4).apply { text = it; setPadding(0, context.dpi(2f), 0, 0) }) }
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            add(row, if (i == 0) 8f else 0f)
        }
    }

    fun table(card: JSONObject) {
        caption(card.optString("title"))
        val columns = card.optJSONArray("columns")
        fun row(cells: JSONArray?, head: Boolean) = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            for (i in 0 until (cells?.length() ?: 0)) {
                addView((if (head) context.text(12f, 700, Theme.MUTED) else said(14f, if (i == 0) 700 else 500, lines = 2)).apply {
                    text = cells?.optString(i).orEmpty()
                    if (i > 0) gravity = Gravity.END
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, if (i == 0) 1.3f else 1f).apply {
                    // Right-aligned cells would touch the one before (T-201).
                    if (i > 0) leftMargin = context.dpi(10f)
                })
            }
        }
        add(row(columns, true), 6f)
        val rows = card.optJSONArray("rows")
        for (i in 0 until (rows?.length() ?: 0)) {
            add(divider(), 6f, height = context.dpi(1f))
            add(row(rows?.optJSONArray(i), false), 6f)
        }
    }

    private fun divider() = View(context).apply { setBackgroundColor(Theme.LINE) }

    private fun number(v: Double): String = if (v == Math.floor(v) && abs(v) < 1e15) v.toLong().toString() else "%.2f".format(v).trimEnd('0').trimEnd('.')
}

/** A bar of [fraction] of the width, on a faint track. */
private class BarView(context: Context, private val fraction: Float, private val color: Int) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    override fun onDraw(canvas: Canvas) {
        val r = height / 2f
        paint.color = Theme.withAlpha(color, 0.14f)
        canvas.drawRoundRect(0f, 0f, width.toFloat(), height.toFloat(), r, r, paint)
        paint.color = color
        canvas.drawRoundRect(0f, 0f, maxOf(height.toFloat(), width * fraction.coerceIn(0f, 1f)), height.toFloat(), r, r, paint)
    }
}

/** A line through [values], scaled to fit, with a soft fill under it and a dot on the last one. */
private class LineView(context: Context, private val values: List<Double>, private val color: Int) : View(context) {
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = context.dp(2.5f)
        strokeJoin = Paint.Join.ROUND
        strokeCap = Paint.Cap.ROUND
        this.color = color
    }
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { this.color = Theme.withAlpha(color, 0.12f) }
    private val dot = Paint(Paint.ANTI_ALIAS_FLAG).apply { this.color = color }

    override fun onDraw(canvas: Canvas) {
        if (values.size < 2) return
        val pad = context.dp(5f)
        val lo = values.min()
        val span = (values.max() - lo).takeIf { it > 0 } ?: 1.0
        val w = width - 2 * pad
        val h = height - 2 * pad
        fun x(i: Int) = pad + w * i / (values.size - 1)
        fun y(v: Double) = (pad + h - h * ((v - lo) / span)).toFloat()
        val line = Path()
        values.forEachIndexed { i, v -> if (i == 0) line.moveTo(x(i), y(v)) else line.lineTo(x(i), y(v)) }
        val area = Path(line).apply {
            lineTo(x(values.size - 1), height.toFloat())
            lineTo(x(0), height.toFloat())
            close()
        }
        canvas.drawPath(area, fill)
        canvas.drawPath(line, stroke)
        canvas.drawCircle(x(values.size - 1), y(values.last()), context.dp(4f), dot)
    }
}

/** Cards' pictures, through r1d (`GET /api/cards/image`): the host never fetches the web itself. */
object CardImages {
    private val cache = object : LruCache<String, Bitmap>(12 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
    }

    fun show(view: ImageView, url: String, px: Int) {
        val key = "$url@$px"
        view.tag = key
        cache.get(key)?.let { view.setImageBitmap(it); return }
        R1d.bytes("/api/cards/image?url=${R1d.enc(url)}") { bytes ->
            val bmp = bytes?.let { decode(it, px) } ?: return@bytes
            cache.put(key, bmp)
            if (view.tag == key) {
                view.alpha = 0f
                view.setImageBitmap(bmp)
                view.animate().alpha(1f).setDuration(180).start()
            }
        }
    }

    /** About `px` on its long side (sampled down, so a big picture costs little memory). */
    private fun decode(bytes: ByteArray, px: Int): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        var sample = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= px) sample *= 2
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
    }
}

/** A card's links (headlines' stories): the wheel moves between them, a click opens one in the R1's reader. */
class CardScreen(shell: Shell, private val card: JSONObject, accent: Int) :
    ListPage(shell, card.optString("title").ifEmpty { card.optString("type") }, accent) {
    init {
        list.rebuild {
            for (item in cardLinks(card)) {
                val row = LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    setPadding(context.dpi(8f), context.dpi(10f), context.dpi(8f), context.dpi(10f))
                }
                item.optString("image").takeIf { it.isNotEmpty() }?.let {
                    row.addView(ImageView(context).apply {
                        scaleType = ImageView.ScaleType.CENTER_CROP
                        background = roundRect(Theme.FOCUS, context.dp(10f))
                        clipToOutline = true
                        CardImages.show(this, it, context.dpi(112f))
                    }, LinearLayout.LayoutParams(context.dpi(56f), context.dpi(56f)).apply { rightMargin = context.dpi(12f) })
                }
                row.addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(context.text(17f, 700).keepCase().apply { text = item.optString("title"); maxLines = 3; ellipsize = TextUtils.TruncateAt.END })
                    item.optString("tagline").ifEmpty { item.optString("summary") }.takeIf { it.isNotEmpty() }?.let {
                        addView(context.text(14f, 500, Theme.MUTED).keepCase().apply { text = it; maxLines = 2; ellipsize = TextUtils.TruncateAt.END; setPadding(0, context.dpi(3f), 0, 0) })
                    }
                    listOf(item.optString("source"), item.optString("time")).filter { it.isNotBlank() }.joinToString(" · ").takeIf { it.isNotEmpty() }?.let {
                        addView(context.text(12f, 600, Theme.MUTED).keepCase().apply { text = it; setPadding(0, context.dpi(3f), 0, 0) })
                    }
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                card(row) { shell.open(Target.of("link", "url" to item.optString("url"))) }
            }
        }
    }
}
