package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.animation.OvershootInterpolator
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextClock
import dev.r1os.host.net.Agent
import dev.r1os.host.net.R1d
import dev.r1os.host.net.currentChat
import dev.r1os.host.ui.Backdrop
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Look
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Spring
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject

/**
 * The glance screen (ADR-0019): what the R1 wakes to. The time, date, battery and Wi-Fi at the top
 * left, and the agent you were last with peeking up over the bottom edge.
 *
 *   click        sleep again
 *   turn down    the next widget (ADR-0042): apps' cards, one at a time between the time and the agent
 *   turn up      home, by morphing into it: the clock shrinks into home's clock and the agent rises
 *                to the middle, while home fades in around them. One spring follows the wheel, so a
 *                turn is one fluid motion: the first ticks resist (each moves it less), the third
 *                carries it the rest of the way. A stray tick only peeks; a pause or a turn down
 *                eases it back.
 *   hold         talk to that agent
 *   double click home
 *   touch        the agent: a tap opens your chat with it (T-122), a stroke pets it (ADR-0043); a widget's
 *                card opens its app; a notification opens, and a swipe across it puts it away (T-129).
 *                What a touch opens, opens straight over this as it fades (T-128), not on home
 *
 * And the way back (ADR-0022): turning up past the first agent on home folds home back into it,
 * the same morph in reverse and with the same resistance, and the R1 sleeps.
 *
 * Touch leaves it only where you aim (the agent, a card, the row: ADR-0059); there's no swipe that opens
 * it, so a hand in a pocket can't. It sits over the whole shell, and with a PIN it's the lock screen
 * (ADR-0016): touches that would leave open the dial. Pulled down from the top edge over another screen
 * (T-124, [dropBegin]), it goes back to that screen, not home. The shell routes its input.
 */
