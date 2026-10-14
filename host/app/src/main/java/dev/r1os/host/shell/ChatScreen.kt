package dev.r1os.host.shell

import android.animation.ObjectAnimator
import android.animation.PropertyValuesHolder
import android.animation.ValueAnimator
import android.graphics.Color
import android.graphics.DashPathEffect
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.ShapeDrawable
import android.graphics.drawable.shapes.RoundRectShape
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import dev.r1os.host.net.Agent
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject

/**
 * A conversation with one of your agents (docs/specs/experience.md §2.3; ported from
 * shell/src/chat.tsx). You talk by holding the side button, or turn the R1 on its side to type ([typeOn],
 * ADR-0059): a box and r1os's keyboard come up, and upright again they go. Replies stream in,
 * and a note marks where r1d tidied a long chat into a fresh session. The wheel scrolls; one tick past
 * the top brings Back (ADR-0014).
 *
 * A build the agent starts is a live card **pinned at the bottom**, under the conversation, so a long
 * reply can't push it out of view. When it's done it joins the conversation there, at the moment it
 * finished (where r1d stores it too), and a click opens the newest app it made (Max, 2026-10-09).
 *
 * Messages between agents (ADR-0012) are cards in the other agent's colour: a question it sent, another
 * agent's answer (then the agent passes it on, in a turn r1d starts), or a note. A thread, an agent's
 * exchanges with one other agent, opens here too: its header names both.
 */
