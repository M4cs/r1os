package dev.r1os.host.shell

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import dev.r1os.host.net.Ev
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * Where to go: the web shell's `Target` (shell/src/launcher.tsx), kept as its JSON so screens that
 * aren't native yet can hand it to the web fallback unchanged (ADR-0017 decision 5).
 */
class Target(val json: JSONObject) {
    val t: String get() = json.optString("t")
    fun str(key: String): String? = if (json.has(key) && !json.isNull(key)) json.optString(key).ifEmpty { null } else null

    /** The agent a screen belongs to, if it belongs to one (App.tsx agentOf). */
    val agent: String?
        get() = when (t) {
            "chat", "chats", "memory" -> str("agent")
            "agent", "agent-look" -> str("id")
            else -> null
        }

    override fun toString() = json.toString()

    companion object {
        fun of(t: String, vararg kv: Pair<String, Any?>) = Target(JSONObject().apply {
            put("t", t)
            for ((k, v) in kv) if (v != null) put(k, v)
        })

        fun parse(s: String): Target? = runCatching { Target(JSONObject(s)) }.getOrNull()?.takeIf { it.t.isNotEmpty() }
    }
}

/**
 * One screen of the native shell. The shell stacks them over home: double tap (and the overscroll
 * Back, ADR-0014) pops back to whatever was underneath.
 */
abstract class Screen(val shell: Shell) {
    val context: Context get() = shell.context
    abstract val view: View

    /** What opened it ([Shell.open]): a change of look builds the shell again and opens it again from this. */
    var opener: Target? = null

    /** Where its focus is (a list's row), kept over a rebuild; -1: nowhere to keep. */
    open fun focusIndex(): Int = -1
    open fun restoreFocus(i: Int) {}

    /** The agent this screen belongs to: push-to-talk shows its face, and words go to it. */
    open val agentId: String? = null

    /** An event from r1d, after the shell's own handling. True when this screen consumed it. */
    open fun onEvent(ev: Ev): Boolean = false

    /** Every event, whether or not this screen is on top (agent states, builds, app changes). */
    open fun onAnyEvent(ev: Ev) {}

    /** The agents changed: the list, or one's state, tool or unread count. */
    open fun onAgents() {}

    /** On top and on screen (also when the display comes back on). */
    open fun onShow() {}

    /** Covered by another screen, or the display went off. */
    open fun onHide() {}

    /** Docked on its side (T-156), or upright again: what wants the wheel and the button turns to touch. */
    open fun onDock(on: Boolean) {}

    /** Its header, when it has one: a tap lights it as Back (T-156). Without one, two taps at the top go back (the shell's). */
    open val backHeader: BackHeader? get() = null

    /** r1d came back after a restart: a deploy may have replaced what web pages load. */
    open fun onReconnect() {}

    /** Popped for good. */
    open fun onDestroy() {}
}

/**
 * A page's header (ADR-0014, ADR-0021): its title, big and bold. One wheel tick past the top of the
 * page makes the title itself the Back control: a caret slides in beside it and the title fills
 * with the accent, ink on top, same words, same size. While it's lit, a click goes back and a tick
 * down puts it out (the page's own highlight steps aside meanwhile: [PageScreen]). By touch the same: a
 * tap lights it, a second tap goes back ([onBackTouched]).
 */
class BackHeader(context: Context, private val content: View, private val accent: Int = Theme.FG, height: Float = 52f) : FrameLayout(context) {
    constructor(context: Context, title: String, accent: Int = Theme.FG) : this(context, context.text(28f, 700).apply {
        text = title
        maxLines = 1
        ellipsize = android.text.TextUtils.TruncateAt.END
        gravity = Gravity.CENTER_VERTICAL
        letterSpacing = -0.01f
    }, accent)

