package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.AgentApps
import dev.r1os.host.AgentPointer
import dev.r1os.host.net.Ev
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text

/**
 * The app window on screen (ADR-0061, T-113): the Android app an agent is using, live, under who's using it. Your
 * touches go into the app (its display, not the R1's own); the wheel scrolls it; a double click leaves (the agent
 * carries on behind).
 *
 * When the agent hands you the app ("Sign in to Uber": `app_ask_user`), a banner says so with **done** and **not now**;
 * a click is done too. In that agent's chat the app shows small, as a live preview ([AppPreview]); a touch there brings
 * this screen up, and asked to take over it comes up by itself.
 *
 * **let me use it** takes the app for yourself (Q63, T-139): it moves onto the R1's own screen as it is, and the window
 * closes; the agent's next step is told.
 *
 * What the agent says in that chat shows over the app in a bubble, as over the browser (T-090); touching it goes to the
 * chat. Push-to-talk here is small in a corner, and your words go to that chat while you stay on the app (T-138).
 */
class AppWindowScreen(shell: Shell) : Screen(shell) {
    private val who = context.text(15f, 700, Theme.FG)
    private val where = context.text(13f, 600, Theme.MUTED).keepCase().apply { maxLines = 1 }
    private val askText = context.text(16f, 700, Theme.FG).keepCase()
    private var offChange: (() -> Unit)? = null
    private var offFrame: (() -> Unit)? = null
    private val frame = FrameLayout(context)
    /** The agent's colour, for its pointer. */
    private var tint = Theme.accent
    /** The agent's pointer over the app (T-137): the app's display pixels, placed as the picture fits them. */
    private val pointer = PointerView(context, AgentPointer.app, small = false, tint = { tint }) { x, y, out ->
        val size = AgentApps.size
        val v = picture
        if (size.x == 0 || v.width == 0) false else {
            val s = minOf(v.width / size.x.toFloat(), v.height / size.y.toFloat())
            out.set((v.width - size.x * s) / 2f + x * s, (v.height - size.y * s) / 2f + y * s)
            true
        }
    }

