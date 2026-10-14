package dev.r1os.host.ime

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.view.View
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp

/**
 * r1os's keyboard (ADR-0055): big rounded keys in the look's colours and font, laid out for the R1 turned on its side
 * (and still usable upright, for an app that won't turn). Letters, then numbers and symbols on two pages; shift (twice
 * for caps), backspace (held, it keeps going), and the field's own action on the big key (go, search, send, next,
 * done). Holding a letter on the top row types its number. A strip above says what the field is for, with a key to put
 * the keyboard away.
 */
@SuppressLint("ViewConstructor")
class KeyboardView(context: Context, private val ime: R1Keyboard) : View(context) {
    private enum class Page { LETTERS, SYMBOLS, MORE }
    private data class Key(val label: String, val code: String, val wide: Float = 1f, val icon: String? = null, val special: Boolean = false)
    private data class Placed(val key: Key, val rect: RectF)

    private var page = Page.LETTERS
    /** 0: off, 1: the next letter, 2: caps lock. */
    var shift = 0
        set(v) { field = v; invalidate() }
    /** What the big key says: "go", "search", "send", "next", "done", or ↵. */
    var action = "↵"
        set(v) { field = v; layoutKeys(); invalidate() }
    /** What the field is for (its hint), for the strip. */
    var hint = ""
        set(v) { field = v; invalidate() }

    private var placed: List<Placed> = emptyList()
    private var pressed: Placed? = null
    private var longFired = false
    private val main = Handler(Looper.getMainLooper())
    private var hideRect = RectF()

    private val keyPaint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { textAlign = Paint.Align.CENTER; typeface = font(600) }
    private val smallPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { textAlign = Paint.Align.LEFT; typeface = font(500) }

    private fun font(w: Int): Typeface = runCatching { Theme.font(w) }.getOrDefault(Typeface.DEFAULT_BOLD)

    /** Icons come from r1d a moment after they're asked for: kept, and the keyboard redraws as each arrives. */
    private val icons = HashMap<String, android.graphics.drawable.Drawable>()
    private fun icon(name: String, color: Int): android.graphics.drawable.Drawable =
        icons.getOrPut("$name/$color") { Icons.drawable(name, color).also { it.callback = this } }

    override fun verifyDrawable(who: android.graphics.drawable.Drawable): Boolean = who in icons.values || super.verifyDrawable(who)

    private val strip get() = context.dp(STRIP_DP)
    private val gap get() = context.dp(5f)

    private fun rows(): List<List<Key>> {
        val letters = { s: String -> s.map { c -> Key(c.toString(), c.toString()) } }
        val shiftKey = Key("shift", "shift", 1.4f, icon = if (shift == 2) "arrow-fat-line-up" else "arrow-fat-up", special = true)
        val back = Key("back", "back", 1.4f, icon = "backspace", special = true)
        val bottom = { toggle: String ->
            listOf(
                Key(toggle, if (toggle == "abc") "letters" else "symbols", 1.4f, special = true),
                Key(",", ","),
                Key("", " ", 4.6f),
                Key(".", "."),
                Key(action, "action", 1.9f, icon = if (action == "↵") "arrow-elbow-down-left" else null, special = true),
            )
        }
        return when (page) {
            Page.LETTERS -> listOf(letters("qwertyuiop"), letters("asdfghjkl"), listOf(shiftKey) + letters("zxcvbnm") + back, bottom("123"))
            Page.SYMBOLS -> listOf(
                letters("1234567890"), letters("@#\$_&-+()/"),
                listOf(Key("=\\<", "more", 1.4f, special = true)) + letters("*\"':;!?") + back, bottom("abc"),
            )
            Page.MORE -> listOf(
                letters("~`|•√π÷×¶∆"), letters("£€¥^°={}\\"),
                listOf(Key("?123", "symbols", 1.4f, special = true)) + letters("%©®™✓[]") + back, bottom("abc"),
            )
        }
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val w = MeasureSpec.getSize(widthMeasureSpec)
        // On its side the R1 is 640×480: keys take a little over half; upright, a little less.
        val landscape = resources.configuration.orientation == android.content.res.Configuration.ORIENTATION_LANDSCAPE
        val h = (strip + context.dp(if (landscape) KEYS_LAND_DP else KEYS_PORT_DP)).toInt()
        setMeasuredDimension(w, h)
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) = layoutKeys()

    private fun layoutKeys() {
        if (width == 0) return
        val rs = rows()
        val top = strip
        val rowH = (height - top - gap) / rs.size
        val out = ArrayList<Placed>()
        for ((i, row) in rs.withIndex()) {
            val units = row.sumOf { it.wide.toDouble() }.toFloat()
            // Rows of letters line up with the top row's keys: a short row is centred.
            val full = maxOf(10f, units)
            val unit = (width - gap) / full
            var x = gap + (full - units) * unit / 2f
            val y = top + i * rowH
            for (k in row) {
                val r = RectF(x, y, x + k.wide * unit - gap, y + rowH - gap)
                out.add(Placed(k, r))
                x += k.wide * unit
            }
        }
        placed = out
        hideRect = RectF(width - context.dp(52f), 0f, width.toFloat(), strip)
    }