@SuppressLint("ViewConstructor")
class GlanceView(context: Context, private val home: HomeScreen) : FrameLayout(context) {
    /** Its background over home (the look's, ADR-0043); it fades as the morph goes, so home appears around the clock and agent. */
    private val scrim = Backdrop(context, Look.background("glance"))
    /** The clock's face (the look's): one line of digits scales onto home's clock in the morph; the others fade. */
    private val clockStyle = Look.clock
    private val clock: View = Clocks.face(context, clockStyle)
    private val textual = Clocks.textual(clockStyle)
    private val date = TextClock(context).apply {
        format12Hour = "EEEE, MMMM d"
        format24Hour = "EEEE, MMMM d"
        setTextSize(TypedValue.COMPLEX_UNIT_DIP, 17f)
        typeface = Theme.font(500)
        setTextColor(Theme.MUTED)
        includeFontPadding = false
        transformationMethod = dev.r1os.host.ui.Lowercase
    }
    private val battery = context.text(13f, 600, Theme.MUTED).apply { fontFeatureSettings = "tnum" }
    private val bolt = ImageView(context).apply { setImageDrawable(Icons.drawable("lightning", Theme.MUTED)) }
    private val wifi = ImageView(context)
    private var wifiIcon = ""
    /** Locked (ADR-0016): a small lock after the Wi-Fi. */
    private val lockIcon = ImageView(context).apply { setImageDrawable(Icons.drawable("lock-simple", Theme.MUTED)); visibility = GONE }
    /** The next alarm (ADR-0028), after the status: its icon and time. */
    private val alarmIcon = ImageView(context).apply { setImageDrawable(Icons.drawable("alarm", Theme.MUTED)); visibility = GONE }
    private val alarmTime = context.text(13f, 600, Theme.MUTED).apply { fontFeatureSettings = "tnum"; visibility = GONE }
    /** The soonest timer (ADR-0050), after the alarm: its icon and countdown. */
    private val timerIcon = ImageView(context).apply { setImageDrawable(Icons.drawable("timer", Theme.MUTED)); visibility = GONE }
    private val timerSlot = FrameLayout(context).apply { visibility = GONE }
    private val device = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        addView(battery)
        addView(bolt, LinearLayout.LayoutParams(context.dpi(13f), context.dpi(13f)).apply { leftMargin = context.dpi(2f) })
        addView(wifi, LinearLayout.LayoutParams(context.dpi(15f), context.dpi(15f)).apply { leftMargin = context.dpi(9f) })
        addView(lockIcon, LinearLayout.LayoutParams(context.dpi(14f), context.dpi(14f)).apply { leftMargin = context.dpi(9f) })
        addView(alarmIcon, LinearLayout.LayoutParams(context.dpi(14f), context.dpi(14f)).apply { leftMargin = context.dpi(10f) })
        addView(alarmTime, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = context.dpi(3f) })
        addView(timerIcon, LinearLayout.LayoutParams(context.dpi(14f), context.dpi(14f)).apply { leftMargin = context.dpi(10f) })
        addView(timerSlot, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = context.dpi(3f) })
    }
    private val character = CharacterView(context, context.dpi(CHAR_DP))

    /**
     * What's playing in the background (ADR-0040): the song or station, and touch controls (Max, 2026-10-10: "Touch
     * controls instead", so a click still sleeps). Shown only while the player has something.
     */
    private val npTitle = context.text(18f, 700).keepCase().apply { maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END }
    private val npSub = context.text(13f, 600, Theme.MUTED).keepCase().apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END }
    private val npToggle = ImageView(context)
    private val nowPlaying = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(14f), context.dpi(12f), context.dpi(10f), context.dpi(12f))
        background = roundRect(Theme.CARD, context.dp(20f))
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            addView(npTitle)
            addView(npSub, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(3f) })
        }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        addView(control(npToggle) {
            if (dev.r1os.host.Player.active) dev.r1os.host.Player.toggle() else dev.r1os.host.MediaSessions.toggle()
        }, LinearLayout.LayoutParams(context.dpi(48f), context.dpi(48f)).apply { leftMargin = context.dpi(8f) })
        addView(control(ImageView(context).apply { setImageDrawable(Icons.drawable("stop", Theme.FG)) }) {
            if (dev.r1os.host.Player.active) dev.r1os.host.Player.stop() else dev.r1os.host.MediaSessions.stop()
        },
            LinearLayout.LayoutParams(context.dpi(48f), context.dpi(48f)).apply { leftMargin = context.dpi(6f) })
    }

    /**
     * What's waiting, the first page (T-130, Max: "Notifications take priority over widgets, and should be their own
     * \"widget\" view in the list, always showing up first"): Android apps' notifications (dev.r1os.host.Notifications) and
     * the reminders you haven't seen (an agent's, an app's: [setWaiting]), newest first. Banners go by themselves after a
     * few seconds (T-123); this is where what they said stays. A touch opens one (straight to it, T-128); a swipe across
     * one puts it away (T-129: an Android app's dismissed, a reminder seen). It wakes to this page while there's any.
     */
    private class Note(
        val key: String, val icon: android.graphics.drawable.Drawable, val title: String, val text: String, val at: Long,
        val open: () -> Unit, val gone: () -> Unit,
    )
    private var noteList: List<Note> = emptyList()
    private fun notePages() = if (noteList.isEmpty()) 0 else 1

    /**
     * Widgets (ADR-0042): apps' cards, filled in by r1d (crate::glance), one at a time between the time and the agent
     * (Max, 2026-10-10: "One at a time"). Turning down shows the next, and round again after the last; while something
     * plays, what's playing is the first page. Touching a card opens its app.
     */
    private var widgets: List<JSONObject> = emptyList()
    private var page = 0
    /**
     * The page you last turned to (T-125, Max: "remember which widget we viewed last"), by what's on it ("app:water",
     * "playing", "reply:<chat>"), since its number moves as replies come and go. Kept in the host's preferences; while
     * it's set, the page follows it as the pages load.
     */
    private var want: String? = null
    /** Something is playing: the first page is the player's. */
    private var playing = false
    private val pageBox = FitBox(context).apply { visibility = GONE; onNatural = { post { fitPage() } } }
    private val dots = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER
        visibility = GONE
    }
    /** Under the time and over the agent: the notifications' summary, then the page and its dots. */
    private val middle = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    /** Docked on its side (T-103, [setDock]): your agents along the bottom. Declared before init, which adds the row. */
    private var dock = false
    private val dockRow = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_HORIZONTAL or Gravity.BOTTOM
        clipChildren = false
        visibility = GONE
    }
    private val dockCast = ArrayList<CharacterView>()
    /** How far the agent peeks now (dp): less while a card needs the room ([fitPage]). */
    private var peek = PEEK_DP
    private var peekAnim: android.animation.ValueAnimator? = null

    private fun pages() = notePages() + work.size + replies.size + (if (playing) 1 else 0) + widgets.size

    /**
     * Replies you haven't read (Max, 2026-10-11: "replace the agent ... with the most recent agent notification ... if
     * there's multiple we should be able to scroll through them, and if we unlock, open to that agent"): one per agent,
     * newest first, from r1d's `/api/whatsup`. Each is a page before what's playing and the widgets, and the agent
     * peeking up is the one whose reply shows (or the newest's, on another page). Leaving opens its chat.
     */
    private class Reply(val agent: String, val chat: String, val title: String, val text: String, val count: Int)
    private var replies: List<Reply> = emptyList()

    /** The reply on the page showing, if it's one. */
    private fun replyOn(): Reply? = replies.getOrNull(page - notePages() - work.size)

    /**
     * An agent working in an app or the R1's browser in the background (T-151, Max: "a new widget on the glance screen
     * that shows what they are working on"): who, where, the task (its chat's title) and the step it's on, and an app's
     * live picture. A page each, after the notifications; touching it opens the chat. Only while one's open: with none,
     * nothing's shown or kept.
     */
    private class Work(val agent: String, val chat: String, val where: String, val browser: Boolean, val asking: String?)
    private var work: List<Work> = emptyList()
    /** Chats' titles (the task), fetched once each. */
    private val titles = HashMap<String, String>()
    /** The step line on the work page showing, kept current as the agent's tools change ([stepChanged]). */
    private var workStep: android.widget.TextView? = null
    private var workStepOf: Work? = null

    /** The agent at work on the page showing, if it's one. */
    private fun workOn(): Work? = work.getOrNull(page - notePages())

    /** Who's working where, again (the app window or the browser opened, asked, answered or closed). */
    private fun refreshWork() {
        val now = ArrayList<Work>()
        val a = dev.r1os.host.AgentApps
        val ag = a.agent
        val ch = a.chat
        if (a.open && ag != null && ch != null) now.add(Work(ag, ch, a.name ?: "an app", false, a.asking))
        val b = dev.r1os.host.AgentBrowser
        val bg = b.agent
        val bc = b.chat
        if (b.open && !b.mine && bg != null && bc != null) {
            val site = b.url?.let { runCatching { android.net.Uri.parse(it).host?.removePrefix("www.") }.getOrNull() }
            now.add(Work(bg, bc, site ?: "the web", true, b.asking))
        }
        val same = now.map { listOf(it.agent, it.chat, it.where, it.asking) } == work.map { listOf(it.agent, it.chat, it.where, it.asking) }
        work = now
        for (w in now) if (w.chat !in titles) {
            titles[w.chat] = ""
            R1d.get("/api/chats/${w.chat}", onError = {}) { r ->
                titles[w.chat] = r.optJSONObject("chat")?.optString("title").orEmpty()
                if (workOn()?.chat == w.chat && morph == null && !locking) showPage()
            }
        }
        if (same) return
        if (page >= pages()) page = 0
        if (morph == null && !locking) showPage()
    }

    /** What an agent at work is doing now: waiting for you, the tool it's in, or done for now. */
    private fun stepOf(w: Work): String {
        if (w.asking != null) return "needs you: ${w.asking}"
        val a = home.shell.agent(w.agent) ?: return ""
        if (a.state !in HomeScreen.BUSY || a.chat != w.chat) return "done for now"
        return a.tool?.takeIf { it.isNotEmpty() }?.let { ChatScreen.toolLabel(it) }?.lowercase() ?: "working on it…"
    }

    /** An agent's tools or state changed: the step line, if a work page shows. */
    fun stepChanged() {
        val w = workStepOf ?: return
        workStep?.text = stepOf(w)
    }

    /** A work page: whose, where, the task, the step, and for an app its live picture (not while locked). */
    private fun workView(w: Work): View {
        val a = home.shell.agent(w.agent)
        val accent = Theme.colorOf(a?.color)
        val words = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(context.text(15f, 700, accent).apply { text = a?.name?.lowercase().orEmpty() })
                addView(context.text(13f, 600, Theme.MUTED).keepCase().apply {
                    text = if (w.browser) "browsing ${w.where}" else "using ${w.where}"
                    maxLines = 1
                    ellipsize = android.text.TextUtils.TruncateAt.END
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = context.dpi(8f) })
            })
            val title = titles[w.chat].orEmpty()
            if (!locked && title.isNotBlank()) {
                addView(context.text(17f, 600, Theme.FG).keepCase().apply {
                    text = title
                    maxLines = 3
                    ellipsize = android.text.TextUtils.TruncateAt.END
                }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
            }
            addView(context.text(14f, 600, accent).keepCase().apply {
                text = stepOf(w)
                maxLines = 2
                ellipsize = android.text.TextUtils.TruncateAt.END
                workStep = this
                workStepOf = w
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
        }
        return LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(16f), context.dpi(14f), context.dpi(14f), context.dpi(14f))
            background = roundRect(Theme.CARD, context.dp(20f))
            addView(words, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            if (!w.browser && !locked) addView(WorkPicture(context), LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                leftMargin = context.dpi(12f)
            })
            setOnClickListener {
                Sounds.play(Sounds.S.OPEN)
                complete { home.shell.open(Target.of("chat", "agent" to w.agent, "id" to w.chat)) }
            }
        }
    }

    /** The app window's live picture, small, with frames only while it's on screen (battery: architecture §7). */
    @android.annotation.SuppressLint("ViewConstructor")
    private class WorkPicture(context: Context) : FrameLayout(context) {
        private val picture = android.widget.ImageView(context).apply { scaleType = android.widget.ImageView.ScaleType.FIT_XY }
        private var off: (() -> Unit)? = null

        init {
            val size = dev.r1os.host.AgentApps.size
            // As tall as at the R1's own shape, narrower for a taller window (ADR-0063).
            val h = context.dpi(WORK_PICTURE_DP) * 4 / 3
            val w = h * size.x / size.y
            clipToOutline = true
            outlineProvider = object : android.view.ViewOutlineProvider() {
                override fun getOutline(view: View, outline: android.graphics.Outline) = outline.setRoundRect(0, 0, view.width, view.height, context.dp(10f))
            }
            setBackgroundColor(android.graphics.Color.BLACK)
            addView(picture, LayoutParams(w, h))
        }

        override fun onVisibilityAggregated(isVisible: Boolean) {
            super.onVisibilityAggregated(isVisible)
            off?.invoke()
            off = if (isVisible) dev.r1os.host.AgentApps.onFrame { picture.setImageBitmap(it) } else null
        }
    }

    /** What's on page `i`, to find it again (T-125). */
    private fun keyOf(i: Int): String? {
        if (i < notePages()) return NOTES
        work.getOrNull(i - notePages())?.let { return "work:" + it.chat }
        val j = i - notePages() - work.size
        return when {
            j < replies.size -> "reply:" + replies[j].chat
            playing && j == replies.size -> "playing"
            else -> widgets.getOrNull(j - replies.size - (if (playing) 1 else 0))?.optString("app")?.takeIf { it.isNotEmpty() }?.let { "app:$it" }
        }
    }

    /** You turned to this page: it's where the glance screen opens next time (the notifications' comes first anyway). */
    private fun remember() {
        want = null
        val k = keyOf(page)?.takeIf { it != NOTES } ?: return
        context.getSharedPreferences("r1os", Context.MODE_PRIVATE).edit().putString(LAST_PAGE, k).apply()
    }

    /** Looks again (on wake, and when an agent's unread count changes while it's up). */
    fun loadReplies() = R1d.get("/api/whatsup") { w ->
        val a = w.optJSONArray("unread") ?: JSONArray()
        val all = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        val counts = all.groupBy { it.optString("agent") }.mapValues { (_, v) -> v.sumOf { it.optInt("count") } }
        val seen = HashSet<String>()
        val fresh = all.filter { seen.add(it.optString("agent")) && home.shell.agent(it.optString("agent")) != null }.map {
            Reply(it.optString("agent"), it.optString("chat"), it.optString("title"), it.optString("last"), counts[it.optString("agent")] ?: 1)
        }
        val newest = fresh.firstOrNull()?.let { it.agent to it.chat }
        // A new newest reply comes to the front while you look; otherwise the page stays where you turned it (or, since
        // it went up, where you left it last time: T-125).
        if (newest != replies.firstOrNull()?.let { it.agent to it.chat }) {
            if (displayOn && up) want = null
            page = notePages()
        }
        replies = fresh
        if (page >= pages()) page = 0
        // Drawn while it's down too, so it comes back up (the fold from home, or waking) as things are (T-093).
        if (morph == null && !locking) showPage()
    }

    /** A reply's page: whose, which chat, and its first words (not while it's locked). Touching it opens the chat. */
    private fun replyView(r: Reply): View {
        val a = home.shell.agent(r.agent)
        val accent = Theme.colorOf(a?.color)
        return LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(16f), context.dpi(14f), context.dpi(16f), context.dpi(14f))
            background = roundRect(Theme.CARD, context.dp(20f))
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(context.text(15f, 700, accent).apply { text = a?.name?.lowercase().orEmpty() })
                addView(context.text(13f, 600, Theme.MUTED).keepCase().apply {
                    text = listOf(r.title, if (r.count > 1) "${r.count} new" else "").filter { it.isNotBlank() }.joinToString(" · ")
                    maxLines = 1
                    ellipsize = android.text.TextUtils.TruncateAt.END
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = context.dpi(8f) })
            })
            addView(context.text(17f, 600, if (locked) Theme.MUTED else Theme.FG).keepCase().apply {
                text = if (locked) "unlock to read it" else r.text.replace(Regex("\\s+"), " ").trim().take(REPLY_CHARS)
                maxLines = 5
                ellipsize = android.text.TextUtils.TruncateAt.END
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
            setOnClickListener {
                Sounds.play(Sounds.S.OPEN)
                complete { openReply(r) }
            }
        }
    }

    private fun openReply(r: Reply) = home.shell.open(Target.of("chat", "agent" to r.agent, "id" to r.chat))

    /**
     * A tap on agent `a` (T-122, Max: "if I tap on an agent, It should open me directly to our chat"): the chat with
     * its reply, if one's waiting, else your current chat with it (ADR-0057). With the PIN on, the dial first.
     */
    private fun openAgent(a: Agent) {
        if (!up || morph != null || locking || home.shell.agent(a.id) == null) return
        Sounds.play(Sounds.S.OPEN)
        val r = replyOn()?.takeIf { it.agent == a.id } ?: replies.firstOrNull { it.agent == a.id }
        if (r != null) {
            complete { openReply(r) }
            return
        }
        // Which chat first, so it opens as this goes (T-128).
        currentChat(a.id) { id -> if (up) complete { home.shell.open(Target.of("chat", "agent" to a.id, "id" to id)) } }
    }

    /** All the widgets r1d sent, whatever the look hides: docked on its side shows them anyway (T-103). */
    private var allWidgets: List<JSONObject> = emptyList()

    /** The widgets r1d sent (`glance` events, `GET /api/glance`): `{app, name, icon, accent?, card?, note?}`. */
    fun setWidgets(a: JSONArray?) {
        allWidgets = (0 until (a?.length() ?: 0)).mapNotNull { a?.optJSONObject(it) }
        pickWidgets()
        showPage()
    }

    /** The look may hide them on the glance screen (Max, 2026-10-12: docked on its side, "show widgets … regardless"). */
    private fun pickWidgets() {
        widgets = if (Look.hides("widgets") && !dock) emptyList() else allWidgets
        if (page >= pages()) page = 0
    }

    /**
     * Puts the current page in its place, with its dots (no pages: the space stays empty). `from`: docked, the side it
     * comes in from (1: the right, as the next; -1: the left).
     */
    private fun showPage(enter: Boolean = false, from: Int = 0) {
        val n = pages()
        pageBox.removeAllViews()
        dots.removeAllViews()
        if (n == 0) {
            pageBox.visibility = GONE
            dots.visibility = GONE
            showAgent()
            post { fitPage() }
            return
        }
        want?.let { k -> (0 until n).firstOrNull { keyOf(it) == k }?.let { page = it } }
        page = Math.floorMod(page, n)
        pageBox.visibility = VISIBLE
        val np = notePages()
        val nw = work.size
        workStep = null
        workStepOf = null
        val v = when {
            page < np -> notesView()
            page - np < nw -> workView(work[page - np])
            page - np - nw < replies.size -> replyView(replies[page - np - nw])
            playing && page - np - nw == replies.size -> nowPlaying
            else -> widgetView(widgets[page - np - nw - replies.size - if (playing) 1 else 0])
        }
        (v.parent as? ViewGroup)?.removeView(v)
        pageBox.addView(v, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.TOP))
        if (enter) {
            v.alpha = 0f
            if (from != 0) v.translationX = context.dp(36f) * from else v.translationY = context.dp(18f)
            v.animate().alpha(1f).translationX(0f).translationY(0f).setDuration(220).setInterpolator(Theme.EASE_OUT).start()
        }
        dots.visibility = if (n > 1) VISIBLE else GONE
        // A reply's page brings its agent up.
        showAgent()
        // The agent makes room once the card's measured (FitBox.onNatural).
        post { fitPage(animate = enter) }
        for (i in 0 until n) {
            dots.addView(View(context).apply {
                background = roundRect(if (i == page) Theme.FG else Theme.MUTED, context.dp(3f))
                alpha = if (i == page) 1f else 0.45f
            }, LinearLayout.LayoutParams(context.dpi(6f), context.dpi(6f)).apply {
                leftMargin = context.dpi(3f); rightMargin = context.dpi(3f)
            })
        }
    }

    /** A turn down: the next page, round again after the last. The agent glances up at it. */
    private fun nextPage() {
        Sounds.play(Sounds.S.TICK)
        want = null
        page++
        showPage(enter = true)
        remember()
        character.glanceAt(-6f)
    }

    /** Docked on its side (T-117). */
    val docked: Boolean get() = dock

    /**
     * Docked, the wheel (along the bottom now) and a swipe turn the pages left and right, round again at the ends:
     * `d` 1 the next, -1 the one before. Nothing folds toward home or sleep (T-117).
     */
    fun turnPage(d: Int) {
        if (pages() < 2) {
            Sounds.play(Sounds.S.BUMP)
            return
        }
        Sounds.play(Sounds.S.TICK)
        want = null
        page += d
        showPage(enter = true, from = d)
        remember()
    }

    /**
     * A word from the agent on wake (ADR-0043): what's waiting for you, or the time of day. Not on every wake: when
     * the part of the day changes, there's something new, or it's been a while; never with a hidden agent.
     */
    private val bubble = context.text(16f, 650, Theme.FG).apply {
        setPadding(context.dpi(14f), context.dpi(9f), context.dpi(14f), context.dpi(10f))
        background = roundRect(Theme.SURFACE, context.dp(18f))
        visibility = GONE
    }
    private val unbubble = Runnable {
        bubble.animate().alpha(0f).translationY(context.dp(6f)).setDuration(260).withEndAction { bubble.visibility = GONE }.start()
        dots.animate().alpha(1f).setDuration(260).start()
    }
    private val greetNow = Runnable { greet() }
    private val yawnNow = Runnable { if (up && morph == null) character.react("yawn") }

    private fun greet() {
        if (!up || morph != null || Look.hides("greeting") || Look.agentPlace == "hidden") return
        // A reply's page already says it (and the bubble would sit over the card).
        if (replyOn() != null) return
        val cal = java.util.Calendar.getInstance()
        val hour = cal.get(java.util.Calendar.HOUR_OF_DAY)
        val period = when (hour) {
            in 5..10 -> "morning"
            in 11..16 -> "day"
            in 17..21 -> "evening"
            else -> "night"
        }
        val unread = agent.unread
        val notes = if (Look.hides("notifications")) 0 else dev.r1os.host.Notifications.all().size
        val news = unread + notes
        val now = android.os.SystemClock.elapsedRealtime()
        val due = period != Greeting.period || news > Greeting.news || now - Greeting.at > GREET_EVERY_MS
        Greeting.news = news
        if (!due) return
        Greeting.at = now
        Greeting.period = period
        bubble.text = when {
            unread > 1 -> "$unread new from me"
            unread == 1 -> "i've got a reply for you"
            notes > 1 -> "$notes notifications for you"
            notes == 1 -> "a notification for you"
            period == "morning" -> "morning!"
            period == "day" -> "hey there"
            period == "evening" -> "evening!"
            else -> "still up?"
        }
        placeBubble()
        bubble.visibility = VISIBLE
        bubble.alpha = 0f
        bubble.translationY = context.dp(8f)
        bubble.animate().alpha(1f).translationY(0f).setDuration(300).setInterpolator(Theme.EASE_OUT).start()
        // It speaks where the page's dots are: they step aside meanwhile.
        dots.animate().alpha(0f).setDuration(200).start()
        removeCallbacks(unbubble)
        postDelayed(unbubble, BUBBLE_MS)
    }

    /**
     * The greeting beside the agent, not over its head (Max, 2026-10-13: "Left corner agents should have the greeting
     * to the right of them, center should be to the right of them, right cornered should be to the left of them"): level
     * with its face as far as it peeks now (it sinks for a tall card), clear of its body, wrapping before the screen's
     * edge.
     */
    private fun placeBubble() {
        val lp = bubble.layoutParams as LayoutParams
        val dp = context.dp(1f)
        val box = CHAR_DP * dp
        val boxLeft = when (Look.agentPlace) {
            "left" -> -CORNER_DP * dp
            "right" -> width - box + CORNER_DP * dp
            else -> (width - box) / 2f
        }
        val gap = BUBBLE_GAP_DP * dp
        val edge = context.dp(12f)
        if (Look.agentPlace == "right") {
            val bodyLeft = boxLeft + box * BODY_SIDE
            lp.gravity = Gravity.BOTTOM or Gravity.END
            lp.leftMargin = 0
            lp.rightMargin = (width - bodyLeft + gap).toInt()
            bubble.maxWidth = (bodyLeft - gap - edge).toInt().coerceAtLeast(context.dpi(80f))
        } else {
            val bodyRight = boxLeft + box * (1 - BODY_SIDE)
            lp.gravity = Gravity.BOTTOM or Gravity.START
            lp.rightMargin = 0
            lp.leftMargin = (bodyRight + gap).toInt()
            bubble.maxWidth = (width - bodyRight - gap - edge).toInt().coerceAtLeast(context.dpi(80f))
        }
        // Its face is a little under halfway down its box, whose top is [peek] above the bottom edge.
        lp.bottomMargin = context.dpi((peek - CHAR_DP * FACE_AT - BUBBLE_HALF_DP).coerceAtLeast(10f))
        bubble.layoutParams = lp
    }

    /** A widget: its card as a chat draws it, or (no card yet) its name and why. Touching it opens its app. */
    private fun widgetView(w: JSONObject): View {
        val accent = Theme.colorOf(w.optString("accent").ifEmpty { null })
        val card = w.optJSONObject("card")
        val v = if (card != null) cardView(context, card, accent) else LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(14f), context.dpi(14f), context.dpi(14f), context.dpi(14f))
            background = roundRect(Theme.CARD, context.dp(18f))
            addView(ImageView(context).apply { setImageDrawable(Icons.drawable(w.optString("icon").ifEmpty { "app-window" }, accent)) },
                LinearLayout.LayoutParams(context.dpi(22f), context.dpi(22f)).apply { rightMargin = context.dpi(12f) })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(context.text(16f, 700).keepCase().apply { text = w.optString("name") })
                addView(context.text(13f, 600, Theme.MUTED).apply { text = w.optString("note") })
            })
        }
        val id = w.optString("app")
        v.setOnClickListener {
            Sounds.play(Sounds.S.OPEN)
            complete { home.shell.open(Target.of("app", "id" to id)) }
        }
        return v
    }

    /**
     * Room for the page (Max, 2026-10-11: widgets "dont get cut off by the agent"): the card keeps its whole height
     * under the time (and the notifications), and the agent sinks only as far as that needs, out of sight if it must.
     * Its head never covers the card. Without a page it peeks as usual. Run when the page, the notifications or the
     * clock's height change.
     */
    private fun fitPage(animate: Boolean = false) {
        if (morph != null || height == 0 || dock) return
        val want = if (pages() == 0) PEEK_DP else {
            val lp = middle.layoutParams as LayoutParams
            val dotsH = if (dots.visibility == VISIBLE) context.dpi(14f) else 0
            val end = lp.topMargin + pageBox.natural + dotsH
            // What's left under it for the agent (dp), and the gap; its view's empty top may sit over the card's end.
            val room = (height - end) / context.dp(1f) - MIDDLE_GAP_DP
            (room + BODY_INSET_DP).coerceIn(0f, PEEK_WIDGETS_DP)
        }
        setPeek(want, animate)
    }

    private fun setPeek(to: Float, animate: Boolean) {
        if (to == peek && peekAnim == null) return
        peekAnim?.cancel()
        val from = peek
        fun place(p: Float) {
            peek = p
            (character.layoutParams as LayoutParams).bottomMargin = -context.dpi(CHAR_DP - p)
            (middle.layoutParams as? LayoutParams)?.bottomMargin = context.dpi(maxOf(p - BODY_INSET_DP, 0f) + MIDDLE_GAP_DP)
            requestLayout()
        }
        if (!animate || !displayOn) {
            peekAnim = null
            place(to)
            return
        }
        peekAnim = android.animation.ValueAnimator.ofFloat(from, to).apply {
            duration = 260
            interpolator = Theme.EASE_OUT
            addUpdateListener { place(it.animatedValue as Float) }
            addListener(object : android.animation.AnimatorListenerAdapter() {
                override fun onAnimationEnd(a: android.animation.Animator) { peekAnim = null }
            })
            start()
        }
    }

    /**
     * A reminder you haven't seen, waiting on the glance screen (T-123): an agent's (`reminder`, its id in r1d's alarms;
     * `title` is the agent's name) or an app's (`notice`, its id in r1d's notices; `app` opens). `at`: when it went off.
     */
    class Waiting(
        val kind: String, val id: String, val agent: String?, val app: String?,
        val title: String, val text: String, val icon: String, val color: Int, val at: Long,
    )
    private var waiting: List<Waiting> = emptyList()

    /** The reminders you haven't seen (the shell's, from r1d), newest first. */
    fun setWaiting(w: List<Waiting>) {
        val asked = waiting.filter { it.kind == "approval" }.map { it.id }.toSet()
        waiting = w
        // An agent waiting on you (T-160; Max, 2026-10-13: "Make sure those checkout cards take priority on glance"): the
        // notifications' page comes up for a new one, whatever page was showing.
        if (w.any { it.kind == "approval" && it.id !in asked }) {
            want = null
            page = 0
        }
        showNotes()
    }

    /** What's waiting changed (a notification came or went, a reminder was seen): the first page follows. */
    private fun showNotes() {
        val shown = keyOf(page)
        val had = noteList.isNotEmpty()
        noteList = if (Look.hides("notifications")) emptyList() else {
            val android = dev.r1os.host.Notifications.all().map { n ->
                Note("n:" + n.key, n.icon() ?: Icons.drawable("bell", Theme.MUTED), n.title.ifEmpty { n.app }, n.text, n.at,
                    open = { home.shell.openNote(n.key) }, gone = { dev.r1os.host.Notifications.dismiss(n.key) })
            }
            val reminders = waiting.map { w ->
                Note("w:" + w.kind + ":" + w.id, Icons.drawable(w.icon, w.color), w.title, w.text, w.at,
                    open = { home.shell.openWaiting(w) }, gone = { home.shell.seenWaiting(w, open = false) })
            }
            // Agents waiting on you first (T-160), then the newest.
            (android + reminders).sortedWith(compareByDescending<Note> { it.key.startsWith("w:approval:") }.thenByDescending { it.at })
        }
        // Asleep, it'll wake to them; else the page you're on stays yours as the pages shift.
        if (noteList.isNotEmpty() && (!had && !displayOn)) {
            want = null
            page = 0
        } else if (shown != null) {
            (0 until pages()).firstOrNull { keyOf(it) == shown }?.let { page = it }
        }
        if (morph == null && !locking) showPage()
    }

    /** The notifications' page: the newest few, and how many more (a touch lists them all). */
    private fun notesView(): View = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(4f), context.dpi(6f), context.dpi(4f), context.dpi(6f))
        background = roundRect(Theme.CARD, context.dp(20f))
        for (n in noteList.take(MAX_NOTES)) {
            addView(noteRow(n), LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        }
        val more = noteList.size - MAX_NOTES
        if (more > 0) addView(context.text(13f, 650, Theme.MUTED).apply {
            text = "+$more more"
            setPadding(context.dpi(14f), context.dpi(6f), context.dpi(14f), context.dpi(4f))
            setOnClickListener {
                Sounds.play(Sounds.S.OPEN)
                complete { home.shell.push(NotificationsScreen(home.shell)) }
            }
        })
    }

    /** One notification: a touch opens it, a swipe across puts it away (T-129). */
    private fun noteRow(n: Note): View {
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(12f), context.dpi(8f), context.dpi(12f), context.dpi(8f))
            addView(ImageView(context).apply { setImageDrawable(n.icon) },
                LinearLayout.LayoutParams(context.dpi(24f), context.dpi(24f)).apply { rightMargin = context.dpi(12f) })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(context.text(15f, 700).keepCase().apply {
                    text = n.title
                    maxLines = 1
                    ellipsize = android.text.TextUtils.TruncateAt.END
                })
                if (n.text.isNotBlank()) addView(context.text(13f, 600, Theme.MUTED).keepCase().apply {
                    text = n.text
                    maxLines = 2
                    ellipsize = android.text.TextUtils.TruncateAt.END
                })
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        }
        swipeAway(row, onTap = {
            Sounds.play(Sounds.S.OPEN)
            complete { n.open() }
        }) {
            // Gone from the page at once; Android or r1d catch up.
            noteList = noteList.filter { it.key != n.key }
            n.gone()
            showPage()
        }
        return row
    }

    /**
     * A row that a swipe across puts away (T-129): it follows the finger, fading, and past about a third of its width
     * (or a flick) slides off and [onGone]; else it springs back. A touch that doesn't move is [onTap]. Docked, it keeps
     * the touch from the card's own swipe, which turns the page.
     */
    @SuppressLint("ClickableViewAccessibility")
    private fun swipeAway(v: View, onTap: () -> Unit, onGone: () -> Unit) {
        val slop = android.view.ViewConfiguration.get(context).scaledTouchSlop
        var x0 = 0f
        var y0 = 0f
        var dragging = false
        var moved = false
        var vt: android.view.VelocityTracker? = null
        v.setOnTouchListener { _, e ->
            when (e.actionMasked) {
                android.view.MotionEvent.ACTION_DOWN -> {
                    x0 = e.rawX
                    y0 = e.rawY
                    dragging = false
                    moved = false
                    vt = android.view.VelocityTracker.obtain().also { it.addMovement(e) }
                    v.parent?.requestDisallowInterceptTouchEvent(true)
                }
                android.view.MotionEvent.ACTION_MOVE -> {
                    vt?.addMovement(e)
                    val dx = e.rawX - x0
                    val dy = e.rawY - y0
                    if (Math.abs(dx) > slop || Math.abs(dy) > slop) moved = true
                    if (!dragging && Math.abs(dx) > slop && Math.abs(dx) > Math.abs(dy)) dragging = true
                    if (dragging) {
                        v.translationX = dx
                        v.alpha = 1f - (Math.abs(dx) / maxOf(v.width, 1)).coerceIn(0f, 1f) * 0.8f
                    }
                }
                android.view.MotionEvent.ACTION_UP -> {
                    val dx = e.rawX - x0
                    vt?.addMovement(e)
                    vt?.computeCurrentVelocity(1000)
                    val vx = vt?.xVelocity ?: 0f
                    vt?.recycle()
                    vt = null
                    when {
                        dragging && (Math.abs(dx) > v.width * 0.35f || Math.abs(vx) > context.dp(FLING_DP)) -> {
                            val side = if ((if (Math.abs(dx) > slop) dx else vx) > 0) 1 else -1
                            Sounds.play(Sounds.S.CLOSE)
                            v.animate().translationX(side * v.width.toFloat()).alpha(0f).setDuration(180)
                                .setInterpolator(Theme.EASE_IN_OUT).withEndAction { onGone() }.start()
                        }
                        dragging -> v.animate().translationX(0f).alpha(1f).setDuration(220).setInterpolator(Theme.SETTLE).start()
                        !moved -> onTap()
                    }
                }
                android.view.MotionEvent.ACTION_CANCEL -> {
                    vt?.recycle()
                    vt = null
                    v.animate().translationX(0f).alpha(1f).setDuration(200).start()
                }
            }
            true
        }
    }

    /** A round touch button around an icon. */
    private fun control(icon: ImageView, onTap: () -> Unit) = FrameLayout(context).apply {
        background = roundRect(Theme.FOCUS, context.dp(999f))
        addView(icon, LayoutParams(context.dpi(22f), context.dpi(22f), Gravity.CENTER))
        setOnClickListener {
            dev.r1os.host.ui.Sounds.play(dev.r1os.host.ui.Sounds.S.OPEN)
            onTap()
        }
    }

    /** r1os's own player first; else what another app plays (Spotify, with image v0.2.12's permission). */
    private fun refreshPlaying() {
        val was = playing
        val own = dev.r1os.host.Player.now
        val other = dev.r1os.host.MediaSessions.now
        playing = when {
            Look.hides("playing") -> false
            own.state != "stopped" -> { showPlaying(own); true }
            other != null -> {
                npTitle.text = other.title
                npSub.text = listOf(other.subtitle, other.app, if (other.playing) "" else "paused").filter { it.isNotBlank() }.joinToString(" · ")
                npToggle.setImageDrawable(Icons.drawable(if (other.playing) "pause" else "play", Theme.FG))
                true
            }
            else -> false
        }
        // It starts or stops: the pages shift, and the player's page shows first (while you look: T-125).
        if (playing != was) {
            if (displayOn && up) want = null
            page = notePages()
            showPage()
        }
    }

    private fun showPlaying(n: dev.r1os.host.Player.Now) {
        npTitle.text = n.song.ifEmpty { n.title.ifEmpty { "Playing" } }
        npSub.text = when (n.state) {
            "loading" -> "loading…"
            "error" -> "couldn't play it"
            else -> listOf(if (n.song.isNotEmpty()) n.title else n.subtitle, if (n.state == "paused") "paused" else "")
                .filter { it.isNotBlank() }.joinToString(" · ")
        }
        val playing = n.state == "playing" || n.state == "loading"
        npToggle.setImageDrawable(Icons.drawable(if (playing) "pause" else "play", Theme.FG))
    }

    /** Its background's flow (ADR-0043): the wheel and the button nudge a gradient's glows. */
    fun nudge(delta: Int) {
        scrim.nudge(delta)
        // Once you're turning, the greeting has done its job.
        if (bubble.visibility == VISIBLE) {
            removeCallbacks(unbubble)
            unbubble.run()
        }
    }
    fun press(down: Boolean) = scrim.press(down)

    /** It's up, over home. */
    var up = false
        private set
    /** Where its time, date and status end (px from its top): a reminder shows below them, not over the clock. */
    fun infoBottom(): Int = (device.parent as? View)?.bottom ?: 0

    /** The agent it shows: the one hold-to-talk goes to (a reply's, else the shell's). */
    var agent: Agent = Agent.DEFAULT
        private set
    /** The shell's: the one you were last with. */
    private var fallback: Agent = Agent.DEFAULT
    /** Leaving to read this reply: its chat opens on home. */
    private var leavingTo: Reply? = null
    /** Ticks turned up, short of opening. */
    var pull = 0
        private set
    /** Home is on screen: the morph finished. */
    var onOpened: () -> Unit = {}

    private var displayOn = false
    /** How far it is from the glance screen (0) to home (1): the wheel moves its target. */
    private val spring = Spring { apply(it) }
    /** The turn got there: it's gliding home, and nothing takes it back. */
    private var committed = false
    private var morph: Morph? = null
    private val relax = Runnable { if (locking) relaxLock() else letGo() }

    /** Home is being pulled back into the glance screen (going to sleep). */
    var locking = false
        private set
    /** The pull to sleep finished: the glance screen is up at rest and the display can go off. */
    var onLocked: () -> Unit = {}
    private val lockFailsafe = Runnable { if (up && locking && committed) locked() }

    // ---- the PIN lock (ADR-0016) ----------------------------------------------------------------------

    /**
     * Locked (r1d's word, set by the shell): the way home is the dial. Turning up, a double click, or a touch that would
     * leave (a widget, the notifications) opens it; what the touch wanted happens once the code's right.
     */
    var locked = false
        set(v) {
            val was = field
            field = v
            lockIcon.visibility = if (v && !Look.hides("status")) VISIBLE else GONE
            if (was != v && up && morph == null && replyOn() != null) showPage()
        }
    /** A code dialled: the shell asks r1d, then calls [unlockedNow] or [dialWrong]. */
    var onCode: (String) -> Unit = {}
    private val dial = PinDial(context).apply { visibility = GONE }
    private var afterUnlock: (() -> Unit)? = null
    private val dialIdle = Runnable { closeDial() }
    val dialOpen: Boolean get() = dial.visibility == VISIBLE

    /** The dial comes up where the agent peeks (it ducks out of the way); `then` runs once it's unlocked. */
    fun openDial(then: (() -> Unit)? = null) {
        if (then != null) afterUnlock = then
        removeCallbacks(dialIdle)
        postDelayed(dialIdle, DIAL_IDLE_MS)
        if (dialOpen) return
        removeCallbacks(greetNow)
        removeCallbacks(unbubble)
        bubble.visibility = GONE
        dial.reset("dial your pin")
        dial.visibility = VISIBLE
        dial.alpha = 0f
        dial.translationY = context.dp(36f)
        dial.animate().cancel()
        dial.animate().alpha(1f).translationY(0f).setDuration(280).setInterpolator(Theme.POP).start()
        rising?.cancel()
        character.animate().cancel()
        character.animate().translationY(context.dp(CHAR_DP)).setDuration(220).setInterpolator(Theme.EASE_OUT).start()
        middle.animate().cancel()
        middle.animate().alpha(0f).setDuration(200).start()
        Sounds.play(Sounds.S.OPEN)
    }

    /** Put away (a double click on the first reel, or a pause): the agent peeks up again. */
    fun closeDial() {
        removeCallbacks(dialIdle)
        if (!dialOpen) return
        afterUnlock = null
        dial.animate().cancel()
        dial.animate().alpha(0f).translationY(context.dp(36f)).setDuration(180).withEndAction { dial.visibility = GONE }.start()
        character.animate().cancel()
        character.animate().translationY(0f).setDuration(380).setInterpolator(OvershootInterpolator(1.4f)).start()
        middle.animate().cancel()
        middle.animate().alpha(1f).setDuration(220).start()
    }

    /** The wheel and the button while the dial's open. */
    fun dialInput(ev: dev.r1os.host.net.Ev): Boolean {
        removeCallbacks(dialIdle)
        postDelayed(dialIdle, DIAL_IDLE_MS)
        return dial.input(ev)
    }

    /** Not the code: r1d's words ("wrong code", "too many tries · wait 2 min"). */
    fun dialWrong(why: String) = dial.wrong(why)

    /** The code was right: home (the morph), then what was waiting for it. */
    fun unlockedNow() {
        locked = false
        removeCallbacks(dialIdle)
        val then = afterUnlock
        afterUnlock = null
        if (!up) return
        dial.animate().cancel()
        dial.animate().alpha(0f).setDuration(160).withEndAction { dial.visibility = GONE }.start()
        middle.animate().cancel()
        complete(then)
    }

    /** Down at once (the display went off). */
    private fun dropDial() {
        removeCallbacks(dialIdle)
        afterUnlock = null
        dial.animate().cancel()
        dial.visibility = GONE
        middle.animate().cancel()
        middle.alpha = 1f
    }

    /** Where each piece goes, measured when a morph starts. */
    private class Morph(
        val clockDx: Float, val clockDy: Float, val clockScale: Float,
        val dateDy: Float,
        val deviceDx: Float, val deviceDy: Float,
        val charDx: Float, val charDy: Float, val charScale: Float,
    )

    init {
        isClickable = true // swallows touches
        clipChildren = false
        visibility = INVISIBLE // laid out even when hidden: a pull from home measures it at once
        addView(scrim, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        // The look (ADR-0043): the time on the left (ADR-0020) or centred, and what's left out.
        val centred = Look.align == "center"
        if (Look.hides("date")) date.visibility = GONE
        if (Look.hides("status")) listOf(battery, bolt, wifi).forEach { it.visibility = GONE }
        if (Look.hides("status") && Look.hides("alarm")) device.visibility = GONE
        val info = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            clipChildren = false
            if (centred) gravity = Gravity.CENTER_HORIZONTAL
            // Its own width: a vertical layout's default (match parent) would clip it to its siblings'.
            addView(clock, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT))
            addView(date, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
            addView(device, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
        }
        addView(info, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.TOP or (if (centred) Gravity.CENTER_HORIZONTAL else Gravity.START)).apply {
            if (!centred) leftMargin = context.dpi(20f)
            topMargin = context.dpi(22f)
        })
        // The middle starts under the time and status, however tall the clock's face is (a dial, two lines).
        info.addOnLayoutChangeListener { _, _, _, _, bottom, _, _, _, _ ->
            if (dock) return@addOnLayoutChangeListener
            val top = maxOf(context.dpi(NOTES_TOP_DP), bottom + context.dpi(12f))
            val lp = middle.layoutParams as? LayoutParams ?: return@addOnLayoutChangeListener
            if (lp.topMargin != top) {
                lp.topMargin = top
                middle.post { middle.requestLayout(); fitPage() }
            }
        }
        // Peeking up over the bottom edge: its face and most of its body show. In the middle, or from a corner (the
        // look's); hidden, it only shows as the morph lands it on home.
        val place = Look.agentPlace
        addView(character, LayoutParams(context.dpi(CHAR_DP), context.dpi(CHAR_DP), Gravity.BOTTOM or when (place) {
            "left" -> Gravity.START
            "right" -> Gravity.END
            else -> Gravity.CENTER_HORIZONTAL
        }).apply {
            bottomMargin = -context.dpi(CHAR_DP - PEEK_DP)
            if (place == "left") leftMargin = -context.dpi(CORNER_DP)
            if (place == "right") rightMargin = -context.dpi(CORNER_DP)
        })
        if (place == "hidden") character.alpha = 0f
        // Touch: a tap opens your chat with it (T-122, ADR-0059), a stroke across pets it (ADR-0043).
        character.pokeable(onTap = { openAgent(agent) })
        // Its greeting, beside it ([placeBubble] puts it there as it speaks).
        addView(bubble, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or when (place) {
            "left" -> Gravity.START
            "right" -> Gravity.END
            else -> Gravity.CENTER_HORIZONTAL
        }).apply {
            leftMargin = context.dpi(18f); rightMargin = context.dpi(18f)
            bottomMargin = context.dpi(PEEK_DP + 10f)
        })
        // The PIN's dial (ADR-0016), over where the agent peeks.
        addView(dial, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.BOTTOM).apply {
            bottomMargin = context.dpi(22f)
        })
        dial.onCode = { onCode(it) }
        dial.onOut = { closeDial() }
        // Something landed while you look: it perks up.
        home.shell.keep(dev.r1os.host.Notifications.onPosted {
            if (up && displayOn && morph == null && !Look.hides("notifications")) {
                character.react("surprised")
                // At rest, it's back for it (T-157).
                dev.r1os.host.ui.Dim.poke()
            }
        })
        // Under the time, the date and the device's status: the notifications, then the page (what's playing, or a
        // widget) and its dots, down to where the agent peeks up. A card taller than that is cut off.
        middle.addView(pageBox, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        middle.addView(dots, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(6f)).apply { topMargin = context.dpi(8f) })
        addView(middle, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT).apply {
            leftMargin = context.dpi(14f); rightMargin = context.dpi(14f); topMargin = context.dpi(NOTES_TOP_DP)
            bottomMargin = context.dpi(PEEK_DP + MIDDLE_GAP_DP)
        })
        addView(dockRow, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.BOTTOM))
        showNotes()
        home.shell.keep(dev.r1os.host.Notifications.onChange { showNotes() })
        refreshPlaying()
        home.shell.keep(dev.r1os.host.Player.onChange { refreshPlaying() })
        refreshWork()
        home.shell.keep(dev.r1os.host.AgentApps.onChange { refreshWork() })
        home.shell.keep(dev.r1os.host.AgentBrowser.onChange { refreshWork() })
        dev.r1os.host.MediaSessions.start(context) { refreshPlaying() }
    }

    // ---- showing ------------------------------------------------------------------------------------

    /** Up at once: the display is off (or going off), so there's nothing to animate yet. */
    fun show(a: Agent) {
        removeCallbacks(relax)
        removeCallbacks(failsafe)
        removeCallbacks(lockFailsafe)
        afterOpen = null
        locking = false
        if (morph != null) home.setGlanceHidden(false)
        morph = null
        pull = 0
        committed = false
        spring.snapTo(0f)
        dropDial()
        up = true
        visibility = VISIBLE
        dropped = false
        translationY = 0f
        // The docked four come up with it (docked from home, they'd been told it wasn't up, and stayed still).
        dockCast.forEach { it.setActive(displayOn && dock) }
        // It wakes to the page you left it on (T-125), else the first: the newest reply, what's playing, or a widget.
        page = 0
        // What's waiting comes first (T-130), then an agent at work (T-151); else the page you left it on.
        want = if (noteList.isNotEmpty() || work.isNotEmpty()) null else context.getSharedPreferences("r1os", Context.MODE_PRIVATE).getString(LAST_PAGE, null)
        animate().cancel()
        alpha = 1f
        fallback = a
        showPage()
        loadReplies()
        character.translationY = context.dp(RISE_DP) // it rises when the display comes on
    }

    /** Down at once, with no morph: setup started under it (r1d was reset while the R1 slept). */
    fun dismiss() {
        if (!up) return
        removeCallbacks(relax)
        removeCallbacks(failsafe)
        removeCallbacks(lockFailsafe)
        afterOpen = null
        spring.cancel()
        spring.snapTo(0f)
        if (morph != null) home.setGlanceHidden(false)
        morph = null
        up = false
        pull = 0
        committed = false
        locking = false
        visibility = INVISIBLE
        character.setActive(false)
    }

    /** The shell's agent (the one you were last with), or its state changed: it shows unless a reply's agent does. */
    fun setAgent(a: Agent) {
        fallback = a
        showAgent()
    }

    /** A reply's agent while folding from home it isn't: home's own, so the fold matches. */
    private fun shownAgent(): Agent {
        if (locking) return fallback
        // An agent at work on the page showing peeks up.
        workOn()?.let { w -> home.shell.agent(w.agent)?.let { return it } }
        val r = replyOn() ?: replies.firstOrNull()
        return r?.let { home.shell.agent(it.agent) } ?: fallback
    }

    /** Draws the agent it shows; another one swaps in with a dip while you watch. */
    // ---- docked on its side (T-103) ------------------------------------------------------------------

    /**
     * Docked and turned on its side (Max, 2026-10-12: "display up to 4 agents across the bottom of the screen peeking,
     * with the clock in the top left and notifications/widgets in the top right"): the time stays where it is, the
     * middle (notifications, the page, its dots) moves to the right, and your first four agents peek along the bottom
     * in place of the one. The shell turns the screen ([dev.r1os.host.ime.Rotation.dock]).
     */
    fun setDock(on: Boolean) {
        if (on == dock) return
        dock = on
        val lp = middle.layoutParams as LayoutParams
        if (on) {
            lp.leftMargin = context.dpi(DOCK_SPLIT_DP)
            lp.topMargin = context.dpi(18f)
            lp.bottomMargin = context.dpi(DOCK_PEEK_DP + 10f)
        } else {
            lp.leftMargin = context.dpi(14f)
            lp.topMargin = maxOf(context.dpi(NOTES_TOP_DP), infoBottom() + context.dpi(12f))
        }
        middle.layoutParams = lp
        character.visibility = if (on) GONE else VISIBLE
        bubble.visibility = GONE
        dockRow.visibility = if (on) VISIBLE else GONE
        // Docked, a swipe across the card turns the pages too (T-117); upright, touches are the cards' own.
        pageBox.onSwipe = if (on) { d -> turnPage(d) } else null
        if (on) dockAgents() else dockCast.forEach { it.setActive(false) }
        pickWidgets()
        showPage()
        requestLayout()
        post { fitPage() }
    }

    /** Your first four agents, in home's order, along the bottom while it's docked on its side. */
    fun dockAgents() {
        if (!dock) return
        val list = home.shell.agents.take(4)
        while (dockCast.size < list.size) {
            val cv = CharacterView(context, context.dpi(DOCK_CHAR_DP))
            dockCast += cv
            dockRow.addView(cv, LinearLayout.LayoutParams(context.dpi(DOCK_CHAR_DP), context.dpi(DOCK_CHAR_DP)).apply {
                bottomMargin = -context.dpi(DOCK_CHAR_DP - DOCK_PEEK_DP)
            })
        }
        while (dockCast.size > list.size) dockRow.removeView(dockCast.removeAt(dockCast.size - 1))
        for ((cv, a) in dockCast.zip(list)) {
            cv.set(a.form, a.color, a.finish, if (a.state == "error") "idle" else a.state, CharacterView.activityOf(a.tool), a.unread, eyes = a.eyes, wear = a.wear)
            cv.setActive(displayOn && up)
            // A tap opens your chat with that one (T-122).
            cv.pokeable(onTap = { openAgent(a) })
        }
    }

    private fun showAgent() {
        val a = shownAgent()
        val swap = a.id != agent.id && displayOn && up && !locking && morph == null && rising?.isRunning != true
        agent = a
        val draw = { character.set(a.form, a.color, a.finish, if (a.state == "error") "idle" else a.state, CharacterView.activityOf(a.tool), a.unread, eyes = a.eyes, wear = a.wear) }
        if (!swap) { draw(); return }
        character.animate().cancel()
        character.animate().translationY(context.dp(SWAP_DP)).setDuration(140).setInterpolator(Theme.EASE_OUT).withEndAction {
            draw()
            character.animate().translationY(0f).setDuration(340).setInterpolator(OvershootInterpolator(1.6f)).start()
        }.start()
    }

    /** The display came on or went off: it animates only while it's on. On wake, the agent rises. */
    fun setDisplayOn(on: Boolean) {
        displayOn = on
        // Waking: what's waiting first (T-130).
        if (on && up && morph == null && !dropped && noteList.isNotEmpty() && page != 0) {
            want = null
            page = 0
            showPage()
        }
        character.setActive(on && up && morph == null)
        dockCast.forEach { it.setActive(on && up && dock) }
        if (on && up && morph == null && Look.agentPlace != "hidden") {
            rise()
            // Idle life (ADR-0043): a greeting once it's up; late at night, a yawn.
            postDelayed(greetNow, GREET_DELAY_MS)
            val hour = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
            if (hour >= 23 || hour < 5) postDelayed(yawnNow, YAWN_DELAY_MS)
        }
        if (!on) {
            dropDial()
            removeCallbacks(greetNow)
            removeCallbacks(yawnNow)
            removeCallbacks(unbubble)
            bubble.visibility = GONE
            dots.alpha = 1f
        }
    }

    /**
     * The agent hops up into place as the display comes on: it rises past its spot stretched tall,
     * squashes as it lands, and wobbles to rest. All transform properties, on the render thread.
     */
    private fun rise() {
        rising?.cancel()
        character.animate().cancel()
        character.pivotX = character.width / 2f
        character.pivotY = character.height.toFloat() // squash from its feet
        val d = context.dp(RISE_DP)
        fun kf(prop: android.util.Property<View, Float>, vararg v: Pair<Float, Float>) =
            android.animation.PropertyValuesHolder.ofKeyframe(prop, *v.map { (t, x) -> android.animation.Keyframe.ofFloat(t, x) }.toTypedArray())
        rising = android.animation.ObjectAnimator.ofPropertyValuesHolder(character,
            kf(View.TRANSLATION_Y, 0f to d, 0.42f to -context.dp(18f), 0.68f to context.dp(5f), 0.86f to -context.dp(2f), 1f to 0f),
            kf(View.SCALE_Y, 0f to 0.86f, 0.38f to 1.1f, 0.6f to 0.9f, 0.8f to 1.03f, 1f to 1f),
            kf(View.SCALE_X, 0f to 1.08f, 0.38f to 0.93f, 0.6f to 1.08f, 0.8f to 0.98f, 1f to 1f),
            kf(View.ROTATION, 0f to -5f, 0.4f to 4f, 0.65f to -2.5f, 0.85f to 1f, 1f to 0f),
        ).apply {
            duration = 900
            startDelay = 90
            interpolator = Theme.EASE_IN_OUT
            start()
        }
    }

    private var rising: android.animation.Animator? = null

    // ---- the morph to home --------------------------------------------------------------------------

    /** One tick up: the spring's target moves on, less each tick; the third commits it home. */
    fun pullUp() {
        if (committed) return
        if (dropped) { dropAway(); return }
        if (locked) { openDial(); return }
        removeCallbacks(relax)
        if (!begin()) { finish(); return }
        pull++
        if (pull >= HomeScreen.PULL_TICKS) { commit(); return }
        Sounds.play(if (pull == 1) Sounds.S.PULL1 else Sounds.S.PULL2)
        spring.animateTo(STEPS[pull - 1], PULL_STIFFNESS, PULL_DAMPING)
        postDelayed(relax, RELAX_MS)
    }

    /** One tick down: back a step (at the start, a little nudge). */
    fun pullDown() {
        if (committed) return
        removeCallbacks(relax)
        if (pull == 0) { if (pages() > 1) nextPage() else nudge(); return }
        pull--
        if (pull == 0) letGo() else {
            spring.animateTo(STEPS[pull - 1], PULL_STIFFNESS, PULL_DAMPING)
            postDelayed(relax, RELAX_MS)
        }
    }

    /** All the way to home (double click, or words to send), then `then`. */
    fun complete(then: (() -> Unit)? = null) {
        if (dropped) { dropAway(then); return }
        if (locked && up) { openDial(then); return }
        // Something to open (a card, a chat, a notification, words you said): straight to it, not home first (T-128).
        if (then != null && !committed && morph == null && up && !locking) { leaveTo(then); return }
        removeCallbacks(relax)
        if (then != null) afterOpen = then
        if (committed) return
        if (!begin()) { finish(); return }
        commit()
    }

    /**
     * Straight to what you touched (T-128, Max: "Opening things from the glance screen should open directly to those
     * rather than going home first"): it opens over home, under this, and this fades away; [onLeft] then.
     */
    private fun leaveTo(then: () -> Unit) {
        removeCallbacks(relax)
        afterOpen = null
        leavingTo = null
        pull = 0
        spring.snapTo(0f)
        then()
        animate().cancel()
        animate().alpha(0f).setDuration(LEAVE_MS).setInterpolator(Theme.EASE_IN_OUT).withEndAction {
            alpha = 1f
            if (up && !dropped) {
                dismiss()
                onLeft()
            }
        }.start()
    }

    /** It went, straight to something (T-128): the shell's turn (Android's keyguard, docked mode). */
    var onLeft: () -> Unit = {}

    /** What to do once home is on screen (open the chat the words went to). Runs once. */
    private var afterOpen: (() -> Unit)? = null
    /** A commit always ends on home, even if the spring never reports settling. */
    private val failsafe = Runnable { if (up && committed) finish() }

    /**
     * Home it goes: the spring carries on from wherever (and however fast) it is, critically damped,
     * so it settles exactly on home's clock and agent with no overshoot to snap back from.
     */
    private fun commit() {
        committed = true
        pull = 0
        removeCallbacks(relax)
        Sounds.play(Sounds.S.OPEN)
        spring.animateTo(1f, COMMIT_STIFFNESS, 1f) { finish() }
        removeCallbacks(failsafe)
        postDelayed(failsafe, 1200)
    }

    /** Back to the glance screen, smoothly. */
    private fun letGo() {
        removeCallbacks(relax)
        if (committed) return
        pull = 0
        spring.animateTo(0f, RELAX_STIFFNESS, 1f) {
            morph = null
            home.setGlanceHidden(false)
            character.setActive(displayOn && up)
        }
    }

    // ---- pulled down from the top edge, over whatever's on screen (T-124) --------------------------

    /**
     * Pulled down over another screen (T-124, Max: "swiping down from the top of the screen to show the glance view at
     * any time … closer functionally to swiping down on an iOS device"): the same glance screen, sliding down with the
     * finger. Leaving it (a swipe up, turning up, a double click, or a touch that opens something) slides it back up to
     * that screen, not home; what the touch opens goes on top of it.
     */
    var dropped = false
        private set
    /** Settled open (true), or gone back up (false): the screen under it stops, and starts again (the shell's). */
    var onDrop: (Boolean) -> Unit = {}
    private var dropOpen = false

    /** The finger's at the top edge, pulling: up from above the screen, with agent `a`. */
    fun dropBegin(a: Agent) {
        show(a)
        dropped = true
        dropOpen = false
        animate().cancel()
        translationY = -height.toFloat()
        character.translationY = 0f
    }

    /** `dy` px down from the top edge. */
    fun dropTo(dy: Float) {
        if (!dropped) return
        translationY = (dy - height).coerceIn(-height.toFloat(), 0f)
    }

    /** Let go: settled open, or back up when it wasn't pulled far enough. */
    fun dropSettle(open: Boolean) {
        if (!dropped) return
        if (!open) { dropAway(); return }
        animate().cancel()
        animate().translationY(0f).setDuration(260).setInterpolator(Theme.SETTLE).withEndAction {
            if (!dropped || dropOpen) return@withEndAction
            dropOpen = true
            onDrop(true)
            setDisplayOn(displayOn)
        }.start()
    }

    /** Back up, to the screen it covered, then `then`. */
    fun dropAway(then: (() -> Unit)? = null) {
        if (!dropped) { then?.invoke(); return }
        animate().cancel()
        Sounds.play(Sounds.S.CLOSE)
        animate().translationY(-height.toFloat()).setDuration(220).setInterpolator(Theme.EASE_IN_OUT).withEndAction {
            if (!dropped) return@withEndAction
            dropped = false
            dismiss()
            translationY = 0f
            if (dropOpen) onDrop(false)
            dropOpen = false
            then?.invoke()
        }.start()
    }

    // ---- the morph back: home to the glance screen, to sleep (ADR-0022) -----------------------------

    /**
     * One tick up past the first agent: home folds back toward the glance screen, against the same
     * resistance as unlocking. The first tick puts this up over home looking exactly like it; the
     * third finishes the fold and [onLocked] puts the R1 to sleep.
     */
    fun lockTick(a: Agent) {
        if (committed) return
        removeCallbacks(relax)
        if (!locking) {
            if (up) return
            up = true
            locking = true
            // Home's own agent (not a reply's), so the fold matches what's on screen.
            setAgent(a)
            rising?.cancel()
            character.animate().cancel()
            character.translationY = 0f
            if (!begin()) {
                up = false
                locking = false
                return
            }
            spring.snapTo(1f) // every piece on home's: it looks like home until it moves
            visibility = VISIBLE
        }
        pull++
        if (pull >= HomeScreen.PULL_TICKS) { commitLock(); return }
        Sounds.play(if (pull == 1) Sounds.S.PULL1 else Sounds.S.PULL2)
        spring.animateTo(1f - STEPS[pull - 1], PULL_STIFFNESS, PULL_DAMPING)
        postDelayed(relax, RELAX_MS)
    }

    /** A tick down mid-fold: back toward home a step. */
    fun lockBack() {
        if (committed || !locking) return
        removeCallbacks(relax)
        pull--
        if (pull <= 0) relaxLock() else {
            spring.animateTo(1f - STEPS[pull - 1], PULL_STIFFNESS, PULL_DAMPING)
            postDelayed(relax, RELAX_MS)
        }
    }

    /** Unfold back to home (a pause, a turn down, a click). */
    fun relaxLock() {
        removeCallbacks(relax)
        if (!locking || committed) return
        pull = 0
        spring.animateTo(1f, RELAX_STIFFNESS, 1f) {
            up = false
            locking = false
            visibility = INVISIBLE
            morph = null
            spring.snapTo(0f)
            home.setGlanceHidden(false)
            character.setActive(false)
        }
    }

    private fun commitLock() {
        committed = true
        pull = 0
        removeCallbacks(relax)
        Sounds.play(Sounds.S.SLEEP)
        spring.animateTo(0f, COMMIT_STIFFNESS, 1f) { locked() }
        removeCallbacks(lockFailsafe)
        postDelayed(lockFailsafe, 1200)
    }

    /** Folded: the glance screen at rest, over home, ready for the display to go off. */
    private fun locked() {
        removeCallbacks(lockFailsafe)
        committed = false
        locking = false
        pull = 0
        morph = null
        spring.snapTo(0f)
        home.setGlanceHidden(false)
        onLocked()
    }

    private fun nudge() {
        Sounds.play(Sounds.S.BUMP)
        character.animate().cancel()
        character.animate().translationY(context.dp(12f)).setStartDelay(0).setDuration(110).setInterpolator(Theme.EASE_OUT)
            .withEndAction { character.animate().translationY(0f).setDuration(300).setInterpolator(OvershootInterpolator(2f)).start() }.start()
    }

    /** Measures where everything goes (once per morph). False when home has nothing to morph onto. */
    private fun begin(): Boolean {
        if (morph != null) return true
        // Leaving with a reply's agent up: home turns to it, so it lands there, and its chat opens after.
        leavingTo = if (locking) null else (replyOn() ?: replies.firstOrNull())?.takeIf { it.agent == agent.id }
        if (!locking && agent.id != home.agentId && home.shell.agent(agent.id) != null) home.select(agent.id, animate = false)
        val target = home.currentCharacter() ?: return false
        rising?.cancel()
        character.animate().cancel()
        character.translationY = 0f
        character.scaleX = 1f
        character.scaleY = 1f
        character.rotation = 0f
        character.pivotX = character.width / 2f
        character.pivotY = character.height / 2f
        character.setActive(false) // rest pose, so it lands exactly on home's still character
        fun at(v: View) = IntArray(2).also { v.getLocationOnScreen(it) }
        val c0 = at(clock)
        val c1 = at(home.clockView)
        val d0 = at(device)
        val d1 = at(home.statusView)
        val a0 = at(character)
        val a1 = at(target)
        val scale = if (textual) home.clockView.textSize / (clock as android.widget.TextView).textSize else 1f
        morph = Morph(
            clockDx = (c1[0] - c0[0]).toFloat(), clockDy = (c1[1] - c0[1]).toFloat(), clockScale = scale,
            // The date rides up under the shrinking clock while it fades.
            dateDy = (c1[1] - c0[1]) - clock.height * (1 - scale),
            deviceDx = (d1[0] - d0[0]).toFloat(), deviceDy = (d1[1] - d0[1]).toFloat(),
            // Centre to centre: it scales about its centre on the way.
            charDx = (a1[0] + target.width / 2f) - (a0[0] + character.width / 2f),
            charDy = (a1[1] + target.height / 2f) - (a0[1] + character.height / 2f),
            charScale = target.width.toFloat() / character.width,
        )
        home.setGlanceHidden(true)
        return true
    }

    /** Puts every piece at `p` of the way from the glance screen (0) to home (1). Properties only. */
    private fun apply(p: Float) {
        scrim.alpha = (1f - p).coerceIn(0f, 1f)
        middle.alpha = (1f - p * 2.5f).coerceIn(0f, 1f)
        bubble.alpha = minOf(bubble.alpha, middle.alpha)
        val m = morph
        if (m == null) {
            for (v in listOf(clock, date, device)) { v.translationX = 0f; v.translationY = 0f; v.alpha = 1f }
            clock.scaleX = 1f; clock.scaleY = 1f
            if (Look.agentPlace == "hidden") character.alpha = 0f
            character.translationX = 0f
            if (rising?.isRunning != true) {
                character.translationY = 0f
                character.scaleX = 1f; character.scaleY = 1f; character.rotation = 0f
            }
            return
        }
        clock.pivotX = 0f
        clock.pivotY = 0f
        val s = 1f + (m.clockScale - 1f) * p
        clock.scaleX = s
        clock.scaleY = s
        clock.translationX = m.clockDx * p
        clock.translationY = m.clockDy * p
        // A face that isn't digits (words, a dial, stacked) fades as it goes, and home's clock takes over.
        if (!textual) clock.alpha = (1f - p * 2.5f).coerceIn(0f, 1f)
        if (Look.agentPlace == "hidden") character.alpha = p
        date.translationX = m.clockDx * p
        date.translationY = m.dateDy * p
        date.alpha = (1f - p * 2.5f).coerceIn(0f, 1f)
        device.translationX = m.deviceDx * p
        device.translationY = m.deviceDy * p
        character.translationX = m.charDx * p
        character.translationY = m.charDy * p
        // Jelly: it stretches with the wheel's speed and leans a little into the move.
        val v = spring.velocity
        val stretch = (v * 0.045f).coerceIn(-0.08f, 0.14f)
        val cs = 1f + (m.charScale - 1f) * p
        character.scaleY = cs * (1f + stretch)
        character.scaleX = cs * (1f - stretch * 0.55f)
        character.rotation = (v * 1.6f).coerceIn(-5f, 5f)
    }

    /** Home is on screen: hand over to it. */
    private fun finish() {
        spring.cancel()
        removeCallbacks(relax)
        removeCallbacks(failsafe)
        up = false
        pull = 0
        committed = false
        locking = false
        visibility = INVISIBLE
        home.setGlanceHidden(false)
        morph = null
        spring.snapTo(0f)
        character.setActive(false)
        home.landCharacter()
        onOpened()
        // What was asked for (a widget's app, words to send), else the reply it was showing.
        val reply = leavingTo
        leavingTo = null
        val then = afterOpen ?: reply?.let { r -> { openReply(r) } }
        afterOpen = null
        then?.invoke()
    }

    // ---- device status --------------------------------------------------------------------------------

    /**
     * The soonest timer (ADR-0050), or none: its countdown, which ticks only while the glance screen is on screen (a
     * paused one shows what's left). `skew` is r1d's clock minus this one's. Left out with the next alarm (the look's
     * `alarm` part).
     */
    fun setTimer(t: org.json.JSONObject?, skew: Long) {
        timerSlot.removeAllViews()
        val show = t != null && !Look.hides("alarm")
        timerIcon.visibility = if (show) VISIBLE else GONE
        timerSlot.visibility = timerIcon.visibility
        if (t == null || !show) return
        val c = countdown(context, t, skew, 13f)
        c.typeface = Theme.font(600)
        c.setTextColor(Theme.MUTED)
        timerSlot.addView(c)
    }

    /** The next alarm's time ("7:00 am", "sat 7:00 am"), or none. */
    fun setNextAlarm(text: String?) {
        alarmIcon.visibility = if (text != null && !Look.hides("alarm")) VISIBLE else GONE
        alarmTime.visibility = alarmIcon.visibility
        alarmTime.text = text.orEmpty()
    }

    fun onDeviceStatus(pct: Int, charging: Boolean, wifiLevel: Int) {
        battery.text = if (pct >= 0) "$pct%" else ""
        bolt.visibility = if (charging) VISIBLE else GONE
        val icon = when {
            wifiLevel < 0 -> "wifi-slash"
            wifiLevel >= 3 -> "wifi-high"
            wifiLevel == 2 -> "wifi-medium"
            wifiLevel == 1 -> "wifi-low"
            else -> "wifi-none"
        }
        if (icon != wifiIcon) {
            wifiIcon = icon
            wifi.setImageDrawable(Icons.drawable(icon, Theme.MUTED))
        }
    }

    private companion object {
        /** Where the notifications summary and the page start: just under the time, the date and the device row. */
        const val NOTES_TOP_DP = 160f
        /** How far a corner agent sits past the screen's edge. */
        const val CORNER_DP = 34f
        /** The greeting comes once the agent has risen, and stays this long. */
        const val GREET_DELAY_MS = 800L
        const val BUBBLE_MS = 3600L
        /** The greeting beside the agent: the gap from its body, half its own height, and where its face is. */
        const val BUBBLE_GAP_DP = 4f
        const val BUBBLE_HALF_DP = 20f
        const val FACE_AT = 0.48f
        /** Each side of an agent's box its body doesn't fill (its forms sit in the middle ~60%). */
        const val BODY_SIDE = 0.2f
        /** With nothing new, it greets you again after this long (or when the part of the day changes). */
        const val GREET_EVERY_MS = 3 * 60 * 60_000L
        const val YAWN_DELAY_MS = 1700L
        /** The agent here: a little bigger than on home, so it shrinks into place. */
        const val CHAR_DP = 232f
        /** Docked on its side (T-103): where the right column starts, and the agents along the bottom. */
        const val DOCK_SPLIT_DP = 250f
        const val DOCK_CHAR_DP = 128f
        const val DOCK_PEEK_DP = 92f
        /** The dial goes away after this long without a turn or a click. */
        const val DIAL_IDLE_MS = 20_000L
        /** How much of it shows over the bottom edge: its face and most of its body. */
        /** The width of an agent-at-work page's live picture of the app (T-151). */
        const val WORK_PICTURE_DP = 78f
        const val PEEK_DP = 172f
        /** A reply's words on its page, at most. */
        const val REPLY_CHARS = 220
        /** How far the agent dips to swap for another (dp). */
        const val SWAP_DP = 70f
        /** With widgets, at most: its face and less of its body. A taller card sinks it further ([fitPage]). */
        const val PEEK_WIDGETS_DP = 124f
        /** The empty top of the agent's view, above its head: what can sit over a card without covering it. */
        const val BODY_INSET_DP = 40f
        /** Between the page's dots and the agent. */
        const val MIDDLE_GAP_DP = 10f
        /** How far below its spot it starts when the display comes on. */
        const val RISE_DP = 90f
        /** How far the resisting ticks take the morph: less each time. The next one commits. */
        val STEPS = floatArrayOf(0.2f, 0.36f)
        /** Following the wheel: quick, with only a hint of give (no visible swing back). */
        const val PULL_STIFFNESS = 260f
        const val PULL_DAMPING = 0.85f
        /** The glide home once it's committed: critically damped, so it lands exactly. */
        const val COMMIT_STIFFNESS = 300f
        /** Easing back after a pause. */
        const val RELAX_STIFFNESS = 150f
        /** A pause this long, short of committing, lets it ease back. */
        const val RELAX_MS = 1400L
        /** Where the page you last turned to is kept (T-125). */
        const val LAST_PAGE = "glance_page"
        /** The notifications' page's key (T-130): it isn't remembered; it comes first anyway. */
        const val NOTES = "notes"
        /** Notifications shown on their page; the rest are "+N more". */
        const val MAX_NOTES = 3
        /** How quickly it fades as what you touched opens (T-128). */
        const val LEAVE_MS = 200L
        /** A flick that puts a notification away however far it went (dp a second). */
        const val FLING_DP = 900f
    }
}

