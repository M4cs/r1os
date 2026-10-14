package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text

/**
 * An app's reminder (ADR-0024): r1d woke the display, and this drops in at the top, over whatever is on
 * screen (the glance screen, usually): the app's icon in its accent, the title big, the body under it, and
 * one soft note. It goes by itself after [SHOW_MS] (T-123, ADR-0059), or sooner when you turn the wheel or
 * press the button, or the display sleeps. Going by itself isn't seeing it: a reminder comes back on wake,
 * and the glance screen keeps it in what's waiting. Touching it opens the app.
 *
 * It also shows an agent's reminder (ADR-0028): the agent's name, a bell in its colour, and the note.
 * Touching it, or the wheel or the button, means you've seen it ([onSeen]).
 */
@SuppressLint("ViewConstructor")
class NoticeBanner(
    context: Context,
    private val onOpen: (app: String) -> Unit,
    private val onSeen: (reminder: String) -> Unit,
    /** An Android app's notification touched: it opens (dev.r1os.host.Notifications). */
    private val onOpenNote: (key: String) -> Unit = {},
    /** A routine's note touched: its chat opens (R7). */
    private val onOpenChat: (agent: String, chat: String) -> Unit = { _, _ -> },
    /** An app's notice touched: seen (T-049), as it opens. */
    private val onSeenNotice: (id: Long) -> Unit = {},
) : LinearLayout(context) {
    private val iconView = ImageView(context)
    private val tile = FrameLayout(context).apply {
        addView(iconView, FrameLayout.LayoutParams(context.dpi(22f), context.dpi(22f), Gravity.CENTER))
    }
    private val title = context.text(19f, 700).apply { maxLines = 2; ellipsize = TextUtils.TruncateAt.END }
    private val body = context.text(16f, 500, Theme.MUTED).apply { maxLines = 4; ellipsize = TextUtils.TruncateAt.END }
    private val from = context.text(13f, 600, Theme.MUTED)
    private var app: String? = null
    /** The Android app's notification it shows (its key). */
    private var noteKey: String? = null
    /** A routine's chat it shows (agent, chat). */
    private var chat: Pair<String, String>? = null
    /** The reminder it shows (its id), rather than an app's notice. */
    var reminder: String? = null
        private set
    /** The app's notice it shows (its id in r1d, T-049): it comes back on wake until it's seen. */
    var appNotice: Long? = null
        private set
    var shown = false
        private set
    /** It's been up long enough (T-123). */
    private val autoHide = Runnable { hide() }

    init {
        orientation = HORIZONTAL
        gravity = Gravity.TOP
        setPadding(context.dpi(12f), context.dpi(12f), context.dpi(14f), context.dpi(13f))
        visibility = GONE
        addView(tile, LayoutParams(context.dpi(40f), context.dpi(40f)).apply { rightMargin = context.dpi(12f) })
        addView(LinearLayout(context).apply {
            orientation = VERTICAL
            addView(from)
            addView(title, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(1f) })
            addView(body, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(3f) })
        }, LayoutParams(0, LayoutParams.WRAP_CONTENT, 1f))
        setOnClickListener {
            val r = reminder
            val k = noteKey
            val c = chat
            if (c != null) {
                hide()
                onOpenChat(c.first, c.second)
            } else if (r != null) {
                hide()
                onSeen(r)
            } else if (k != null) {
                hide()
                onOpenNote(k)
            } else {
                appNotice?.let(onSeenNotice)
                appNotice = null
                app?.let(onOpen)
            }
        }
    }

    fun show(ev: Ev) {
        val accent = Theme.COLORS[ev.str("accent").orEmpty()] ?: Theme.accent
        app = ev.str("app")
        appNotice = ev.json.optLong("id").takeIf { ev.json.has("id") && it > 0 }
        noteKey = null
        reminder = null
        chat = null
        iconView.setImageDrawable(Icons.drawable(ev.str("icon") ?: "bell", accent))
        tile.background = roundRect(Theme.withAlpha(accent, 0.2f), context.dp(12f))
        from.text = ev.str("name").orEmpty()
        title.text = ev.str("title").orEmpty()
        val b = ev.str("body").orEmpty()
        body.text = b
        body.visibility = if (b.isBlank()) GONE else VISIBLE
        pop(accent)
    }

    /** An Android app's notification: its own icon, its name, the title and the text. Touching it opens it. */
    fun showNote(n: dev.r1os.host.Notifications.Item) {
        app = null
        reminder = null
        appNotice = null
        chat = null
        noteKey = n.key
        iconView.setImageDrawable(n.icon())
        tile.background = null
        from.text = n.app
        title.text = n.title.ifEmpty { n.app }
        body.text = n.text
        body.visibility = if (n.text.isBlank()) GONE else VISIBLE
        title.transformationMethod = null
        body.transformationMethod = null
        pop(Theme.MUTED)
    }

    /** Agent `a`'s reminder `id`: `text`. */
    fun showReminder(id: String, text: String, a: dev.r1os.host.net.Agent) {
        if (shown && reminder == id) return
        val accent = Theme.colorOf(a.color)
        app = null
        noteKey = null
        chat = null
        reminder = id
        appNotice = null
        iconView.setImageDrawable(Icons.drawable("bell-ringing", accent))
        tile.background = roundRect(Theme.withAlpha(accent, 0.2f), context.dp(12f))
        from.text = a.name
        title.text = text
        body.visibility = GONE
        pop(accent)
    }

    /** A routine ran (R7): the agent's name, the routine's title and its reply's first words. Touching it opens the chat. */
    fun showRoutine(title: String, text: String, a: dev.r1os.host.net.Agent, chatId: String) {
        val accent = Theme.colorOf(a.color)
        app = null
        noteKey = null
        reminder = null
        appNotice = null
        chat = a.id to chatId
        iconView.setImageDrawable(Icons.drawable("repeat", accent))
        tile.background = roundRect(Theme.withAlpha(accent, 0.2f), context.dp(12f))
        from.text = a.name
        this.title.text = title
        body.text = text
        body.visibility = if (text.isBlank()) GONE else VISIBLE
        pop(accent)
    }

    /** A pomodoro moved on (ADR-0050): what's next ("break · 5 min") and where it is, in the colour of the agent that set it. */
    fun showTimer(what: String, detail: String, a: dev.r1os.host.net.Agent) {
        val accent = Theme.colorOf(a.color)
        app = null
        noteKey = null
        reminder = null
        appNotice = null
        chat = null
        iconView.setImageDrawable(Icons.drawable("timer", accent))
        tile.background = roundRect(Theme.withAlpha(accent, 0.2f), context.dp(12f))
        from.text = "pomodoro"
        title.text = what
        body.text = detail
        body.visibility = if (detail.isBlank()) GONE else VISIBLE
        pop(accent)
    }

    private fun pop(accent: Int) {
        // Opaque (T-096): the surface's few percent of see-through let the glance screen's page, or a chat's header,
        // read through it.
        background = roundRect(Theme.withAlpha(Theme.SURFACE, 1f), context.dp(20f), context.dpi(1.5f), Theme.withAlpha(accent, 0.55f))
        Sounds.play(Sounds.S.NOTE)
        shown = true
        visibility = VISIBLE
        animate().cancel()
        // In from above the top edge, fading in as it comes (Max, 2026-10-12: "slide in and out while also fading").
        alpha = 0f
        translationY = -offscreen()
        animate().alpha(1f).translationY(0f).setDuration(380).setInterpolator(Theme.POP).start()
        removeCallbacks(autoHide)
        postDelayed(autoHide, SHOW_MS)
    }

    fun hide(animate: Boolean = true) {
        removeCallbacks(autoHide)
        if (!shown) return
        shown = false
        animate().cancel()
        if (!animate) {
            visibility = GONE
            return
        }
        // Out the way it came, past the top edge, fading as it goes.
        animate().alpha(0f).translationY(-offscreen()).setDuration(280).setInterpolator(Theme.EASE_IN_OUT)
            .withEndAction { visibility = View.GONE }.start()
    }

    /** How far up it goes to be off the screen: its height and its margin (a guess before it's first laid out). */
    private fun offscreen(): Float {
        val top = (layoutParams as? MarginLayoutParams)?.topMargin ?: 0
        return if (height > 0) (height + top).toFloat() else context.dp(120f)
    }

    private companion object {
        /** How long a banner stays by itself (Max: "auto dismiss after a short period"). */
        const val SHOW_MS = 6000L
    }
}