    override fun onDraw(c: Canvas) {
        c.drawColor(Theme.BG)
        // The strip: what the field is for, and the key that puts the keyboard away.
        smallPaint.color = Theme.MUTED
        smallPaint.textSize = context.dp(13f)
        c.drawText(hint.ifEmpty { "type, or hold the button to talk" }, context.dp(14f), strip / 2f + context.dp(5f), smallPaint)
        icon("caret-down", Theme.MUTED).let { d ->
            val s = context.dp(20f).toInt()
            val cx = hideRect.centerX().toInt()
            val cy = hideRect.centerY().toInt()
            d.setBounds(cx - s / 2, cy - s / 2, cx + s / 2, cy + s / 2)
            d.draw(c)
        }
        val radius = context.dp(10f)
        for (p in placed) {
            val k = p.key
            val down = pressed === p
            val isAction = k.code == "action"
            val lit = (k.code == "shift" && shift > 0) || isAction
            keyPaint.color = when {
                down -> Theme.accent
                lit -> if (isAction) Theme.accent else Theme.withAlpha(Theme.accent, 0.35f)
                k.special -> Theme.FOCUS
                else -> Theme.CARD
            }
            c.drawRoundRect(p.rect, radius, radius, keyPaint)
            val ink = if (down || isAction) Theme.BG else Theme.FG
            if (k.icon != null) {
                icon(k.icon, ink).let { d ->
                    val s = (minOf(p.rect.width(), p.rect.height()) * 0.42f).toInt()
                    val cx = p.rect.centerX().toInt()
                    val cy = p.rect.centerY().toInt()
                    d.setBounds(cx - s / 2, cy - s / 2, cx + s / 2, cy + s / 2)
                    d.draw(c)
                }
                continue
            }
            val label = if (k.code.length == 1 && k.code[0].isLetter() && shift > 0) k.label.uppercase() else k.label
            textPaint.color = ink
            textPaint.textSize = if (label.length > 1) context.dp(15f) else minOf(context.dp(24f), p.rect.height() * 0.48f)
            c.drawText(label, p.rect.centerX(), p.rect.centerY() - (textPaint.descent() + textPaint.ascent()) / 2f, textPaint)
            // The number a top-row letter types when held.
            NUMBERS[k.code]?.takeIf { page == Page.LETTERS }?.let { n ->
                smallPaint.color = if (down) Theme.BG else Theme.MUTED
                smallPaint.textSize = context.dp(10f)
                c.drawText(n, p.rect.right - context.dp(11f), p.rect.top + context.dp(13f), smallPaint)
            }
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                if (hideRect.contains(e.x, e.y)) { ime.hideKeyboard(); return true }
                press(at(e.x, e.y))
            }
            MotionEvent.ACTION_MOVE -> {
                val now = at(e.x, e.y)
                if (now !== pressed && !longFired) press(now)
            }
            MotionEvent.ACTION_UP -> {
                val p = pressed
                main.removeCallbacksAndMessages(null)
                if (p != null && !longFired && p.key.code != "back") type(p.key)
                pressed = null
                longFired = false
                invalidate()
            }
            MotionEvent.ACTION_CANCEL -> {
                main.removeCallbacksAndMessages(null)
                pressed = null
                invalidate()
            }
        }
        return true
    }

    private fun at(x: Float, y: Float): Placed? = placed.firstOrNull { it.rect.contains(x, y) }
        ?: placed.minByOrNull { val dx = it.rect.centerX() - x; val dy = it.rect.centerY() - y; dx * dx + dy * dy }?.takeIf { y > strip }

    private fun press(p: Placed?) {
        main.removeCallbacksAndMessages(null)
        pressed = p
        longFired = false
        invalidate()
        if (p == null) return
        Sounds.play(Sounds.S.TICK)
        val code = p.key.code
        when {
            // Backspace goes at once, then again and again while it's held, faster as it goes.
            code == "back" -> {
                ime.backspace()
                var every = 380L
                val again = object : Runnable {
                    override fun run() {
                        if (pressed !== p) return
                        ime.backspace()
                        every = maxOf(45L, (every * 0.8).toLong())
                        main.postDelayed(this, every)
                    }
                }
                main.postDelayed(again, every)
            }
            NUMBERS.containsKey(code) && page == Page.LETTERS -> main.postDelayed({
                if (pressed === p) {
                    longFired = true
                    Sounds.play(Sounds.S.POP)
                    ime.commit(NUMBERS.getValue(code))
                }
            }, LONG_MS)
        }
    }

    private var lastShift = 0L

    private fun type(k: Key) {
        when (k.code) {
            "shift" -> {
                val now = android.os.SystemClock.uptimeMillis()
                shift = when {
                    shift == 2 -> 0
                    now - lastShift < 350 -> 2
                    shift == 1 -> 0
                    else -> 1
                }
                lastShift = now
            }
            "letters" -> { page = Page.LETTERS; layoutKeys(); invalidate() }
            "symbols" -> { page = Page.SYMBOLS; layoutKeys(); invalidate() }
            "more" -> { page = Page.MORE; layoutKeys(); invalidate() }
            "action" -> ime.action()
            else -> {
                val text = if (shift > 0 && k.code.length == 1 && k.code[0].isLetter()) k.code.uppercase() else k.code
                ime.commit(text)
                if (shift == 1) shift = 0
            }
        }
    }

    /** Numbers first (a number or phone field), else letters. */
    fun start(numbers: Boolean) {
        page = if (numbers) Page.SYMBOLS else Page.LETTERS
        layoutKeys()
        invalidate()
    }

    companion object {
        const val STRIP_DP = 30f
        const val KEYS_LAND_DP = 196f
        const val KEYS_PORT_DP = 220f
        const val LONG_MS = 380L
        val NUMBERS = mapOf("q" to "1", "w" to "2", "e" to "3", "r" to "4", "t" to "5", "y" to "6", "u" to "7", "i" to "8", "o" to "9", "p" to "0")
    }
}