/**
 * A widget's slot: its card laid out at its full height, then scaled down to fit (to [MIN_FIT] at most, so its words
 * stay readable) and cut off below that. Scaled about its top centre.
 */
private class FitBox(context: Context) : FrameLayout(context) {
    /** The card's own height (px), however much room it has; [onNatural] hears when it changes. */
    var natural = 0
        private set
    var onNatural: () -> Unit = {}
    /** A sideways swipe across the card: 1 to the left (the next), -1 to the right (T-117). A tap still reaches the card. */
    var onSwipe: ((Int) -> Unit)? = null
    private var downX = 0f
    private var downY = 0f
    private var swiping = false
    private val slop = android.view.ViewConfiguration.get(context).scaledTouchSlop

    /** Follows a touch: sideways far enough, and more than it moves up or down, it's a swipe. */
    private fun track(ev: android.view.MotionEvent) {
        when (ev.actionMasked) {
            android.view.MotionEvent.ACTION_DOWN -> { downX = ev.x; downY = ev.y; swiping = false }
            android.view.MotionEvent.ACTION_MOVE -> {
                val dx = ev.x - downX
                if (kotlin.math.abs(dx) > slop * 2 && kotlin.math.abs(dx) > kotlin.math.abs(ev.y - downY) * 1.5f) swiping = true
            }
            android.view.MotionEvent.ACTION_UP -> if (swiping) {
                swiping = false
                onSwipe?.invoke(if (ev.x < downX) 1 else -1)
            }
            android.view.MotionEvent.ACTION_CANCEL -> swiping = false
        }
    }

