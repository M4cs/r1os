package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.drawable.Drawable
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import dev.r1os.host.ui.IconDrawable
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Spring
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.traced
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text

/**
 * A wheel-driven list: every native screen below home is one. The wheel moves the focus, a click
 * activates it; touch works too.
 *
 * The look (ADR-0021): big, bold, lowercase labels on black, and the focused row filled solid in
 * the accent with its text in near-black, like ink on orange. No outlines. The fill springs from
 * row to row, stretching to each row's size, and the wheel ticks as it goes.
 *
 * Screens rebuild the list when their state changes ([rebuild]); the focus stays on the same row.
 */
@SuppressLint("ViewConstructor")
class FocusList(context: Context) : ScrollView(context) {
    private class Row(val view: View, val onActivate: () -> Unit, val onFocus: (Boolean) -> Unit)

    private val frame = FrameLayout(context).apply { clipChildren = false }
    private val column = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val highlight = Highlight(context)
    private val rows = ArrayList<Row>()

    /** The fill's colour (the agent's, on its pages). */
    var accent = Theme.accent
        set(v) { field = v; highlight.color = v }

    /** Row size: 1 for lists, larger for the launcher (bigger type, tiles and padding). */
    var scale = 1f

    var focus = -1
        private set
    val size get() = rows.size
    private var gap = context.dpi(4f)

    init {
        isVerticalScrollBarEnabled = false
        overScrollMode = OVER_SCROLL_NEVER
        isFocusable = false
        clipToPadding = false
        highlight.color = accent
        frame.addView(highlight, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        frame.addView(column, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT))
        addView(frame)
    }

    /** Space between rows (lists 4 dp; forms 10). */
    fun gap(dp: Float) { gap = context.dpi(dp) }

    fun clear() {
        rows.clear()
        column.removeAllViews()
        focus = -1
        highlight.hide(animate = false)
    }

    /**
     * Clears and refills the list, keeping the focus on the same row: the one with the same `tag` (an id) if the focused
     * row has one, so rows arriving above it don't slide the focus onto another (T-006: Accounts' focus moved from
     * "connect an account" to "sign out" as the accounts loaded); else the same place, or the nearest.
     */
    fun rebuild(build: FocusList.() -> Unit) = traced("r1os rebuild") {
        val keep = focus
        val keepTag = rows.getOrNull(keep)?.view?.tag
        clear()
        build()
        if (rows.isNotEmpty()) {
            val same = keepTag?.let { t -> rows.indexOfFirst { it.view.tag == t } }?.takeIf { it >= 0 }
            setFocus(same ?: keep.coerceIn(0, rows.size - 1), animate = false)
        }
    }

    // ---- rows that don't take the focus ------------------------------------------------------------

    /** A caption between groups: lowercase Rubik, quieter than the rows (Max, 2026-10-09). */
    fun section(label: String) {
        column.addView(context.text(15f * (1f + (scale - 1f) / 2f), 600, Theme.MUTED).apply {
            text = label
            setPadding(context.dpi(10f), context.dpi(18f), 0, context.dpi(4f))
        })
    }

    /** Text that isn't a control: a hint, an empty state, a message. */
    fun note(s: String, color: Int = Theme.MUTED, size: Float = 15f, center: Boolean = false, weight: Int = 500): TextView =
        context.text(size, weight, color).apply {
            text = s
            setLineSpacing(0f, 1.15f)
            if (center) gravity = Gravity.CENTER_HORIZONTAL
            setPadding(context.dpi(8f), context.dpi(6f), context.dpi(8f), context.dpi(6f))
        }.also { place(it) }

    /** Any view that isn't a control. */
    fun place(v: View, matchWidth: Boolean = true, gravity: Int = Gravity.START) {
        column.addView(v, LinearLayout.LayoutParams(if (matchWidth) LinearLayout.LayoutParams.MATCH_PARENT else LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            this.gravity = gravity
            if (column.childCount > 0) topMargin = gap
        })
    }

    // ---- rows that do -----------------------------------------------------------------------------