    private val caretIcon = Icons.drawable("caret-left", Theme.FG)
    private val caret = ImageView(context).apply {
        setImageDrawable(caretIcon)
        visibility = GONE
    }
    private val fill = GradientDrawable().apply {
        cornerRadius = context.dp(16f)
        setColor(accent)
        alpha = 0
    }
    /** The caret and the title, on the fill: the part that lights up. */
    private val chip = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(PAD), context.dpi(4f), context.dpi(PAD), context.dpi(4f))
        background = fill
        addView(caret, LinearLayout.LayoutParams(context.dpi(22f), context.dpi(22f)).apply { rightMargin = context.dpi(6f) })
        addView(content, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
    }
    private var fade: android.animation.ValueAnimator? = null

    var shown = false
        private set

    init {
        // Place the header [INSET] dp left of the page's margin: the chip's own padding brings the
        // title back in line, and the lit fill stays inside the header (pages keep clipping, so a
        // list scrolling under the header is hidden by it).
        addView(chip, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER_VERTICAL))
        minimumHeight = context.dpi(height)
    }

    fun setTitle(s: String) { (content as? TextView)?.text = s }

    fun show() {
        if (shown) return
        shown = true
        Sounds.play(Sounds.S.BUMP)
        caret.visibility = VISIBLE
        caret.alpha = 0f
        caret.scaleX = 0.4f
        caret.scaleY = 0.4f
        caret.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(280).setInterpolator(android.view.animation.OvershootInterpolator(2.2f)).start()
        // The title makes room for the caret: it glides over rather than jumping.
        val shift = context.dp(28f)
        content.translationX = -shift
        content.animate().translationX(0f).setDuration(260).setInterpolator(Theme.SETTLE).start()
        ink(true)
        fadeFill(255)
    }

    fun hide() {
        dim?.let { removeCallbacks(it) }
        if (!shown) return
        shown = false
        caret.animate().alpha(0f).scaleX(0.4f).scaleY(0.4f).setDuration(140).withEndAction {
            if (!shown) {
                caret.visibility = GONE
                content.translationX = context.dp(28f)
                content.animate().translationX(0f).setDuration(200).setInterpolator(Theme.SETTLE).start()
            }
        }.start()
        ink(false)
        fadeFill(0)
    }

    private fun ink(on: Boolean) {
        inkOn(content, on)
        caretIcon.setColor(if (on) Theme.ON_ACCENT else Theme.FG)
    }

    private fun fadeFill(to: Int) {
        fade?.cancel()
        fade = android.animation.ValueAnimator.ofInt(fill.alpha, to).apply {
            duration = 180
            addUpdateListener { fill.alpha = it.animatedValue as Int }
            start()
        }
    }

    /** Lit by a tap: it goes out by itself after [TOUCH_LIT_MS]. */
    private var dim: Runnable? = null

    /**
     * A tap on the header (T-156; Max, 2026-10-13: "When we tap the header, highlight with the back caret like we do
     * elsewhere to confirm going back"): out, it lights as Back ([light], as the wheel's tick past the top does); lit, it
     * goes back ([back]). Lit by a tap and left, it goes out by itself ([out]).
     */
    fun onBackTouched(light: () -> Unit = { show() }, out: () -> Unit = { hide() }, back: () -> Unit) {
        val off = Runnable { if (shown) out() }
        dim = off
        val tap = OnClickListener {
            if (shown) {
                removeCallbacks(off)
                back()
            } else {
                light()
                postDelayed(off, TOUCH_LIT_MS)
            }
        }
        chip.setOnClickListener(tap)
        setOnClickListener(tap)
    }

    companion object {
        /** How long a header lit by a tap stays lit for the second. */
        const val TOUCH_LIT_MS = 3000L
        private const val PAD = 10f
        /** How far left of the page's margin to place a header (its chip's padding). */
        const val INSET = PAD
    }
}

internal fun roundRect(color: Int, radius: Float, stroke: Int = 0, strokeColor: Int = Color.TRANSPARENT) = GradientDrawable().apply {
    cornerRadius = radius
    setColor(color)
    if (stroke > 0) setStroke(stroke, strokeColor)
}