class ChatScreen(
    shell: Shell,
    private val agent: Agent,
    id: String?,
    private val say: String?,
    private val sayApp: String?,
    /** Photos to send with `say` (ADR-0031). */
    private val sayImages: List<String> = emptyList(),
) : Screen(shell) {
    /** The chat's id: null until a new chat sends its first words. */
    var chatId: String? = id
        private set

    override val agentId get() = agent.id

    private val accent = Theme.colorOf(agent.color)
    /** A bubble's widest: most of the column, with room for its opposite margin. */
    private val bubbleMax = (context.resources.displayMetrics.widthPixels - 2 * context.dp(MARGIN)) * 0.84f
    private val character = CharacterView(context, context.dpi(44f))
    private val nameView = context.text(28f, 700).apply { text = agent.name; letterSpacing = -0.01f; maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END }
    private val header: BackHeader
    override val backHeader get() = header
    private val list = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(0, context.dpi(10f), 0, context.dpi(12f))
    }
    /** The conversation scrolls under the header: it's clipped there and fades as it goes. */
    private val scroll = ScrollView(context).apply {
        isVerticalScrollBarEnabled = false
        overScrollMode = View.OVER_SCROLL_NEVER
        isFocusable = false
        isVerticalFadingEdgeEnabled = true
        setFadingEdgeLength(context.dpi(28f))
        addView(list)
    }
    private val empty = context.text(18f, 500, Theme.MUTED).apply {
        gravity = Gravity.CENTER
        text = "Ask ${agent.name} anything, or for an app."
        setPadding(context.dpi(24f), context.dpi(110f), context.dpi(24f), 0)
        setLineSpacing(0f, 1.2f)
    }
    /** The conversation, and over its corner the R1's browser while this chat's agent uses it (T-090). */
    private val body = FrameLayout(context)
    private var preview: BrowserPreview? = null
    private var offBrowser: (() -> Unit)? = null
    /** The app window, small, while this chat's agent drives an Android app (ADR-0061). */
    private var appPreview: AppPreview? = null
    private var offApps: (() -> Unit)? = null
    private val streamBubble = bubble("assistant", "")
    private val typing = Typing()
    private val footText = context.text(15f, 600, Theme.MUTED)
    /** What the agent is doing, under the conversation: only while it works. */
    private val foot = LinearLayout(context)
    /**
     * Typing to the agent (T-089, Max: "Going landscape mode in a chat should open a keyboard and text box where you can
     * type in a prompt to send"): the shell opens it as the R1 goes on its side ([typeOn]). The keyboard's send key, or
     * a click, sends it; it stays for the next words until the R1's upright again.
     */
    private val box = android.widget.EditText(context).apply {
        hint = "type to ${agent.name.lowercase()}"
        setHintTextColor(Theme.MUTED)
        setTextColor(Theme.FG)
        textSize = 17f
        typeface = Theme.font(600)
        inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_CAP_SENTENCES or
            android.text.InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
        // Wraps to a few lines, but Enter is still the send key (no multi-line flag).
        setHorizontallyScrolling(false)
        maxLines = 3
        imeOptions = android.view.inputmethod.EditorInfo.IME_ACTION_SEND or android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI
        background = roundRect(Theme.CARD, context.dp(16f))
        setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(10f))
        visibility = View.GONE
        setOnEditorActionListener { _, action, _ ->
            if (action != android.view.inputmethod.EditorInfo.IME_ACTION_SEND) return@setOnEditorActionListener false
            sendTyped()
            true
        }
    }
    /** The box is up: the R1's on its side. */
    var typingOn = false
        private set
    override val view = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
        setPadding(0, context.dpi(10f), 0, context.dpi(12f))
    }

    private var busy = false
    private var stream = StringBuilder()
    /** The reply so far in its bubble: once a frame, however many pieces came (they each redid all its text). */
    private var streamPending = false
    private val streamShow = Runnable {
        streamPending = false
        if (stream.isEmpty()) return@Runnable
        streamBubble.text = plain(stream.toString())
        if (streamBubble.parent == null) {
            list.addView(streamBubble, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                gravity = Gravity.START; topMargin = context.dpi(8f)
            })
        }
        streamBubble.visibility = View.VISIBLE
        hideTyping()
        updateHead()
        scrollToEnd(true)
    }

    private var state = "idle"
    private var waiting = false
    private var activity: String? = null
    private var said = false
    private var shown = false
    private var count = 0
    /**
     * The messages before the newest [RECENT] it opened with, oldest first: turning up past the top shows the next
     * [RECENT] (T-106: a chat carried on for weeks, ADR-0057, can't build every message as it opens).
     */
    private var earlier: List<JSONObject> = emptyList()
    /** Where [add] puts the next item while earlier messages go in above the rest; -1 otherwise. */
    private var prepending = -1
    private val cards = HashMap<Long, BuildCard>()
    /** Builds still running, pinned under the conversation. */
    private val pinned = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    /** The newest finished build with an app: a click opens it. */
    private var ready: BuildCard? = null
    /** A click on a running build arms its stop; a second click within 3 s stops it. */
    private val stopping = Confirm { cards.values.forEach { it.update() }; updateHead() }
    /** Photos taken from here (ADR-0031), going with your next words: shown over the footer. */
    private val pending = ArrayList<String>()
    private val pendingRow = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.END
        visibility = View.GONE
    }

    init {
        val head = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            clipChildren = false
            addView(character, LinearLayout.LayoutParams(context.dpi(44f), context.dpi(44f)))
            addView(nameView.apply { setPadding(context.dpi(10f), 0, 0, 0) },
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            // The camera, by touch (a click does it too, when nothing else wants the click).
            addView(android.widget.ImageView(context).apply {
                setImageDrawable(dev.r1os.host.ui.Icons.drawable("camera", Theme.MUTED))
                setPadding(context.dpi(8f), context.dpi(8f), context.dpi(8f), context.dpi(8f))
                setOnClickListener { openCamera() }
            }, LinearLayout.LayoutParams(context.dpi(40f), context.dpi(40f)))
        }
        header = BackHeader(context, head, accent, height = 52f)
        header.onBackTouched { shell.back() }
        // The header stays put at the top; the conversation scrolls beneath it.
        view.addView(header, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(MARGIN - BackHeader.INSET), 0, context.dpi(MARGIN), 0)
        })
        body.addView(scroll, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        view.addView(body, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f).apply {
            setMargins(context.dpi(MARGIN), 0, context.dpi(MARGIN), 0)
        })
        view.addView(pinned, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(MARGIN), 0, context.dpi(MARGIN), 0)
        })
        view.addView(pendingRow, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(MARGIN), context.dpi(6f), context.dpi(MARGIN), 0)
        })
        foot.apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
            setPadding(0, context.dpi(10f), 0, context.dpi(2f))
            addView(talkDot(), LinearLayout.LayoutParams(context.dpi(24f), context.dpi(24f)))
            addView(footText)
            // While it works, a touch here does what a click does: once arms the stop, again stops it (T-156).
            setOnClickListener {
                if (busy && chatId != null) stopping.tap("turn") { stopTurn() }
            }
        }
        view.addView(box, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(MARGIN), context.dpi(6f), context.dpi(MARGIN), 0)
        })
        view.addView(foot, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        list.addView(empty, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        streamBubble.visibility = View.GONE
        typing.view.visibility = View.GONE
        updateHead()

        // What you just said goes in after the conversation so far: once it has loaded (2026-10-09, Max: sent
        // straight away, it landed above the history, which loads in the background).
        val sayNow = {
            if (say != null && !said) {
                said = true
                if (busy) {
                    // Still answering what came before: show the words, and that they didn't go.
                    addBubble("user", say.trim())
                    addBubble("error", "Still answering the last message: say it again in a moment")
                } else {
                    send(say, sayApp, sayImages)
                }
            }
        }
        val cid = chatId
        if (cid != null) {
            R1d.get("/api/chats/$cid", onError = { sayNow() }) { r ->
                // A thread with another agent (ADR-0012): both names in the header.
                val c = r.optJSONObject("chat")
                if (c?.optString("kind") == "thread") {
                    val peer = shell.agent(c.optString("peer"))?.name ?: c.optString("title")
                    nameView.text = "${agent.name} · $peer"
                    nameView.setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 24f)
                    empty.text = "What ${agent.name} and $peer say to each other shows here."
                }
                val ms = r.optJSONArray("messages") ?: JSONArray()
                val all = (0 until ms.length()).mapNotNull { ms.optJSONObject(it) }
                earlier = all.dropLast(RECENT)
                all.takeLast(RECENT).forEach { addMessage(it) }
                if (r.optBoolean("busy")) setBusy(true)
                // A build from here still running: pinned until it's done.
                r.optJSONObject("building")?.let { b ->
                    val id = b.optLong("build")
                    if (id != 0L && cards[id] == null) {
                        val app = b.optString("app").ifEmpty { null }
                        pin(BuildCard(id, app != null).also { it.app = app; it.update() })
                    }
                }
                scrollToEnd(false)
                sayNow()
            }
        }
        when {
            say == null -> warm()
            cid == null -> sayNow()
        }
    }

    // ---- items ----------------------------------------------------------------------------------

    private fun add(v: View, gravity: Int = Gravity.START, stretch: Boolean = false) {
        if (empty.parent != null) list.removeView(empty)
        val lp = LinearLayout.LayoutParams(if (stretch) LinearLayout.LayoutParams.MATCH_PARENT else LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            this.gravity = gravity
            topMargin = if (count == 0 || prepending == 0) 0 else context.dpi(10f)
        }
        count++
        if (prepending >= 0) {
            list.addView(v, prepending++, lp)
            return
        }
        // Keep the live bits (streaming reply, typing dots) last.
        val at = list.indexOfChild(streamBubble).takeIf { it >= 0 } ?: list.indexOfChild(typing.view).takeIf { it >= 0 } ?: list.childCount
        list.addView(v, at, lp)
        scrollToEnd(true)
    }

    /**
     * A message: big Rubik on a solid fill (no outlines). Yours in the agent's colour with ink text,
     * the agent's on dark grey; the corner nearest its side is tucked in. Shown as written.
     */
    private fun bubble(role: String, text: String): TextView = context.text(18f, 500).keepCase().apply {
        this.text = text
        setLineSpacing(0f, 1.22f)
        setPadding(context.dpi(14f), context.dpi(11f), context.dpi(14f), context.dpi(11f))
        maxWidth = bubbleMax.toInt()
        val r = context.dp(20f)
        val s = context.dp(6f)
        background = when (role) {
            "user" -> GradientDrawable().apply { cornerRadii = floatArrayOf(r, r, r, r, s, s, r, r); setColor(accent) }
            "assistant" -> GradientDrawable().apply { cornerRadii = floatArrayOf(r, r, r, r, r, r, s, s); setColor(Theme.CARD) }
            else -> null
        }
        when (role) {
            "user" -> setTextColor(Theme.ON_ACCENT)
            "error" -> {
                setTextColor(Theme.BAD)
                setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 15f)
                gravity = Gravity.CENTER
                transformationMethod = dev.r1os.host.ui.Lowercase // an error, not something said
            }
        }
    }

    /** Photos you sent: thumbnails on your side, over your words. */
    private fun addPhotos(ids: List<String>) {
        if (ids.isEmpty()) return
        val row = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
        for (id in ids) {
            row.addView(android.widget.ImageView(context).apply {
                scaleType = android.widget.ImageView.ScaleType.CENTER_CROP
                background = roundRect(Theme.CARD, context.dp(16f))
                clipToOutline = true
                dev.r1os.host.camera.Photos.show(this, id, context.dpi(PHOTO_DP))
            }, LinearLayout.LayoutParams(context.dpi(PHOTO_DP), context.dpi(PHOTO_DP)).apply { leftMargin = context.dpi(6f) })
        }
        add(row, Gravity.END)
    }

    /** Words with a photo just taken (System One's "what am I looking at", ADR-0032). */
    fun sendPhoto(words: String, id: String) = send(words, null, pending + id)

    /** The camera, for a photo to send (ADR-0031): your next words take it along; talk there to send both at once. */
    private fun openCamera() {
        shell.push(CameraScreen(shell, source = "chat") { id, words ->
            if (words != null) {
                send(words, null, pending + id)
            } else {
                pending += id
                showPending()
            }
        })
    }

    private fun showPending() {
        pendingRow.removeAllViews()
        pendingRow.visibility = if (pending.isEmpty()) View.GONE else View.VISIBLE
        for (id in pending) {
            pendingRow.addView(android.widget.ImageView(context).apply {
                scaleType = android.widget.ImageView.ScaleType.CENTER_CROP
                background = roundRect(Theme.CARD, context.dp(12f))
                clipToOutline = true
                dev.r1os.host.camera.Photos.show(this, id, context.dpi(56f))
            }, LinearLayout.LayoutParams(context.dpi(56f), context.dpi(56f)).apply { leftMargin = context.dpi(6f) })
        }
    }

    private fun addBubble(role: String, text: String) = when (role) {
        "user" -> add(bubble(role, text), Gravity.END)
        "error" -> add(bubble(role, text), Gravity.CENTER_HORIZONTAL)
        else -> add(bubble("assistant", plain(text)), Gravity.START)
    }

    /** A routine ran here (T-092): its title and when, in place of its prompt, so its runs don't run together. */
    private fun addRoutine(title: String, at: Long) {
        val time = android.text.format.DateFormat.getTimeFormat(context).format(java.util.Date(at))
        val day = if (android.text.format.DateUtils.isToday(at)) "" else android.text.format.DateFormat.format("EEE ", at).toString()
        add(context.text(13f, 600, Theme.MUTED).apply {
            this.text = "$title · $day$time"
            gravity = Gravity.CENTER
            setPadding(context.dpi(12f), context.dpi(14f), context.dpi(12f), context.dpi(4f))
        }, stretch = true)
    }

    /** r1d tidied a long chat into a fresh session here: a quiet line, no rules around it. */
    private fun addNote(text: String) = add(context.text(14f, 500, Theme.MUTED).apply {
        this.text = "· $text ·"
        gravity = Gravity.CENTER
        setPadding(context.dpi(12f), context.dpi(8f), context.dpi(12f), context.dpi(8f))
    }, stretch = true)

    /**
     * What the agent is doing: one small pill in the agent's colour for the whole turn. It shows the tool in
     * use now, and `+N` for the ones before it (Max, 2026-10-10: one badge, not a pill per call).
     */
    private var toolBadge: TextView? = null
    private var tools = 0

    private fun addTool(name: String) {
        tools++
        val label = toolLabel(name)
        val more = if (tools > 1) "  +${tools - 1}" else ""
        val text = android.text.SpannableString(label + more).apply {
            if (more.isNotEmpty()) {
                setSpan(android.text.style.ForegroundColorSpan(Theme.withAlpha(accent, 0.6f)), label.length, length, 0)
            }
        }
        val badge = toolBadge?.takeIf { it.parent != null }
        if (badge != null) {
            badge.text = text
            badge.animate().cancel()
            badge.scaleX = 1.06f
            badge.scaleY = 1.06f
            badge.animate().scaleX(1f).scaleY(1f).setDuration(160).setInterpolator(Theme.SETTLE).start()
            return
        }
        toolBadge = context.text(14f, 600, accent).apply {
            this.text = text
            setPadding(context.dpi(12f), context.dpi(6f), context.dpi(12f), context.dpi(6f))
            background = roundRect(Theme.withAlpha(accent, 0.14f), context.dp(999f))
        }.also { add(it) }
    }

    /** A turn ended: the next one gets a badge of its own. */
    private fun endTools() {
        toolBadge = null
        tools = 0
    }

    private fun addMessage(m: JSONObject) {
        val role = m.optString("role")
        val text = m.optString("text")
        // A routine's prompt (T-092): you wrote it once, when you set it up; its runs show as dividers.
        val routine = m.optJSONObject("meta")?.optString("routine").orEmpty()
        if (role == "user" && routine.isNotEmpty()) return addRoutine(routine, m.optLong("at"))
        when (role) {
            "build" -> {
                val meta = m.optJSONObject("meta") ?: JSONObject()
                val card = BuildCard(meta.optLong("build"), meta.optString("kind") == "edit")
                card.stage = meta.optString("stage", "done")
                card.app = meta.optString("app").ifEmpty { null }
                card.label = text
                card.stopped = meta.optBoolean("stopped")
                card.deleted = meta.optJSONArray("deleted")?.let { a -> (0 until a.length()).map { a.optString(it) } }
                card.update()
                if (card.id != 0L) cards[card.id] = card
                add(card.view, stretch = true)
                // Only the newest decides what a click opens, not one shown later from further up.
                if (card.stage == "done" && card.app != null && prepending < 0) {
                    ready = card
                    linkedNewer = false
                }
            }
            "note" -> addNote(text)
            "card" -> m.optJSONObject("meta")?.optJSONObject("card")?.let { addCard(it) }
            "mail" -> addMail(m.optJSONObject("meta"), text, out = false)
            "mail_out" -> addMail(m.optJSONObject("meta"), text, out = true)
            else -> {
                if (role == "user") addPhotos(dev.r1os.host.camera.Photos.idsOf(m.optJSONObject("meta")))
                addBubble(role, text)
            }
        }
    }

    /**
     * A message between agents (ADR-0012), in the other agent's colour: what this agent asked or told another
     * (`out`), or another's question, note or answer. Their words keep their case, like anything said.
     */
    private fun addMail(meta: JSONObject?, text: String, out: Boolean) {
        val other = shell.agent(meta?.optString(if (out) "to" else "from"))
        val name = other?.name ?: "another agent"
        val color = Theme.colorOf(other?.color)
        val kind = meta?.optString("kind")
        val label = when {
            out && kind == "tell" -> "note to $name"
            out -> "asked $name"
            kind == "reply" && meta?.optBoolean("error") == true -> "$name couldn't answer"
            kind == "reply" -> "$name answered"
            kind == "tell" -> "note from $name"
            else -> "$name asks"
        }
        val card = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(12f))
            background = roundRect(Theme.withAlpha(color, if (out) 0.08f else 0.14f), context.dp(18f))
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(View(context).apply { background = roundRect(color, context.dp(4f)) },
                    LinearLayout.LayoutParams(context.dpi(8f), context.dpi(8f)).apply { rightMargin = context.dpi(8f) })
                addView(context.text(14f, 700, color).apply { this.text = label })
            })
            addView(context.text(17f, 500, if (out) Theme.MUTED else Theme.FG).keepCase().apply {
                this.text = plain(text)
                setLineSpacing(0f, 1.18f)
                maxLines = if (out) 3 else 14
                ellipsize = android.text.TextUtils.TruncateAt.END
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
        }
        add(card, stretch = true)
    }

    /**
     * A card the agent showed (ADR-0039): drawn in the chat, full width. One with links is what a click opens next
     * (the newest card or built app wins), unless a newer app was built here since.
     */
    private fun addCard(card: JSONObject) {
        add(cardView(context, card, accent), stretch = true)
        if (cardLinks(card).isNotEmpty() && prepending < 0) {
            linked = card
            linkedNewer = true
        }
    }

    /** The newest card with links, and whether it's newer than [ready]. */
    private var linked: JSONObject? = null
    private var linkedNewer = false

    private fun openCard(card: JSONObject) {
        val links = cardLinks(card)
        Sounds.play(Sounds.S.OPEN)
        if (links.size == 1) shell.open(Target.of("link", "url" to links[0].optString("url")))
        else shell.push(CardScreen(shell, card, accent))
    }

    /** A running build: its card waits under the conversation, sliding up into view. */
    private fun pin(card: BuildCard) {
        cards[card.id] = card
        card.pinned = true
        pinned.addView(card.view, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            topMargin = context.dpi(8f)
        })
        card.view.alpha = 0f
        card.view.translationY = context.dp(24f)
        card.view.animate().alpha(1f).translationY(0f).setDuration(260).setInterpolator(Theme.EASE_OUT).start()
        scrollToEnd(true)
    }

    /** It finished: into the conversation, where it is now. */
    private fun unpin(card: BuildCard) {
        if (!card.pinned) return
        card.pinned = false
        card.view.animate().cancel()
        pinned.removeView(card.view)
        card.view.alpha = 0f
        card.view.translationY = 0f
        add(card.view, stretch = true)
        card.view.animate().alpha(1f).setDuration(220).start()
    }

    /** The [RECENT] messages before the first one shown, above it, where you were staying in view. */
    private fun showEarlier() {
        val chunk = earlier.takeLast(RECENT)
        earlier = earlier.dropLast(RECENT)
        val first = list.getChildAt(0)
        val before = list.height
        prepending = 0
        chunk.forEach { addMessage(it) }
        prepending = -1
        (first?.layoutParams as? LinearLayout.LayoutParams)?.let { it.topMargin = context.dpi(10f); first.layoutParams = it }
        Sounds.play(Sounds.S.TICK)
        list.post { scroll.scrollTo(0, (list.height - before).coerceAtLeast(0)) }
    }

    private fun scrollToEnd(smooth: Boolean) = scroll.post {
        val bottom = (list.height - scroll.height).coerceAtLeast(0)
        if (smooth) scroll.smoothScrollTo(0, bottom) else scroll.scrollTo(0, bottom)
    }

    // ---- state ----------------------------------------------------------------------------------

    private fun setBusy(b: Boolean) {
        busy = b
        if (b && stream.isEmpty()) showTyping() else hideTyping()
        updateHead()
    }

    private fun showTyping() {
        if (typing.view.parent == null) list.addView(typing.view)
        typing.view.visibility = View.VISIBLE
        typing.start()
        scrollToEnd(true)
    }

    private fun hideTyping() {
        typing.stop()
        typing.view.visibility = View.GONE
    }

    private fun updateHead() {
        val s = if (busy) (if (stream.isNotEmpty()) "speaking" else if (state == "idle") "thinking" else state) else "idle"
        character.set(agent.form, agent.color, agent.finish, s, activity, eyes = agent.eyes, wear = agent.wear)
        // Only while it's working: no hint at rest (Max, 2026-10-09).
        footText.text = when {
            busy && stopping.armed == "turn" -> "Click again to stop ${agent.name}"
            busy && waiting -> "Waiting for another agent to finish…"
            busy -> "${agent.name} is on it…"
            else -> ""
        }
        foot.visibility = if (busy) View.VISIBLE else View.GONE
        preview?.takeIf { it.visibility == View.VISIBLE }?.update(agent.name, accent, busy)
    }

    private fun warm() = R1d.post("/api/chats/warm", JSONObject().put("agent", agent.id).put("chat", chatId ?: JSONObject.NULL))

    /** Marks the chat's replies seen while it's on screen. */
    private fun seen() {
        val id = chatId ?: return
        if (shown) R1d.post("/api/chats/$id/seen")
    }

    /**
     * Stops its turn (T-013): r1d asks the agent's CLI to stop, wherever it runs, and the chat gets what it had said
     * and a note ("stopped").
     */
    fun stopTurn() {
        val id = chatId ?: return
        Sounds.play(Sounds.S.CLOSE)
        footText.text = "Stopping…"
        R1d.post("/api/chats/$id/stop")
    }

    /** On its side (T-089): the box, and r1os's keyboard for it. Upright: both go (what's typed stays for next time). */
    fun typeOn(on: Boolean) {
        if (on == typingOn) return
        typingOn = on
        val ime = context.getSystemService(android.view.inputmethod.InputMethodManager::class.java)
        if (on) {
            box.visibility = View.VISIBLE
            box.requestFocus()
            box.setSelection(box.text.length)
            box.post { ime.showSoftInput(box, android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT) }
            scrollToEnd(false)
        } else {
            ime.hideSoftInputFromWindow(box.windowToken, 0)
            box.clearFocus()
            box.visibility = View.GONE
        }
    }

    /**
     * Docked on its side (T-156): the box shows, and its keyboard comes when it's touched, so what the agent says stays in
     * view. Upright again it goes, unless the R1's held on its side to type (T-089).
     */
    override fun onDock(on: Boolean) {
        if (typingOn) return
        box.visibility = if (on) View.VISIBLE else View.GONE
        if (!on) box.clearFocus()
    }

    /** What's typed goes, as if said; the box empties for the next. Not while the agent's still on the last. */
    private fun sendTyped() {
        val text = box.text.toString().trim()
        if (text.isEmpty()) return
        if (busy) {
            footText.text = "${agent.name} is still answering"
            return
        }
        Sounds.play(Sounds.S.POP)
        box.setText("")
        send(text)
    }

    /** Your words from the full browser over this chat (T-090): sent here as if said here. False while it's busy. */
    fun say(text: String): Boolean {
        if (busy) return false
        send(text)
        return true
    }

    private fun send(raw: String, app: String? = null, photos: List<String> = pending.toList()) {
        val text = raw.trim()
        if (text.isEmpty() || busy) return
        state = "thinking"
        setBusy(true)
        endTools()
        addPhotos(photos)
        addBubble("user", text)
        pending.clear()
        showPending()
        val fail = { msg: String ->
            state = "error"
            setBusy(false)
            addBubble("error", msg)
        }
        val post = { id: String ->
            val body = JSONObject().put("text", text)
            if (app != null) body.put("app", app)
            if (photos.isNotEmpty()) body.put("images", JSONArray(photos))
            R1d.post("/api/chats/$id/messages", body, onError = fail)
        }
        val id = chatId
        if (id != null) {
            post(id)
        } else {
            R1d.post("/api/chats", JSONObject().put("agent", agent.id), onError = fail) { r ->
                val made = r.optJSONObject("chat")?.optString("id").orEmpty()
                if (made.isEmpty()) return@post fail("Couldn't start a chat")
                chatId = made
                post(made)
            }
        }
    }

    // ---- events ---------------------------------------------------------------------------------

    override fun onEvent(ev: Ev): Boolean {
        if (header.shown) {
            when {
                ev.type == "wheel" && ev.int("delta") > 0 -> { header.hide(); return true }
                ev.type == "wheel" -> return true
                ev.isTap -> { shell.back(); return true }
            }
        }
        when {
            ev.type == "wheel" -> {
                val d = ev.int("delta")
                when {
                    d < 0 && scroll.scrollY <= 0 && earlier.isNotEmpty() -> showEarlier()
                    d < 0 && scroll.scrollY <= 0 -> header.show()
                    else -> scroll.smoothScrollBy(0, (d * context.dp(60f)).toInt())
                }
                return true
            }
            // A click on a running build: click again to stop it. Otherwise it opens the newest app built here.
            ev.isTap && cards.values.any { it.pinned } -> {
                val running = cards.values.last { it.pinned }
                stopping.tap("stop-${running.id}") {
                    R1d.post("/api/builds/stop", JSONObject().put("build", running.id))
                    Sounds.play(Sounds.S.CLOSE)
                }
                return true
            }
            // While it works: a click, then another, stops it (T-013), as a build stops.
            ev.isTap && busy && chatId != null -> {
                stopping.tap("turn") { stopTurn() }
                return true
            }
            // The R1's browser in the corner (T-090): a click opens it full, as a touch does.
            ev.isTap && preview?.visibility == View.VISIBLE -> {
                Sounds.play(Sounds.S.OPEN)
                shell.push(BrowserScreen(shell))
                return true
            }
            ev.isTap && linkedNewer && linked != null -> {
                // The newest card's links (ADR-0039): a click reads them.
                openCard(linked!!)
                return true
            }
            ev.isTap -> {
                // The newest app built here opens; otherwise the camera, for a photo to send.
                val app = ready?.takeIf { !it.undone }?.app
                if (app == null) {
                    Sounds.play(Sounds.S.OPEN)
                    openCamera()
                    return true
                }
                Sounds.play(Sounds.S.OPEN)
                shell.open(Target.of("app", "id" to app))
                return true
            }
            ev.type == "voice" && ev.str("state") == "final" -> {
                val t = ev.str("text")?.trim().orEmpty()
                if (t.isEmpty()) return false
                // "Stop" while it works stops it (T-013); other words wait their turn, as before.
                if (busy && isStop(t)) {
                    stopTurn()
                    return true
                }
                send(t)
                return true
            }
        }
        return false
    }

    override fun onAnyEvent(ev: Ev) {
        val id = chatId
        when (ev.type) {
            "agent_state" -> if (id != null && ev.str("chat") == id) {
                state = ev.str("state") ?: "idle"
                // A turn r1d started here (another agent's answer to pass on, ADR-0012): show it working.
                if (state == "thinking" && !busy) setBusy(true)
                waiting = state == "waiting"
                if (state == "idle" || state == "error") activity = null
                updateHead()
            }
            // Replies here are seen while the chat is on screen (r1d counts the rest as unread).
            "unread" -> if (id != null && ev.str("chat") == id && ev.int("count") > 0) seen()
            "build" -> cards[ev.json.optLong("id")]?.let { c ->
                c.stage = ev.str("stage") ?: c.stage
                c.section = ev.str("section") ?: c.section
                c.app = ev.str("app") ?: c.app
                c.error = ev.str("error")
                if (ev.str("kind") == "edit") c.edit = true
                if (ev.json.optBoolean("stopped")) c.stopped = true
                c.said = ev.str("said") ?: c.said
                // A new app's card says what it's called (the stored one reads "Built <name>" too).
                ev.str("name")?.let { n -> if (!c.edit) c.label = "Built $n" }
                ev.json.optJSONArray("deleted")?.let { a -> c.deleted = (0 until a.length()).map { a.optString(it) } }
                c.update()
                if (c.stage == "done" || c.stage == "failed") {
                    unpin(c)
                    if (c.stage == "done" && c.app != null) {
                        ready = c
                        linkedNewer = false
                    }
                    scrollToEnd(true)
                }
            }
            // Warm the agent as soon as the user starts talking here.
            "voice" -> if (ev.str("state") == "listening" && shell.top() === this) warm()
            "chat" -> {
                if (id == null || ev.str("chat") != id) return
                val text = ev.str("text")
                when (ev.str("kind")) {
                    "delta" -> {
                        stream.append(text.orEmpty())
                        // Several land in a frame: the bubble shows them once, as that frame starts (T-106).
                        if (!streamPending) {
                            streamPending = true
                            list.postOnAnimation(streamShow)
                        }
                    }
                    "tool" -> {
                        activity = CharacterView.activityOf(text)
                        clearStream()
                        addTool(text.orEmpty())
                        if (busy) showTyping()
                        updateHead()
                    }
                    "build" -> {
                        val card = BuildCard(ev.json.optLong("build"), ev.str("app") != null)
                        card.app = ev.str("app")
                        card.update()
                        pin(card)
                    }
                    "done" -> {
                        clearStream()
                        endTools()
                        addBubble("assistant", text.orEmpty())
                        setBusy(false)
                    }
                    "error" -> {
                        clearStream()
                        endTools()
                        addBubble("error", text ?: "Something went wrong")
                        setBusy(false)
                    }
                    "note" -> addNote(text.orEmpty())
                    "routine" -> addRoutine(text.orEmpty(), System.currentTimeMillis())
                    // You stopped it (T-013): what it had said by then, and a note.
                    "stopped" -> {
                        clearStream()
                        endTools()
                        if (!text.isNullOrBlank()) addBubble("assistant", text)
                        addNote("stopped")
                        setBusy(false)
                    }
                    "card" -> ev.json.optJSONObject("card")?.let {
                        clearStream()
                        addCard(it)
                    }
                    "mail" -> addMail(ev.json.optJSONObject("mail"), text.orEmpty(), out = false)
                    "mail_out" -> addMail(ev.json.optJSONObject("mail"), text.orEmpty(), out = true)
                }
            }
        }
    }

    private fun clearStream() {
        list.removeCallbacks(streamShow)
        streamPending = false
        stream.clear()
        streamBubble.visibility = View.GONE
        list.removeView(streamBubble)
    }

    override fun onShow() {
        shown = true
        character.setActive(true)
        typing.resume()
        seen()
        // The browser this chat's agent was using is still open (back from the full browser, say): its preview again.
        offBrowser?.invoke()
        offBrowser = dev.r1os.host.AgentBrowser.onChange { refreshPreview() }
        refreshPreview()
        offApps?.invoke()
        offApps = dev.r1os.host.AgentApps.onChange { refreshAppPreview() }
        refreshAppPreview()
    }

    override fun onHide() {
        shown = false
        character.setActive(false)
        typing.pause()
        header.hide()
        offBrowser?.invoke()
        offBrowser = null
        preview?.detach()
        offApps?.invoke()
        offApps = null
    }

    // ---- the R1's browser, small (T-090) ----------------------------------------------------------------

    /** This chat's agent is using the R1's browser: its live preview in the corner (the shell calls it). */
    fun browserPreview() {
        if (!shown) return
        val p = preview ?: BrowserPreview(context) { shell.push(BrowserScreen(shell)) }.also { p ->
            preview = p
            body.addView(p, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or Gravity.END).apply {
                bottomMargin = context.dpi(8f)
            })
        }
        p.visibility = View.VISIBLE
        p.attach()
        p.update(agent.name, accent, busy)
        p.post { if (p.visibility == View.VISIBLE) previewRoom(true) }
    }

    /** Room under the conversation for the preview, so its last words scroll clear of it. */
    private fun previewRoom(on: Boolean) {
        val tallest = listOfNotNull(preview, appPreview).filter { it.visibility == View.VISIBLE }.maxOfOrNull { it.height } ?: 0
        val bottom = context.dpi(12f) + if (on) tallest + context.dpi(8f) else 0
        if (list.paddingBottom == bottom) return
        list.setPadding(0, context.dpi(10f), 0, bottom)
        if (on) scrollToEnd(true)
    }

    /** The preview as the browser stands: gone once it's closed or someone else's (your own, another chat's). */
    private fun refreshPreview() {
        val b = dev.r1os.host.AgentBrowser
        val ours = b.open && !b.mine && b.chat == chatId && chatId != null
        if (!ours) {
            preview?.let { it.detach(); it.visibility = View.GONE; previewRoom(false) }
            return
        }
        if (shown) browserPreview()
    }

    /** This chat's agent is using an Android app (ADR-0061): its live preview in the other corner (the shell calls it). */
    fun appWindowPreview() {
        if (!shown) return
        val p = appPreview ?: AppPreview(context) { shell.push(AppWindowScreen(shell)) }.also { p ->
            appPreview = p
            body.addView(p, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or Gravity.START).apply {
                bottomMargin = context.dpi(8f)
            })
        }
        p.visibility = View.VISIBLE
        p.update(agent.name, accent, busy)
        p.post { if (p.visibility == View.VISIBLE) previewRoom(true) }
    }

    /** The app preview as the window stands: gone once it's closed or another chat's. */
    private fun refreshAppPreview() {
        val a = dev.r1os.host.AgentApps
        val ours = a.open && a.chat == chatId && chatId != null
        if (!ours) {
            appPreview?.let { it.visibility = View.GONE; previewRoom(preview?.visibility == View.VISIBLE) }
            return
        }
        if (shown) appWindowPreview()
    }

    // ---- pieces -----------------------------------------------------------------------------------

    /** A build the agent started (or its stored result): live while it runs. */
    private inner class BuildCard(val id: Long, var edit: Boolean) {
        var stage = "writing"
        var section: String? = null
        var app: String? = null
        var error: String? = null
        var label: String? = null
        var said: String? = null
        var deleted: List<String>? = null
        var undone = false
        /** Running, under the conversation. */
        var pinned = false
        var stopped = false

        private val title = context.text(19f, 700)
        private val sub = context.text(15f, 500, Theme.MUTED)
        private val warn = context.text(14f, 500, Theme.BAD)
        private val open = context.text(17f, 700, Theme.ON_ACCENT).apply {
            text = "Open"
            gravity = Gravity.CENTER
            setPadding(context.dpi(18f), context.dpi(9f), context.dpi(18f), context.dpi(9f))
            background = roundRect(accent, context.dp(14f))
            setOnClickListener { app?.let { shell.open(Target.of("app", "id" to it)) } }
        }
        private val undo = context.text(16f, 600, Theme.MUTED).apply {
            text = "Undo"
            setPadding(context.dpi(14f), context.dpi(8f), context.dpi(14f), context.dpi(8f))
            setOnClickListener {
                val a = app ?: return@setOnClickListener
                R1d.post("/api/apps/${R1d.enc(a)}/undo") { undone = true; update() }
            }
        }
        private val actions = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(open)
            addView(undo)
        }
        val view = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(12f), context.dpi(12f), context.dpi(12f), context.dpi(12f))
            addView(title)
            addView(sub, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
            addView(warn, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
            addView(actions, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
        }

        fun update() {
            val done = stage == "done"
            val failed = stage == "failed"
            title.text = if (edit) {
                when { done && undone -> "Change undone"; done -> "App changed"; stopped -> "Change stopped"; failed -> "Change didn't work"; else -> "Changing the app" }
            } else {
                when { done -> "App ready"; stopped -> "Build stopped"; failed -> "Build failed"; else -> "Building your app" }
            }
            sub.text = when {
                pinned && stopping.armed == "stop-$id" -> "click again to stop"
                done -> said ?: label ?: app ?: ""
                stopped -> "nothing was installed"
                failed -> error ?: label ?: ""
                stage == "repairing" -> "fixing what the compiler found"
                stage == "checking" -> "checking its data"
                else -> section ?: "starting"
            }
            val del = deleted
            warn.visibility = if (done && !del.isNullOrEmpty() && !undone) View.VISIBLE else View.GONE
            if (del != null) warn.text = "Deleted with its data: ${del.joinToString(", ")}"
            actions.visibility = if (done && app != null) View.VISIBLE else View.GONE
            undo.visibility = if (edit && !undone) View.VISIBLE else View.GONE
            view.alpha = if (undone) 0.6f else 1f
            view.background = roundRect(Theme.CARD, context.dp(20f))
            title.setTextColor(when { done && !undone -> Theme.OK; stopped -> Theme.MUTED; failed -> Theme.BAD; else -> Theme.FG })
            sub.setTextColor(if (pinned && stopping.armed == "stop-$id") Theme.BAD else Theme.MUTED)
        }
    }

    /** Three dots bobbing while the agent works on a reply. */
    private inner class Typing {
        private val dots = List(3) { View(context).apply { background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(Theme.MUTED) } } }
        val view = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(context.dpi(12f), context.dpi(8f), context.dpi(12f), context.dpi(8f))
            dots.forEachIndexed { i, d -> addView(d, LinearLayout.LayoutParams(context.dpi(7f), context.dpi(7f)).apply { if (i > 0) leftMargin = context.dpi(5f) }) }
        }
        private val anims = dots.mapIndexed { i, d ->
            ObjectAnimator.ofPropertyValuesHolder(d,
                PropertyValuesHolder.ofFloat(View.ALPHA, 1f, 0.25f, 1f),
                PropertyValuesHolder.ofFloat(View.TRANSLATION_Y, 0f, -context.dp(3f), 0f)).apply {
                duration = 1000
                repeatCount = ValueAnimator.INFINITE
                startDelay = i * 150L
                interpolator = Theme.EASE_IN_OUT
            }
        }
        private var on = false

        fun start() { on = true; if (shown) anims.forEach { if (!it.isStarted) it.start() } }
        fun stop() { on = false; anims.forEach { it.cancel() } }
        fun resume() { if (on) start() }
        fun pause() = anims.forEach { it.cancel() }
    }

    /** The footer's dot: the accent, with a ring pulsing outward (hold to talk). */
    private fun talkDot(): View {
        // It pulses only while it's on screen: not under another screen, nor with the display off (T-033).
        val box = object : FrameLayout(context) {
            var pulse: android.animation.Animator? = null
            override fun onVisibilityAggregated(isVisible: Boolean) {
                super.onVisibilityAggregated(isVisible)
                pulse?.let { if (isVisible) { if (!it.isStarted) it.start() } else it.cancel() }
            }
        }
        val ring = View(context).apply { background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(Theme.withAlpha(accent, 0.55f)) } }
        val dot = View(context).apply { background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(accent) } }
        box.addView(ring, FrameLayout.LayoutParams(context.dpi(8f), context.dpi(8f), Gravity.CENTER))
        box.addView(dot, FrameLayout.LayoutParams(context.dpi(8f), context.dpi(8f), Gravity.CENTER))
        ObjectAnimator.ofPropertyValuesHolder(ring,
            PropertyValuesHolder.ofKeyframe(View.SCALE_X, android.animation.Keyframe.ofFloat(0f, 1f), android.animation.Keyframe.ofFloat(0.7f, 3f), android.animation.Keyframe.ofFloat(1f, 3f)),
            PropertyValuesHolder.ofKeyframe(View.SCALE_Y, android.animation.Keyframe.ofFloat(0f, 1f), android.animation.Keyframe.ofFloat(0.7f, 3f), android.animation.Keyframe.ofFloat(1f, 3f)),
            PropertyValuesHolder.ofKeyframe(View.ALPHA, android.animation.Keyframe.ofFloat(0f, 1f), android.animation.Keyframe.ofFloat(0.7f, 0f), android.animation.Keyframe.ofFloat(1f, 0f)),
        ).apply {
            duration = 2400
            repeatCount = ValueAnimator.INFINITE
            interpolator = Theme.EASE_OUT
            box.pulse = this
            if (box.isShown) start()
        }
        return box
    }

    companion object {
        /** A sent photo's thumbnail. */
        const val PHOTO_DP = 120f
        /** Messages a chat opens with, and adds each time you turn up past the top (T-106). */
        const val RECENT = 40
        /** Replies are for a tiny screen: markdown links become their text, emphasis goes. */
        fun plain(text: String): String = text
            .replace(Regex("""\[([^\]]+)]\((?:https?:)?[^)]*\)"""), "$1")
            .replace(Regex("""\*\*([^*]+)\*\*"""), "$1")
            .replace(Regex("""(^|\s)\*([^*\n]+)\*(?=\s|[.,!?]|$)"""), "$1$2")
            .replace(Regex("""`([^`\n]+)`"""), "$1")

        /** "Stop" said while an agent works (T-013). */
        private val STOP_WORDS = setOf("stop", "stop it", "stop that", "stop stop", "please stop", "stop please", "cancel", "cancel that", "cancel it", "never mind", "nevermind")

        fun isStop(words: String): Boolean = words.lowercase().replace(Regex("[^a-z ]"), "").trim().replace(Regex(" +"), " ") in STOP_WORDS

        /** The conversation's side margins: bubbles keep clear of the screen's edges. */
        const val MARGIN = 14f

        val TOOL_LABEL = mapOf(
            // The R1's browser (ADR-0054).
            "browser_open" to "Opening a page", "browser_read" to "Reading the page", "browser_text" to "Reading the page",
            "browser_find" to "Looking on the page", "browser_click" to "Clicking", "browser_type" to "Typing",
            "browser_key" to "Pressing a key", "browser_select" to "Choosing", "browser_scroll" to "Scrolling",
            "browser_screenshot" to "Looking at the page", "browser_back" to "Going back", "browser_close" to "Closing the browser",
            "browser_ask_user" to "Waiting for you",
            // Android apps in the app window (ADR-0061).
            "list_android_apps" to "Looking at your apps", "app_open" to "Opening the app", "app_read" to "Reading the screen",
            "app_screenshot" to "Looking at the screen", "app_click" to "Tapping", "app_type" to "Typing", "app_scroll" to "Scrolling",
            "app_back" to "Going back", "app_find" to "Looking for it", "app_wait" to "Waiting for the app", "app_do" to "Working in the app",
            "app_ask_user" to "Waiting for you", "app_close" to "Closing the app",
            "create_app" to "Starting a build", "edit_app" to "Changing an app", "list_apps" to "Looking at your apps",
            "describe_app" to "Looking at an app", "query_app" to "Checking your data", "run_action" to "Updating an app",
            "undo_action" to "Undoing that", "open_app" to "Opening an app", "app_history" to "Looking at the history",
            "undo_app_change" to "Undoing the change", "delete_app" to "Removing an app", "restore_app" to "Checking the trash",
            "remember" to "Remembering that", "list_memories" to "Checking what I remember", "forget" to "Forgetting that",
            "WebSearch" to "Searching the web", "WebFetch" to "Reading a page", "Bash" to "Working it out",
            "Read" to "Reading a file", "Write" to "Writing a file", "Edit" to "Editing a file",
            "Glob" to "Looking through files", "Grep" to "Searching files",
            "list_agents" to "Checking the others", "ask_agent" to "Asking another agent", "tell_agent" to "Leaving a note",
            "read_feed" to "Reading the news",
            "list_android_apps" to "Looking at your Android apps", "open_android_app" to "Opening an Android app",
            "show_android_app" to "Opening the app store", "show_card" to "Making a card",
            "list_notifications" to "Checking your notifications", "open_notification" to "Opening it",
            "dismiss_notification" to "Clearing it", "reply_notification" to "Sending your reply", "control_media" to "Checking the player",
        )

        /**
         * What a badge says for a tool: r1os's own in words ([TOOL_LABEL]); a connection's as its service and what it does
         * ("mcp__claude_ai_Gmail__search_threads" → "Gmail · search threads"); any other in words. Shared with the
         * simulator and the companion (shell/src/tool-labels.ts).
         */
        fun toolLabel(name: String): String {
            TOOL_LABEL[name]?.let { return it }
            MCP.matchEntire(name)?.destructured?.let { (server, tool) -> return "${service(server)} · ${words(tool)}" }
            return words(name).replaceFirstChar { it.uppercase() }
        }

        private val MCP = Regex("mcp__(.+?)__(.+)")

        /** A connection's server as people know it: "claude_ai_Google_Calendar" → "Google Calendar"; Codex's apps are ChatGPT's. */
        private fun service(server: String): String =
            if (server == "codex_apps") "ChatGPT" else server.removePrefix("claude_ai_").replace(Regex("[_-]+"), " ").trim()

        /** A tool's own name in words: "search_threads" → "search threads", "getEvents" → "get events". */
        private fun words(name: String): String =
            name.replace(Regex("([a-z0-9])([A-Z])"), "$1 $2").replace(Regex("[_-]+"), " ").trim().lowercase()
    }
}