    /**
     * A row: an icon in a tinted tile, a big bold label, a second line, and something small on the
     * right (all lowercase, like everything the shell shows: `Lowercase`).
     * `danger` fills it red when focused (a delete that wants a second click).
     */
    fun item(
        icon: String?,
        label: String,
        accent: Int? = null,
        bold: Boolean = true,
        sub: String? = null,
        trailing: String? = null,
        trailingView: View? = null,
        danger: Boolean = false,
        /** A picture instead of [icon]: an Android app's own icon, drawn as it is. */
        image: Drawable? = null,
        /** The label's own typeface (Settings › Look shows each font in itself). */
        font: android.graphics.Typeface? = null,
        /** A long press (touch): the launcher pins an app to home with it (ADR-0044). */
        onLong: (() -> Unit)? = null,
        onActivate: () -> Unit,
    ): LinearLayout {
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            val v = 10f * scale
            setPadding(context.dpi(10f), context.dpi(v), context.dpi(12f), context.dpi(v))
        }
        val tint = if (danger) Theme.BAD else accent ?: Theme.ICON
        var box: FrameLayout? = null
        var iconDrawable: IconDrawable? = null
        if (image != null) {
            row.addView(ImageView(context).apply { setImageDrawable(image); scaleType = ImageView.ScaleType.FIT_CENTER },
                LinearLayout.LayoutParams(context.dpi(38f * scale), context.dpi(38f * scale)).apply { rightMargin = context.dpi(12f * scale) })
        } else if (icon != null) {
            iconDrawable = Icons.drawable(icon, tint)
            box = FrameLayout(context).apply {
                background = roundRect(Theme.withAlpha(tint, 0.18f), context.dp(12f))
                addView(ImageView(context).apply { setImageDrawable(iconDrawable) },
                    FrameLayout.LayoutParams(context.dpi(20f * scale), context.dpi(20f * scale), Gravity.CENTER))
            }
            row.addView(box, LinearLayout.LayoutParams(context.dpi(38f * scale), context.dpi(38f * scale)).apply { rightMargin = context.dpi(12f * scale) })
        }
        val labelColor = if (danger) Theme.BAD else Theme.FG
        val texts = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        val labelView = context.text((if (sub != null) 19f else 21f) * scale, if (bold) 700 else 500, labelColor).apply {
            text = label
            maxLines = if (sub != null) 1 else 2
            ellipsize = TextUtils.TruncateAt.END
            if (font != null) typeface = font
        }
        texts.addView(labelView)
        val subView = sub?.let {
            context.text(14f, 500, Theme.MUTED).apply {
                text = it
                maxLines = 1
                ellipsize = TextUtils.TruncateAt.END
                setPadding(0, context.dpi(2f), 0, 0)
            }.also { v -> texts.addView(v) }
        }
        row.addView(texts, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        val trailingText = if (trailingView == null && trailing != null) context.text(14f, 600, Theme.MUTED).apply { text = trailing; maxLines = 1 } else null
        (trailingView ?: trailingText)?.let {
            row.addView(it, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = context.dpi(8f) })
        }
        if (onLong != null) row.setOnLongClickListener { onLong(); true }
        val boxRest = box?.background
        add(row, highlightColor = if (danger) Theme.BAD else null, onFocus = { on ->
            labelView.setTextColor(if (on) Theme.ON_ACCENT else labelColor)
            subView?.setTextColor(if (on) Theme.withAlpha(Theme.ON_ACCENT, 0.7f) else Theme.MUTED)
            trailingText?.setTextColor(if (on) Theme.withAlpha(Theme.ON_ACCENT, 0.7f) else Theme.MUTED)
            iconDrawable?.setColor(if (on) Theme.ON_ACCENT else tint)
            box?.background = if (on) roundRect(Theme.withAlpha(Color.BLACK, 0.12f), context.dp(12f)) else boxRest
        }, onActivate = onActivate)
        return row
    }

    /**
     * A button: a big label in a dark pill (primary: in the accent's colour; quiet: muted). Focused,
     * the pill fills with the accent like any row. Disabled ones skip the focus.
     */
    fun button(label: String, primary: Boolean = true, accent: Int = this.accent, enabled: Boolean = true, small: Boolean = false, onActivate: () -> Unit): TextView {
        val color = if (primary) accent else Theme.MUTED
        val rest = if (primary) roundRect(Theme.FOCUS, context.dp(14f)) else null
        val b = context.text(if (small) 15f else 20f, 700, color).apply {
            text = label
            gravity = Gravity.CENTER
            val v = if (small) 7f else 13f
            setPadding(context.dpi(14f), context.dpi(v), context.dpi(14f), context.dpi(v))
            background = rest
        }
        if (enabled) {
            add(b, matchWidth = !small, onFocus = { on ->
                b.setTextColor(if (on) Theme.ON_ACCENT else color)
                b.background = if (on) null else rest
            }, onActivate = onActivate)
        } else {
            b.alpha = 0.35f
            place(b, matchWidth = !small, gravity = Gravity.CENTER_HORIZONTAL)
        }
        return b
    }

    /** A row of your own (a Wi-Fi network, an account): its text turns ink and its fill gives way when focused. */
    fun card(v: View, onActivate: () -> Unit) = add(v, onFocus = { on -> inkOn(v, on) }, onActivate = onActivate)

    /** A text field: a click puts the cursor in it and brings up the keyboard. */
    fun field(hint: String, value: String = "", maxLength: Int = 120, password: Boolean = false): EditText {
        val rest = roundRect(Theme.FOCUS, context.dp(14f))
        val f = EditText(context).apply {
            setText(value)
            this.hint = hint.lowercase()
            setHintTextColor(Theme.MUTED)
            setTextColor(Theme.FG)
            textSize = 19f
            typeface = Theme.font(600)
            isSingleLine = true
            filters = arrayOf(android.text.InputFilter.LengthFilter(maxLength))
            if (password) inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
            background = rest
            setPadding(context.dpi(14f), context.dpi(13f), context.dpi(14f), context.dpi(13f))
            imeOptions = android.view.inputmethod.EditorInfo.IME_ACTION_DONE
        }
        add(f, onFocus = { on ->
            f.setTextColor(if (on) Theme.ON_ACCENT else Theme.FG)
            f.setHintTextColor(if (on) Theme.withAlpha(Theme.ON_ACCENT, 0.6f) else Theme.MUTED)
            f.background = if (on) null else rest
        }, onActivate = {
            f.requestFocus()
            f.setSelection(f.text.length)
            context.getSystemService(InputMethodManager::class.java).showSoftInput(f, InputMethodManager.SHOW_IMPLICIT)
        })
        return f
    }

    fun add(
        view: View,
        matchWidth: Boolean = true,
        highlightColor: Int? = null,
        onFocus: (Boolean) -> Unit = { on -> inkOn(view, on) },
        onActivate: () -> Unit,
    ) {
        register(view, highlightColor, onFocus, onActivate)
        column.addView(view, LinearLayout.LayoutParams(if (matchWidth) LinearLayout.LayoutParams.MATCH_PARENT else LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            if (column.childCount > 0) topMargin = gap
        })
    }

    /** A control inside a container placed with [place] (a chip in a wrapping row). */
    fun register(view: View, highlightColor: Int? = null, onFocus: (Boolean) -> Unit = { on -> inkOn(view, on) }, onActivate: () -> Unit) {
        val r = Row(view, onActivate) { on ->
            onFocus(on)
            if (on) highlight.color = highlightColor ?: accent
        }
        rows += r
        if (view !is EditText) view.setOnClickListener {
            setFocus(rows.indexOf(r))
            Sounds.play(Sounds.S.POP)
            r.onActivate()
        }
    }

    // ---- focus ------------------------------------------------------------------------------------

    /** Moves the focus by `delta` rows. False when it would leave the list (the caller's overscroll). */
    fun move(delta: Int): Boolean {
        if (rows.isEmpty()) return false
        val next = if (focus < 0) (if (delta > 0) 0 else rows.size - 1) else focus + delta
        if (next < 0 || next >= rows.size) {
            // Past the last row: show whatever's left below it (a message, the history).
            if (delta > 0) smoothScrollBy(0, context.dpi(80f))
            return false
        }
        setFocus(next)
        Sounds.play(Sounds.S.TICK)
        return true
    }

    fun activate(): Boolean {
        val r = rows.getOrNull(focus) ?: return false
        Sounds.play(Sounds.S.POP)
        r.onActivate()
        return true
    }

    /** The focused row's view (a screen asks which chat is under the focus). */
    fun focused(): View? = rows.getOrNull(focus)?.view

    fun setFocus(i: Int, animate: Boolean = true) {
        if (i !in rows.indices) return
        val old = rows.getOrNull(focus)
        focus = i
        val r = rows[i]
        if (old !== r) old?.onFocus?.invoke(false)
        r.onFocus(true)
        val v = r.view
        val place = Runnable {
            if (focus != i || rows.getOrNull(i) !== r) return@Runnable
            // Where the row is in the column (a chip sits inside a wrapping row).
            val at = android.graphics.Rect(0, 0, v.width, v.height)
            column.offsetDescendantRectToMyCoords(v, at)
            highlight.moveTo(at.left.toFloat(), at.top.toFloat(), at.right.toFloat(), at.bottom.toFloat(), animate)
            // Keep the focus clear of the edges.
            val margin = context.dpi(28f)
            val top = paddingTop + at.top - margin
            val bottom = paddingTop + at.bottom + margin - height
            when {
                top < scrollY -> if (animate) smoothScrollTo(0, top.coerceAtLeast(0)) else scrollTo(0, top.coerceAtLeast(0))
                bottom > scrollY -> if (animate) smoothScrollTo(0, bottom) else scrollTo(0, bottom)
            }
        }
        if (v.isLaidOut && v.height > 0 && !v.isLayoutRequested) place.run() else v.post(place)
    }

    /** Hides the fill (nothing focused). */
    fun blur() {
        rows.getOrNull(focus)?.onFocus?.invoke(false)
        focus = -1
        highlight.hide(animate = true)
    }

    /** The fill: a rounded rect whose four edges each follow a spring, so it moves and stretches like jelly. */
    private class Highlight(context: Context) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val radius = context.dp(14f)
        private var l = 0f
        private var t = 0f
        private var r = 0f
        private var b = 0f
        private var shown = false
        private val sl = Spring { l = it; invalidate() }
        private val st = Spring { t = it; invalidate() }
        private val sr = Spring { r = it; invalidate() }
        private val sb = Spring { b = it; invalidate() }

        var color = Color.WHITE
            set(v) { field = v; paint.color = v; invalidate() }

        fun moveTo(left: Float, top: Float, right: Float, bottom: Float, animate: Boolean) {
            if (!animate || !shown) {
                for ((s, v) in listOf(sl to left, st to top, sr to right, sb to bottom)) s.snapTo(v)
                shown = true
                alpha = 0f
                animate().alpha(1f).setDuration(140).start()
                return
            }
            // The leading edge moves first and the trailing one follows: a little stretch.
            val down = top > t
            sl.animateTo(left, STIFF, DAMP)
            sr.animateTo(right, STIFF, DAMP)
            st.animateTo(top, if (down) SOFT else STIFF, DAMP)
            sb.animateTo(bottom, if (down) STIFF else SOFT, DAMP)
        }

        fun hide(animate: Boolean) {
            shown = false
            if (animate) animate().alpha(0f).setDuration(120).start() else alpha = 0f
        }

        override fun onDraw(c: Canvas) {
            if (!shown && alpha == 0f) return
            if (b > t) c.drawRoundRect(l, t, r, b, radius, radius, paint)
        }

        companion object {
            const val STIFF = 520f
            const val SOFT = 300f
            const val DAMP = 0.72f
        }
    }
}

/** Ink on the accent: a focused row's text and icons turn near-black, and its own fill gives way. */
private val restColors = java.util.WeakHashMap<View, Int>()
private val restBackgrounds = java.util.WeakHashMap<View, Drawable?>()

fun inkOn(v: View, on: Boolean) {
    if (on) {
        if (!restBackgrounds.containsKey(v)) restBackgrounds[v] = v.background
        v.background = null
    } else if (restBackgrounds.containsKey(v)) {
        v.background = restBackgrounds.remove(v)
    }
    fun walk(x: View) {
        when (x) {
            is TextView -> {
                if (on) { if (!restColors.containsKey(x)) restColors[x] = x.currentTextColor; x.setTextColor(Theme.ON_ACCENT) }
                else restColors.remove(x)?.let { x.setTextColor(it) }
            }
            is ImageView -> (x.drawable as? IconDrawable)?.let { d ->
                if (on) { if (!restColors.containsKey(x)) restColors[x] = d.color; d.setColor(Theme.ON_ACCENT) }
                else restColors.remove(x)?.let { d.setColor(it) }
            }
            is ViewGroup -> for (i in 0 until x.childCount) walk(x.getChildAt(i))
        }
    }
    walk(v)
}
