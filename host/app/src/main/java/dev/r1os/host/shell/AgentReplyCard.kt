package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import dev.r1os.host.net.Agent
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Lowercase
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text

/**
 * Talking over an open app (ADR-0025): you stay in the app. Your words go to your agent, told which app
 * is on screen, and this card at the bottom shows it working, then its reply for a few seconds, while the
 * app updates underneath ("add milk" → "Added milk."). Touching it opens the chat. Turning the wheel once
 * the reply is in puts it away.
 */
@SuppressLint("ViewConstructor")
class AgentReplyCard(context: Context, private val onOpen: () -> Unit) : LinearLayout(context) {
    private val character = CharacterView(context, context.dpi(36f))
    private val line = context.text(17f, 600).apply {
        maxLines = 4
        ellipsize = TextUtils.TruncateAt.END
        setLineSpacing(0f, 1.15f)
    }
    private val hide = Runnable { hide() }
    var shown = false
        private set
    /** The reply is in (not still working). */
    var answered = false
        private set

    init {
        orientation = HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(10f), context.dpi(10f), context.dpi(16f), context.dpi(10f))
        visibility = GONE
        addView(character, LayoutParams(context.dpi(36f), context.dpi(36f)).apply { rightMargin = context.dpi(12f) })
        addView(line, LayoutParams(0, LayoutParams.WRAP_CONTENT, 1f))
        setOnClickListener { onOpen() }
    }

    /** The agent is on it (`label`: what it's doing, from its tool). */
    fun working(a: Agent, label: String = "On it…") {
        removeCallbacks(hide)
        answered = false
        show(a, "thinking")
        line.transformationMethod = Lowercase
        line.setTextColor(Theme.MUTED)
        line.text = label
    }

    /** Its reply, as said (it keeps its case), for a few seconds: longer for more words. */
    fun reply(a: Agent, text: String, error: Boolean = false) {
        answered = true
        show(a, if (error) "error" else "speaking")
        line.transformationMethod = null
        line.setTextColor(if (error) Theme.BAD else Theme.FG)
        line.text = ChatScreen.plain(text.trim()).ifEmpty { "Done." }
        Sounds.play(Sounds.S.NOTE)
        removeCallbacks(hide)
        postDelayed(hide, (3000L + 45L * text.length).coerceAtMost(12_000L))
    }

    private fun show(a: Agent, state: String) {
        val accent = Theme.colorOf(a.color)
        character.set(a.form, a.color, a.finish, state, eyes = a.eyes, wear = a.wear)
        character.setActive(true)
        background = roundRect(Theme.SURFACE, context.dp(20f), context.dpi(1.5f), Theme.withAlpha(accent, 0.55f))
        if (shown) return
        shown = true
        visibility = VISIBLE
        animate().cancel()
        alpha = 0f
        translationY = context.dp(24f)
        animate().alpha(1f).translationY(0f).setDuration(280).setInterpolator(Theme.POP).start()
    }

    fun hide(animate: Boolean = true) {
        removeCallbacks(hide)
        if (!shown) return
        shown = false
        character.setActive(false)
        animate().cancel()
        if (!animate) {
            visibility = GONE
            return
        }
        animate().alpha(0f).translationY(context.dp(16f)).setDuration(200).withEndAction { visibility = View.GONE }.start()
    }
}