    // A card that takes touches (a reply opens its chat): the swipe is taken from it once it's clearly sideways.
    override fun onInterceptTouchEvent(ev: android.view.MotionEvent): Boolean {
        if (onSwipe == null) return false
        if (ev.actionMasked != android.view.MotionEvent.ACTION_UP) track(ev)
        return swiping
    }

    // Anywhere else on the card, or once taken: the rest of the touch comes here.
    @android.annotation.SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(ev: android.view.MotionEvent): Boolean {
        if (onSwipe == null) return false
        track(ev)
        return true
    }

    override fun onMeasure(widthSpec: Int, heightSpec: Int) {
        super.onMeasure(widthSpec, heightSpec)
        val child = getChildAt(0) ?: return
        child.measure(MeasureSpec.makeMeasureSpec(measuredWidth, MeasureSpec.EXACTLY), MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED))
        if (child.measuredHeight != natural) {
            natural = child.measuredHeight
            onNatural()
        }
        val scale = if (child.measuredHeight > measuredHeight && measuredHeight > 0) (measuredHeight.toFloat() / child.measuredHeight).coerceAtLeast(MIN_FIT) else 1f
        child.pivotX = measuredWidth / 2f
        child.pivotY = 0f
        child.scaleX = scale
        child.scaleY = scale
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        getChildAt(0)?.let { it.layout(0, 0, it.measuredWidth, it.measuredHeight) }
    }

    private companion object {
        const val MIN_FIT = 0.8f
    }
}

/** When the glance screen last greeted you, kept over rebuilds (a change of look) so they don't greet again. */
private object Greeting {
    var at = -GREET_LONG_AGO
    var period = ""
    var news = 0
    private const val GREET_LONG_AGO = 24 * 60 * 60_000L
}
