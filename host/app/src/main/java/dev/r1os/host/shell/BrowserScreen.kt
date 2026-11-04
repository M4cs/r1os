package dev.r1os.host.shell

import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.AgentBrowser
import dev.r1os.host.AgentPointer
import dev.r1os.host.net.Ev
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text

/**
 * The R1's browser on screen (ADR-0054): the page an agent is working in, as it works, under who's browsing and
 * where. Touch works on the page itself; the wheel scrolls it; a double click leaves (the agent carries on behind).
 *
 * When the agent asks you to do something yourself ("Log in to Amazon": `browser_ask_user`), a banner says so with
 * **done** and **not now**; a click is done too.
 *
 * In that agent's chat the browser shows small, as a live preview in the corner ([BrowserPreview], T-090); a touch
 * there brings this screen up, and asked to take over it comes up by itself. What the agent says in that chat shows
 * over the page in a bubble (Max, 2026-10-11: "show the reply from the agent in a little overlay bubble so we can see
 * what they said"); touching it goes back to the chat.
 *
 * Your own browser with nothing open says what to do, in place of a blank white page (T-203).
 */
class BrowserScreen(shell: Shell) : Screen(shell) {
    private val who = context.text(15f, 700, Theme.FG)
    private val where = context.text(13f, 600, Theme.MUTED).keepCase().apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.MIDDLE }
    private val askText = context.text(16f, 700, Theme.FG).keepCase()
    private val frame = FrameLayout(context)
    /** The agent's colour, for its pointer. */
    private var tint = Theme.accent
    /** The agent's pointer over the page (T-137), in the page's own pixels; not over your own browser. */
    private val pointer = PointerView(context, AgentPointer.browser, small = false, tint = { tint }) { x, y, out -> out.set(x, y); !AgentBrowser.mine }
    private var off: (() -> Unit)? = null
    /** The page while it's here. */
    private var page: android.webkit.WebView? = null
    /** The agent's words while you watch: what it says in the chat it's browsing for, as it says it. */
    private val replyName = context.text(12f, 700, Theme.MUTED)
    private val replyText = context.text(15f, 600, Theme.FG).keepCase().apply { maxLines = 6; ellipsize = android.text.TextUtils.TruncateAt.END }
    private val stream = StringBuilder()
    private val reply = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(14f), context.dpi(9f), context.dpi(14f), context.dpi(11f))
        // Opaque, with a hairline: it sits on any page.
        background = roundRect(Theme.CARD, context.dp(18f), context.dpi(1f), Theme.LINE)
        elevation = context.dp(8f)
        visibility = View.GONE
        addView(replyName)
        addView(replyText)
        setOnClickListener {
            Sounds.play(Sounds.S.OPEN)
            AgentBrowser.chat?.let { shell.toChat(AgentBrowser.agent, it) }
        }
    }

    /** Over the page while there isn't one: a WebView with nothing loaded is white. */
    private val empty = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER
        setBackgroundColor(Theme.BG)
        setPadding(context.dpi(28f), 0, context.dpi(28f), context.dpi(48f))
        visibility = View.GONE
        addView(context.text(20f, 700).apply { text = "nothing open"; gravity = Gravity.CENTER })
        addView(context.text(15f, 500, Theme.MUTED).apply {
            text = "hold the button and say “go to wikipedia.org”"
            gravity = Gravity.CENTER
            setPadding(0, context.dpi(8f), 0, 0)
        })
    }

    private fun button(label: String, accent: Boolean, onTap: () -> Unit) = context.text(15f, 700, if (accent) Theme.BG else Theme.FG).apply {
        text = label
        gravity = Gravity.CENTER
        setPadding(context.dpi(16f), context.dpi(9f), context.dpi(16f), context.dpi(9f))
        background = roundRect(if (accent) Theme.accent else Theme.FOCUS, context.dp(14f))
        setOnClickListener { Sounds.play(Sounds.S.OPEN); onTap() }
    }

    private val banner = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(12f))
        background = roundRect(Theme.SURFACE, context.dp(18f))
        visibility = View.GONE
        addView(askText)
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(button("done", true) { AgentBrowser.answerAsk(true) })
            addView(button("not now", false) { AgentBrowser.answerAsk(false) }, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = context.dpi(8f) })
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
    }

    override val view: View = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(16f), context.dpi(10f), context.dpi(16f), context.dpi(8f))
            addView(who)
            addView(where)
        })
        addView(banner, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(10f), 0, context.dpi(10f), context.dpi(8f))
        })
        addView(frame, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        // Over the page (put in under these when it shows), under the reply bubble.
        frame.addView(pointer, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        frame.addView(empty, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        frame.addView(reply, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM).apply {
            setMargins(context.dpi(10f), 0, context.dpi(10f), context.dpi(10f))
        })
    }

    /** The chat the page is for (not your own browser's): the shell doesn't flash a note for a reply shown here. */
    val chat: String? get() = if (AgentBrowser.mine) null else AgentBrowser.chat

    override val agentId: String? get() = if (AgentBrowser.mine) null else AgentBrowser.agent

    override fun onShow() {
        AgentBrowser.watching = true
        val w = AgentBrowser.view(context)
        (w.parent as? android.view.ViewGroup)?.removeView(w)
        w.scaleX = 1f
        w.scaleY = 1f
        page = w
        // Under the reply bubble.
        frame.addView(w, 0, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        off?.invoke()
        off = AgentBrowser.onChange { render() }
        render()
    }

    override fun onHide() {
        off?.invoke()
        off = null
        // Gone already: closed, or back in the chat's preview (this runs again when the screen's destroyed).
        val w = page ?: return
        page = null
        if (w.parent !== frame) return
        AgentBrowser.watching = false
        AgentBrowser.park()
    }

    override fun onDestroy() {
        // Leaving while asked: not now.
        if (AgentBrowser.asking != null) AgentBrowser.answerAsk(false)
        onHide()
    }

    private fun render() {
        val a = shell.agent(AgentBrowser.agent)
        val name = a?.name?.lowercase() ?: "your agent"
        val asking = AgentBrowser.asking
        who.text = when {
            stopping.armed == "turn" && asking == null -> "click again to stop $name"
            asking != null -> "$name needs you"
            AgentBrowser.mine -> "your browser"
            AgentBrowser.open -> "$name is browsing"
            else -> "the browser's closed"
        }
        who.setTextColor(if (AgentBrowser.mine) Theme.FG else a?.let { Theme.colorOf(it.color) } ?: Theme.FG)
        tint = a?.let { Theme.colorOf(it.color) } ?: Theme.accent
        val nothing = AgentBrowser.mine && asking == null && AgentBrowser.note == null && (AgentBrowser.url.isEmpty() || AgentBrowser.url == "about:blank")
        empty.visibility = if (nothing) View.VISIBLE else View.GONE
        where.visibility = if (nothing) View.GONE else View.VISIBLE
        where.text = AgentBrowser.note ?: AgentBrowser.url.substringAfter("://").substringBefore("/").ifEmpty { "…" }
        banner.visibility = if (asking != null) View.VISIBLE else View.GONE
        askText.text = asking.orEmpty()
        // The agent closed it while you watched.
        if (!AgentBrowser.open && asking == null && shell.top() === this) shell.back()
    }

    /** The chat the page is for: its reply in the bubble as it streams (narration before a tool goes, as in the chat). */
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
        val a = shell.agent(AgentBrowser.agent)
        yours = you
        replyName.text = if (you) "you" else a?.name?.lowercase() ?: "your agent"
        replyName.setTextColor(if (you) Theme.MUTED else a?.let { Theme.colorOf(it.color) } ?: Theme.MUTED)
        replyText.setTextColor(if (you) Theme.MUTED else Theme.FG)
        replyText.text = ChatScreen.plain(text).trim()
        reply.visibility = if (replyText.text.isEmpty()) View.GONE else View.VISIBLE
    }

    /** The agent using the page is working on it (its turn in this chat). */
    private val working: Boolean
        get() = chat != null && shell.agent(AgentBrowser.agent)?.let { it.state in HomeScreen.BUSY && it.chat == chat } == true

    private val stopping = Confirm { render() }

    override fun onEvent(ev: Ev): Boolean {
        // While it works: a click, then another, stops it (T-013), as in its chat.
        if (ev.isTap && AgentBrowser.asking == null && working) {
            stopping.tap("turn") {
                Sounds.play(Sounds.S.CLOSE)
                chat?.let { dev.r1os.host.net.R1d.post("/api/chats/$it/stop") }
            }
            return true
        }
        if (ev.type == "wheel") {
            AgentBrowser.scrollBy(ev.int("delta"))
            return true
        }
        if (ev.isTap && AgentBrowser.asking != null) {
            AgentBrowser.answerAsk(true)
            return true
        }
        return ev.isTap
    }
}