    /** The agent's words while you watch: what it says in the chat using the app, as it says it. */
    private val replyName = context.text(12f, 700, Theme.MUTED)
    private val replyText = context.text(15f, 600, Theme.FG).keepCase().apply { maxLines = 6; ellipsize = android.text.TextUtils.TruncateAt.END }
    private val stream = StringBuilder()
    private val reply = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(14f), context.dpi(9f), context.dpi(14f), context.dpi(11f))
        // Opaque, with a hairline: it sits on any app.
        background = roundRect(Theme.CARD, context.dp(18f), context.dpi(1f), Theme.LINE)
        elevation = context.dp(8f)
        visibility = View.GONE
        addView(replyName)
        addView(replyText)
        setOnClickListener {
            Sounds.play(Sounds.S.OPEN)
            AgentApps.chat?.let { shell.toChat(AgentApps.agent, it) }
        }
    }

    /** The app's picture. Touches on it go into the app, at the same place on its display. */
    @SuppressLint("ClickableViewAccessibility")
    private val picture = ImageView(context).apply {
        scaleType = ImageView.ScaleType.FIT_CENTER
        setBackgroundColor(android.graphics.Color.BLACK)
        setOnTouchListener { v, ev ->
            val size = AgentApps.size
            if (size.x == 0 || !AgentApps.open) return@setOnTouchListener false
            val s = minOf(v.width / size.x.toFloat(), v.height / size.y.toFloat())
            val ox = (v.width - size.x * s) / 2f
            val oy = (v.height - size.y * s) / 2f
            AgentApps.yourTouch(ev, ((ev.x - ox) / s).coerceIn(0f, size.x - 1f), ((ev.y - oy) / s).coerceIn(0f, size.y - 1f))
            if (ev.actionMasked == MotionEvent.ACTION_UP) v.performClick()
            true
        }
    }

    private fun button(label: String, accent: Boolean, onTap: () -> Unit) = context.text(15f, 700, if (accent) Theme.BG else Theme.FG).apply {
        text = label
        gravity = Gravity.CENTER
        setPadding(context.dpi(16f), context.dpi(9f), context.dpi(16f), context.dpi(9f))
        background = roundRect(if (accent) Theme.accent else Theme.FOCUS, context.dp(14f))
        setOnClickListener { Sounds.play(Sounds.S.OPEN); onTap() }
    }

    /** Take the app for yourself: on the R1's own screen, where it was. */
    private val take = button("let me use it", false) { AgentApps.handOver() }

    private val banner = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(12f))
        background = roundRect(Theme.SURFACE, context.dp(18f))
        visibility = View.GONE
        addView(askText)
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(button("done", true) { AgentApps.answerAsk(true) })
            addView(button("not now", false) { AgentApps.answerAsk(false) }, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = context.dpi(8f) })
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
    }

    override val view: View = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(16f), context.dpi(10f), context.dpi(10f), context.dpi(8f))
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(who)
                addView(where)
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            addView(take)
        })
        addView(banner, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(10f), 0, context.dpi(10f), context.dpi(8f))
        })
        addView(frame.apply {
            addView(picture)
            addView(pointer, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        frame.addView(reply, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM).apply {
            setMargins(context.dpi(10f), 0, context.dpi(10f), context.dpi(10f))
        })
    }

    /** The chat using the app: the shell doesn't flash a note for a reply shown here. */
    val chat: String? get() = AgentApps.chat

    /** That chat's reply in the bubble as it streams (narration before a step goes, as in the chat). */
    override fun onAnyEvent(ev: Ev) {
        val c = chat ?: return
        if (ev.type != "chat" || ev.str("chat") != c) return
        val text = ev.str("text").orEmpty()
        when (ev.str("kind")) {
            // What you said here, until its reply comes.
            "user" -> { stream.clear(); say(text, you = true) }
            // You stopped it (T-013).
            "stopped" -> { stream.clear(); say(text.ifBlank { "stopped" }) }
            "tool" -> { stream.clear(); if (!yours) reply.visibility = View.GONE }
            "delta" -> { stream.append(text); say(stream.toString()) }
            "done" -> { stream.clear(); say(text) }
            "error" -> { stream.clear(); say(text.ifEmpty { "Something went wrong" }) }
        }
    }

    /** The bubble shows your words, not the agent's. */
    private var yours = false

    private fun say(text: String, you: Boolean = false) {
        val a = shell.agent(AgentApps.agent)
        yours = you
        replyName.text = if (you) "you" else a?.name?.lowercase() ?: "your agent"
        replyName.setTextColor(if (you) Theme.MUTED else a?.let { Theme.colorOf(it.color) } ?: Theme.MUTED)
        replyText.setTextColor(if (you) Theme.MUTED else Theme.FG)
        replyText.text = ChatScreen.plain(text).trim()
        reply.visibility = if (replyText.text.isEmpty()) View.GONE else View.VISIBLE
    }

    override val agentId: String? get() = AgentApps.agent

    override fun onShow() {
        AgentApps.watching = true
        offChange?.invoke()
        offChange = AgentApps.onChange { render() }
        offFrame?.invoke()
        offFrame = AgentApps.onFrame { show(it) }
        render()
    }

    override fun onHide() {
        offChange?.invoke()
        offChange = null
        offFrame?.invoke()
        offFrame = null
        AgentApps.watching = false
    }

    override fun onDestroy() {
        // Leaving while asked: not now.
        if (AgentApps.asking != null) AgentApps.answerAsk(false)
        onHide()
    }

    /** A frame (shared with the chat's preview, so never recycled here). */
    private fun show(bmp: Bitmap) = picture.setImageBitmap(bmp)

    private fun render() {
        val a = shell.agent(AgentApps.agent)
        val name = a?.name?.lowercase() ?: "your agent"
        val asking = AgentApps.asking
        val app = AgentApps.name?.lowercase() ?: "an app"
        who.text = when {
            stopping.armed == "turn" && asking == null -> "click again to stop $name"
            asking != null -> "$name needs you"
            AgentApps.open -> "$name is using $app"
            else -> "the app window's closed"
        }
        who.setTextColor(a?.let { Theme.colorOf(it.color) } ?: Theme.FG)
        tint = a?.let { Theme.colorOf(it.color) } ?: Theme.accent
        where.text = if (AgentApps.open) app else "…"
        banner.visibility = if (asking != null) View.VISIBLE else View.GONE
        take.visibility = if (AgentApps.open) View.VISIBLE else View.GONE
        askText.text = asking.orEmpty()
        // The agent closed it while you watched.
        if (!AgentApps.open && asking == null && shell.top() === this) shell.back()
    }

    /** The agent using the app is working (its turn in that chat). */
    private val working: Boolean
        get() = AgentApps.chat != null && shell.agent(AgentApps.agent)?.let { it.state in HomeScreen.BUSY && it.chat == AgentApps.chat } == true

    private val stopping = Confirm { render() }

    override fun onEvent(ev: Ev): Boolean {
        // While it works: a click, then another, stops it (T-013), as in its chat.
        if (ev.isTap && AgentApps.asking == null && working) {
            stopping.tap("turn") {
                Sounds.play(Sounds.S.CLOSE)
                AgentApps.chat?.let { dev.r1os.host.net.R1d.post("/api/chats/$it/stop") }
            }
            return true
        }
        if (ev.type == "wheel") {
            AgentApps.scrollBy(ev.int("delta"))
            return true
        }
        if (ev.isTap && AgentApps.asking != null) {
            AgentApps.answerAsk(true)
            return true
        }
        return ev.isTap
    }
}
