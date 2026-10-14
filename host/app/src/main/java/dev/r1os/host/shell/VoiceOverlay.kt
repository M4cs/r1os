package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.Shader
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import dev.r1os.host.net.Agent
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text

/**
 * Push-to-talk (ADR-0009, docs/specs/experience.md §2.5; ported from shell/src/voice.tsx): the
 * agent the words will go to, breathing with your voice, and your words as they're recognised.
 * r1d does the listening; the shell feeds this its `voice` and `voice_level` events.
 *
 * Over a page or an app you're watching (the R1's browser, the app window: T-138), it's [small]: the
 * agent in the bottom corner with your words beside it, and the page or app live and touchable behind.
 */
@SuppressLint("ViewConstructor")
class VoiceOverlay(context: Context) : FrameLayout(context) {
    enum class Phase { LISTENING, SENDING, ERROR }

    private val ring = Ring(context)
    private val character = CharacterView(context, context.dpi(92f))
    private val words = context.text(28f, 650).keepCase().apply {
        gravity = Gravity.CENTER
        letterSpacing = -0.01f
        setLineSpacing(0f, 1.18f)
    }
    private val scroll = object : ScrollView(context) {
        // Centred while it fits; once it overflows it scrolls, newest words in view.
        override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
            super.onLayout(changed, l, t, r, b)
            val child = getChildAt(0) ?: return
            val pad = ((height - child.height) / 2).coerceAtLeast(0)
            child.translationY = pad.toFloat()
            if (child.height > height) scrollTo(0, child.height - height)
        }
    }
    private val bars = List(5) { View(context).apply { background = roundRect(Theme.FG, context.dp(3f)) } }
    private val full = LinearLayout(context)

    // Small (T-138): the same agent and words, in a corner.
    private val miniRing = Ring(context)
    private val miniCharacter = CharacterView(context, context.dpi(50f))
    private val miniWords = context.text(15f, 600).keepCase().apply { setLineSpacing(0f, 1.1f) }
    private val miniBubble = FrameLayout(context).apply {
        setPadding(context.dpi(12f), context.dpi(8f), context.dpi(12f), context.dpi(9f))
        // Opaque, with a hairline: it sits on any page or app.
        background = roundRect(Theme.CARD, context.dp(16f), context.dpi(1f), Theme.LINE)
        addView(miniWords)
    }
    private val mini = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.BOTTOM or Gravity.END
        clipChildren = false
        visibility = GONE
    }

    /** In a corner, over what you're watching (T-138), rather than over the whole screen. */
    var small = false
        private set
    private var seeThrough = false
    private val hint = context.text(13f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER }
    private val barRow = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER }

    /** Docked (T-156): what drops what was heard, and (a talk started by touch) what sends it. */
    var onCancel: (() -> Unit)? = null
    var onSend: (() -> Unit)? = null
    private var showCancel = false
    private var showSend = false
    private fun pill(label: String, fill: Int, ink: Int, run: () -> Unit) = context.text(17f, 700, ink).apply {
        text = label
        gravity = Gravity.CENTER
        setPadding(context.dpi(22f), context.dpi(10f), context.dpi(22f), context.dpi(10f))
        background = roundRect(fill, context.dp(22f))
        setOnClickListener { run() }
    }
    private val cancelPill = pill("cancel", Theme.CARD, Theme.FG) { onCancel?.invoke() }
    private val sendPill = pill("send", Theme.accent, Theme.BG) { onSend?.invoke() }
    private val touchRow = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER
        visibility = GONE
        addView(cancelPill)
        addView(sendPill, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = context.dpi(12f) })
    }

    /** Docked (T-156): cancel on the overlay while it listens, and send for a talk started by touch. */
    fun touchControls(cancel: Boolean, send: Boolean) {
        showCancel = cancel
        showSend = send
        update()
    }

    var phase: Phase? = null
        private set
    private var accent = Theme.accent
    private var level = 0f
    private var released = false
    private var agentName = ""
    private var placeholder = false
    private val breathe = android.animation.ObjectAnimator.ofFloat(words, View.ALPHA, 1f, 0.45f, 1f).apply {
        duration = 1600
        repeatCount = android.animation.ValueAnimator.INFINITE
        interpolator = Theme.EASE_IN_OUT
    }

    init {
        setBackgroundColor(Theme.BG)
        isClickable = true // swallow touches to what's underneath
        visibility = GONE
        val col = full.apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(context.dpi(22f), context.dpi(34f), context.dpi(22f), context.dpi(18f))
        }
        val agent = FrameLayout(context).apply {
            clipChildren = false
            addView(ring, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
            addView(character, LayoutParams(context.dpi(92f), context.dpi(92f), Gravity.CENTER))
        }
        col.addView(agent, LinearLayout.LayoutParams(context.dpi(128f), context.dpi(128f)))
        scroll.isVerticalScrollBarEnabled = false
        scroll.overScrollMode = OVER_SCROLL_NEVER
        scroll.addView(words, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        col.addView(scroll, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f).apply { topMargin = context.dpi(18f); bottomMargin = context.dpi(18f) })
        bars.forEachIndexed { i, b ->
            barRow.addView(b, LinearLayout.LayoutParams(context.dpi(5f), context.dpi(22f)).apply { if (i > 0) leftMargin = context.dpi(5f) })
        }
        val foot = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            minimumHeight = context.dpi(46f)
            addView(barRow, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, context.dpi(22f)).apply { bottomMargin = context.dpi(10f) })
            addView(hint)
            addView(touchRow, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(12f) })
        }
        col.addView(foot, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        addView(col, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        mini.addView(miniBubble, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            bottomMargin = context.dpi(14f)
        })
        mini.addView(FrameLayout(context).apply {
            clipChildren = false
            addView(miniRing, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
            addView(miniCharacter, LayoutParams(context.dpi(50f), context.dpi(50f), Gravity.CENTER))
        }, LinearLayout.LayoutParams(context.dpi(78f), context.dpi(78f)))
        miniWords.maxWidth = context.dpi(250f)
        addView(mini, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or Gravity.END).apply {
            setMargins(context.dpi(10f), 0, context.dpi(4f), context.dpi(4f))
        })
    }

    /** Full screen, or [small] in a corner: what shows, and whether what's behind stays touchable. */
    private fun place(small: Boolean) {
        this.small = small
        full.visibility = if (small) GONE else VISIBLE
        mini.visibility = if (small) VISIBLE else GONE
        isClickable = !small // full screen, it swallows touches to what's underneath
        setBackgroundColor(when {
            small -> android.graphics.Color.TRANSPARENT
            seeThrough -> Theme.withAlpha(Theme.BG, 0.28f)
            else -> Theme.BG
        })
    }

    /** The agent showing now: the big one, or the small one in the corner. */
    private val face: CharacterView get() = if (small) miniCharacter else character

    /**
     * The camera is open behind it (System One's "what am I looking at", ADR-0032): the black goes
     * see-through, so you see what you're asking about.
     */
    fun seeThrough(on: Boolean) {
        seeThrough = on
        place(small)
    }

    /** Whose face it shows: the agent the words will go to. */
    fun setAgent(a: Agent) {
        agentName = a.name
        look = a
        accent = Theme.colorOf(a.color)
        ring.color = accent
        miniRing.color = accent
        bars.forEach { (it.background as android.graphics.drawable.GradientDrawable).setColor(accent) }
        val mood = if (phase == Phase.ERROR) "error" else "listening"
        character.set(a.form, a.color, a.finish, mood, eyes = a.eyes, wear = a.wear)
        miniCharacter.set(a.form, a.color, a.finish, mood, eyes = a.eyes, wear = a.wear)
    }

    private var look: Agent = Agent.DEFAULT

    /** A hold started: fades in, listening; [small] in a corner over a page or an app you're watching (T-138). */
    fun listen(small: Boolean = false) {
        animate().cancel()
        released = false
        val fresh = visibility != VISIBLE || phase != Phase.LISTENING
        if (fresh || small != this.small) {
            character.setActive(false)
            miniCharacter.setActive(false)
            place(small)
        }
        phase = Phase.LISTENING
        level = 0f
        face.set(look.form, look.color, look.finish, "listening", eyes = look.eyes, wear = look.wear)
        setText("")
        if (fresh) {
            Sounds.play(Sounds.S.LISTEN)
            visibility = VISIBLE
            alpha = 0f
            translationY = 0f
            animate().alpha(1f).setDuration(180).setInterpolator(Theme.EASE_OUT).start()
        }
        face.setActive(true)
        update()
    }

    fun partial(text: String) {
        if (phase != Phase.LISTENING) listen()
        setText(text)
    }

    fun level(v: Float) {
        if (phase != Phase.LISTENING) return
        level = v.coerceIn(0f, 1f)
        updateLevel()
    }

    /** The button is up: r1d is catching the tail of what you said. */
    fun released() {
        if (phase != Phase.LISTENING) return
        released = true
        update()
    }

    /** The words went: shows them in the accent, then lifts away. */
    fun sending(text: String) {
        Sounds.play(Sounds.S.SEND)
        phase = Phase.SENDING
        setText(text)
        words.setTextColor(accent)
        miniWords.setTextColor(accent)
        update()
        visibility = VISIBLE
        animate().cancel()
        alpha = 1f
        animate().alpha(0f).translationY(-context.dp(18f)).setStartDelay(290).setDuration(360).setInterpolator(Theme.EASE_IN_OUT)
            .withEndAction { if (phase == Phase.SENDING) hideNow() }.start()
    }

    fun error(text: String, ms: Long) {
        Sounds.play(Sounds.S.ERROR)
        phase = Phase.ERROR
        visibility = VISIBLE
        animate().cancel()
        alpha = 1f
        translationY = 0f
        setText(text.lowercase()) // an error, not something said
        words.setTextColor(Theme.BAD_TEXT)
        miniWords.setTextColor(Theme.BAD_TEXT)
        words.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 22f)
        face.set(look.form, look.color, look.finish, "error", eyes = look.eyes, wear = look.wear)
        update()
        removeCallbacks(hideError)
        postDelayed(hideError, ms)
    }

    private val hideError = Runnable { if (phase == Phase.ERROR) hide() }

    fun hide() {
        if (visibility != VISIBLE) return
        animate().cancel()
        animate().alpha(0f).setStartDelay(0).setDuration(160).withEndAction { hideNow() }.start()
    }

    private fun hideNow() {
        phase = null
        visibility = GONE
        translationY = 0f
        character.setActive(false)
        miniCharacter.setActive(false)
        breathe.cancel()
        words.alpha = 1f
    }

    private fun setText(t: String) {
        placeholder = t.isEmpty() && phase == Phase.LISTENING
        words.text = t.ifEmpty { if (phase == Phase.LISTENING) "listening…" else "" }
        val size = when {
            t.length > 160 -> 19f
            t.length > 70 -> 23f
            else -> 28f
        }
        words.setTextSize(TypedValue.COMPLEX_UNIT_DIP, size)
        words.typeface = Theme.font(if (placeholder) 500 else 650)
        words.setTextColor(if (placeholder) Theme.MUTED else Theme.FG)
        // Small: the newest words, a few lines' worth.
        miniWords.text = words.text.let { if (it.length > MINI_CHARS) "…" + it.takeLast(MINI_CHARS).trimStart() else it }
        miniWords.typeface = Theme.font(if (placeholder) 500 else 600)
        miniWords.setTextColor(if (placeholder) Theme.MUTED else Theme.FG)
        if (placeholder) { if (!breathe.isStarted) breathe.start() } else { breathe.cancel(); words.alpha = 1f }
    }

    private fun update() {
        val listening = phase == Phase.LISTENING
        barRow.visibility = if (listening) VISIBLE else GONE
        hint.text = when (phase) {
            Phase.LISTENING -> when {
                released -> "Finishing…"
                showSend -> "Talk, then send"
                else -> "Let go to send · turn the wheel to cancel"
            }
            Phase.SENDING -> "Sent to $agentName"
            else -> ""
        }
        touchRow.visibility = if (listening && !released && showCancel && !small) VISIBLE else GONE
        sendPill.visibility = if (showSend) VISIBLE else GONE
        if (!listening) level = 0f
        updateLevel()
    }

    private fun updateLevel() {
        val lvl = level
        (if (small) miniRing else ring).animate().scaleX(0.78f + lvl * 0.5f).scaleY(0.78f + lvl * 0.5f).alpha(0.35f + lvl * 0.65f).setDuration(90).setInterpolator(null).start()
        if (small) return
        BARS.forEachIndexed { i, k ->
            bars[i].animate().scaleY(0.15f + minOf(1f, lvl * k * 1.3f)).setDuration(90).setInterpolator(null).start()
        }
    }

    /** A soft disc of the accent behind the agent: radial, 38% at the centre to clear at 68%. */
    private class Ring(context: Context) : View(context) {
        private val p = Paint(Paint.ANTI_ALIAS_FLAG)
        var color = Theme.accent
            set(v) { field = v; p.shader = null; invalidate() }

        init {
            scaleX = 0.78f; scaleY = 0.78f; alpha = 0.35f
        }

        override fun onDraw(c: Canvas) {
            val r = width / 2f
            if (p.shader == null && r > 0) {
                p.shader = RadialGradient(r, r, r, intArrayOf(Theme.withAlpha(color, 0.38f), Theme.withAlpha(color, 0f)), floatArrayOf(0f, 0.68f), Shader.TileMode.CLAMP)
            }
            c.drawCircle(r, r, r, p)
        }
    }

    private companion object {
        val BARS = floatArrayOf(0.55f, 0.8f, 1f, 0.8f, 0.55f)
        /** How much of what you're saying the small one shows. */
        const val MINI_CHARS = 90
    }
}
