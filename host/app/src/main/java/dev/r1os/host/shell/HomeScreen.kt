package dev.r1os.host.shell

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorFilter
import android.graphics.DashPathEffect
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.Shader
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.animation.OvershootInterpolator
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextClock
import android.widget.TextView
import dev.r1os.host.AndroidApps
import dev.r1os.host.net.Agent
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.net.currentChat
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Noise
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * Home (docs/specs/experience.md §2.1; ADR-0011, ADR-0014, ADR-0015, ADR-0016, ADR-0018): your agents
 * on the wheel, then New agent, with the launcher below them.
 *
 *   wheel         moves through your agents; past New agent it pulls the launcher up from the bottom,
 *                 against resistance: a peek, a stretch, then it opens ([PULL_TICKS] ticks)
 *   click         the chat with the agent on screen
 *   up past the first agent: folds back into the glance screen and sleeps (ADR-0022)
 *   double click  the agent's space (on New agent: make one)
 *   hold          talk to the agent on screen (in its current chat, or a new one)
 *   touch         the agent: a tap pokes it, a stroke pets it; its name opens its space; a neighbour selects it
 *   long press    edit mode (ADR-0044): everything wiggles; the wheel (or a drag) moves the one on screen among its
 *                 kind, a pinned app's badge takes it off home, a click keeps it all, cancel (or a double click) doesn't
 *
 * After New agent come the apps pinned to home (ADR-0044), each a stop like an agent's: a click opens it.
 *
 * The clock, battery and Wi-Fi sit at the top as text (ADR-0015): there's no status bar. The agent
 * on screen is large with its name and what it's doing; its neighbours peek above and below, small
 * and still. Moving animates only translation, scale and alpha, and a fast spin retargets mid-flight.
 */
@SuppressLint("ViewConstructor")
class HomeScreen(shell: Shell) : Screen(shell) {
    override val view = FrameLayout(context)

    private val agentsPage = FrameLayout(context)
    /** Home's background (ADR-0043); with flow, the wheel and the button nudge it ([nudge], [press]). */
    private val backdrop = dev.r1os.host.ui.Backdrop(context, dev.r1os.host.ui.Look.background("home"))
    fun nudge(delta: Int) = backdrop.nudge(delta)
    fun press(down: Boolean) = backdrop.press(down)
    private val wheel = FrameLayout(context).apply { clipChildren = false }
    private val slides = LinkedHashMap<String, Slide>()
    private val dots = Dots(context)
    /**
     * Docked on its side (T-156; Max, 2026-10-13: "a little button on the bottom right corner with a mic to speak to an
     * agent"): touched, the agent on screen listens, and the overlay sends or cancels.
     */
    private val dockMic = android.widget.FrameLayout(context).apply {
        visibility = View.GONE
        background = roundRect(Theme.withAlpha(Theme.accent, 0.2f), context.dp(MIC_DP / 2))
        addView(ImageView(context).apply { setImageDrawable(Icons.drawable("microphone", Theme.accent)) },
            android.widget.FrameLayout.LayoutParams(context.dpi(MIC_DP * 0.45f), context.dpi(MIC_DP * 0.45f), Gravity.CENTER))
        setOnClickListener {
            Sounds.play(Sounds.S.POP)
            shell.talkByTouch("start")
        }
    }

    /** On its side with docked mode on (T-156). */
    private var docked = false

    override fun onDock(on: Boolean) {
        docked = on
        dockMic.visibility = if (on) View.VISIBLE else View.GONE
    }

    private val launcherHint = ImageView(context).apply {
        setImageDrawable(Icons.drawable("caret-down", Theme.MUTED))
        alpha = 0f
    }

    private val clock = TextClock(context).apply {
        format12Hour = "h:mm"
        format24Hour = "H:mm"
        setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 34f)
        typeface = Theme.font(650)
        setTextColor(Theme.FG)
        includeFontPadding = false
        fontFeatureSettings = "tnum"
        letterSpacing = -0.01f
    }
    private val battery = context.text(13f, 600, Theme.MUTED).apply { fontFeatureSettings = "tnum" }
    private val bolt = ImageView(context).apply { setImageDrawable(Icons.drawable("lightning", Theme.MUTED)) }
    private val wifi = ImageView(context)
    private var wifiIcon = ""
    private val statusRow = LinearLayout(context)

    /** The launcher: a sheet that rises over home from below. Home stays exactly where it is. */
    private val launcherPage = LinearLayout(context)
    /** The launcher sheet's background: rounded at the top while it's pulled up, square once it covers the screen. No
     *  stroke: a stroke insets the fill by half its width, and what's behind would show along every edge. */
    private val sheetBg = android.graphics.drawable.GradientDrawable().apply { setColor(Theme.SHEET) }

    private fun roundSheet() {
        val t = (launcherPage.translationY / context.dp(SQUARE_DP)).coerceIn(0f, 1f)
        val r = context.dp(24f) * t
        sheetBg.cornerRadii = floatArrayOf(r, r, r, r, 0f, 0f, 0f, 0f)
    }
    private val launcher = FocusList(context)
    var launcherOn = false
        private set
    /** Ticks pulled past New agent, short of opening (ADR-0018). */
    private var pull = 0
    private val relax = Runnable { setPull(0) }
    /**
     * Ticks turned past an end (toward [pastDir]: -1 above the first agent, 1 below New agent). The first
     * [GUARD_TICKS] only bump; the pull to sleep or to the launcher starts after them (Max, 2026-10-09:
     * "extra confirmation to start it"). A pause, or a turn the other way, starts the count over.
     */
    private var past = 0
    private var pastDir = 0
    private val forgetPast = Runnable { past = 0; pastDir = 0 }

    /** The id on screen: an agent's, or [NEW]. */
    var at: String = ""
        private set

    private val builds = HashMap<String, Build>()
    private val buildOwner = HashMap<Long, String>()
    private val timers = ArrayList<Runnable>()
    private var apps: List<JSONObject> = emptyList()
    /** The Android apps installed since the image ([AndroidApps]), and whether Aurora Store is there. */
    private var androidApps: List<AndroidApps.App> = emptyList()
    /** Every Android app, system ones too: a pinned one is looked up here. */
    private var allAndroid: List<AndroidApps.App> = emptyList()
    private var hasStore = false
    private var accounts: JSONObject? = null
    private var shown = false
    private var lastOrder = ""

    /** Apps pinned to home (ADR-0044), after New agent on the wheel: a mini-app's id, or `android:<package>`. */
    private var pins: List<String> = emptyList()
    /** Edit mode (ADR-0044): the order being arranged, until a click keeps it or cancel drops it. */
    private var editing = false
    private var draftAgents = mutableListOf<String>()
    private var draftPins = mutableListOf<String>()
    private val wiggles = ArrayList<android.animation.Animator>()
    /** The clock and status: edit mode puts cancel where they'd be read. */
    private var topLeft: View? = null
    private val cancelPill = context.text(16f, 700).apply {
        text = "cancel"
        setPadding(context.dpi(16f), context.dpi(8f), context.dpi(16f), context.dpi(9f))
        background = roundRect(Theme.FOCUS, context.dp(18f))
        visibility = View.GONE
        setOnClickListener { cancelEdit() }
    }
    private val editHint = context.text(14f, 600, Theme.MUTED).apply {
        text = "turn to move · click to save"
        gravity = Gravity.CENTER
        visibility = View.GONE
    }

    private val step get() = context.dp(STEP_DP)

    private data class Build(val id: Long, val stage: String, val section: String?)

    init {
        // The agents page: clock and status, the wheel and its dots. No buttons: the launcher is a
        // pull down the wheel, and sleep is a click.
        statusRow.apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(battery)
            addView(bolt, LinearLayout.LayoutParams(context.dpi(13f), context.dpi(13f)).apply { leftMargin = context.dpi(2f) })
            addView(wifi, LinearLayout.LayoutParams(context.dpi(15f), context.dpi(15f)).apply { leftMargin = context.dpi(9f) })
        }
        val glance = LinearLayout(context).also { topLeft = it }.apply {
            orientation = LinearLayout.VERTICAL
            // Its own width: a vertical layout's default (match parent) would clip it to the status row's.
            addView(clock, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT))
            addView(statusRow, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(5f) })
        }
        // Home's background (the look's, ADR-0043).
        agentsPage.addView(backdrop, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        agentsPage.addView(wheel, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        agentsPage.addView(glance, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT).apply {
            leftMargin = context.dpi(20f); topMargin = context.dpi(14f)
        })
        agentsPage.addView(launcherHint, FrameLayout.LayoutParams(context.dpi(18f), context.dpi(18f), Gravity.CENTER_HORIZONTAL or Gravity.BOTTOM).apply { bottomMargin = context.dpi(32f) })
        agentsPage.addView(dockMic, FrameLayout.LayoutParams(context.dpi(MIC_DP), context.dpi(MIC_DP), Gravity.END or Gravity.BOTTOM).apply {
            rightMargin = context.dpi(16f); bottomMargin = context.dpi(16f)
        })
        // Edit mode (ADR-0044): cancel at the top right, and what the wheel and the click do at the bottom.
        agentsPage.addView(cancelPill, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.END).apply {
            topMargin = context.dpi(14f); rightMargin = context.dpi(14f)
        })
        agentsPage.addView(editHint, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM).apply { bottomMargin = context.dpi(26f) })
        agentsPage.addView(dots, FrameLayout.LayoutParams(context.dpi(12f), FrameLayout.LayoutParams.MATCH_PARENT, Gravity.END).apply { rightMargin = context.dpi(8f) })

        // The launcher, below the agents (ADR-0018): a sheet with rounded top corners, over home.
        launcherPage.orientation = LinearLayout.VERTICAL
        launcherPage.background = sheetBg
        // Open, it covers the whole screen: its corners square off as it reaches the top.
        launcherPage.animate().setUpdateListener { roundSheet() }
        launcher.setPadding(context.dpi(12f), context.dpi(12f), context.dpi(12f), context.dpi(16f))
        launcher.scale = 1.3f // the launcher's rows: bigger still (Max, 2026-10-09)
        launcherPage.addView(launcher, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        launcherPage.visibility = View.GONE

        view.addView(agentsPage, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        view.addView(launcherPage, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        view.addOnLayoutChangeListener { _, _, top, _, bottom, _, oldTop, _, oldBottom ->
            if (bottom - top != oldBottom - oldTop) {
                if (!launcherOn && pull == 0) launcherPage.translationY = (bottom - top).toFloat()
                place(animate = false)
            }
        }
        buildLauncher()
        loadAccounts()
        loadApps()
        loadAndroid()
        loadPins()
        shell.keep(dev.r1os.host.Notifications.onChange { if (launcherOn) buildLauncher() })
    }

    override val agentId: String? get() = if (launcherOn || at == NEW || at.startsWith(PIN)) null else at.ifEmpty { null }

    // ---- data -------------------------------------------------------------------------------------

    private fun loadAccounts() = R1d.get("/api/accounts") { accounts = it.optJSONObject("accounts"); render() }

    private fun loadApps() = R1d.get("/api/apps") { r ->
        val list = r.optJSONArray("apps")
        apps = (0 until (list?.length() ?: 0)).mapNotNull { list?.optJSONObject(it) }
        buildLauncher()
        // Pinned apps show once their app is known.
        if (!editing) render()
    }

    private fun loadPins() = R1d.get("/api/home") { r -> setPins(r.optJSONArray("pins")) }

    /** The apps pinned to home (`GET /api/home`, `home` events). */
    fun setPins(a: org.json.JSONArray?) {
        pins = (0 until (a?.length() ?: 0)).map { a!!.optString(it) }
        if (!editing) render()
        buildLauncher()
    }

    /** Pinned apps that are here: an installed mini-app, an Android app on the R1, one of r1os's tools (T-091). */
    private fun shownPins(): List<String> = pins.filter { p ->
        when {
            p.startsWith("android:") -> allAndroid.any { it.pkg == p.removePrefix("android:") }
            p.startsWith(TOOL) -> TOOLS.any { it.first == p.removePrefix(TOOL) }
            else -> apps.any { it.optString("id") == p }
        }
    }

    /** One of r1os's tools (the launcher's, or pinned to home): your browser is a screen of its own. */
    private fun openTool(id: String) {
        if (id == "browser") shell.open(Target.of("browser")) else shell.open(Target.of("tool", "id" to id))
    }

    /** The launcher's long press (ADR-0044): onto home, or off it. */
    private fun togglePin(p: String) {
        val on = p in pins
        val next = if (on) pins - p else pins + p
        R1d.put("/api/home", JSONObject().put("pins", org.json.JSONArray(next)), onError = { shell.flash(it) }) { r ->
            setPins(r.optJSONArray("pins"))
            shell.flash(if (on) "off home" else "pinned to home")
        }
    }

    private fun openPin(p: String) {
        Sounds.play(Sounds.S.POP)
        when {
            p.startsWith("android:") -> shell.openAndroid(p.removePrefix("android:"))
            p.startsWith(TOOL) -> openTool(p.removePrefix(TOOL))
            else -> shell.open(Target.of("app", "id" to p))
        }
    }

    /** The launcher's Android apps, again: when the shell starts, and when one is installed or removed. */
    fun loadAndroid() {
        allAndroid = AndroidApps.list(context)
        if (!editing) render()
        androidApps = allAndroid.filter { !it.system }
        hasStore = AndroidApps.has(context, AndroidApps.STORE)
        buildLauncher()
    }

    /** Selects `id` (an agent's, or [NEW]); unknown ids fall back to the focus agent. */
    fun select(id: String?, animate: Boolean = true) {
        val ids = entries()
        at = when {
            id != null && id in ids -> id
            at in ids -> at
            else -> shell.focus.takeIf { it in ids } ?: ids.first()
        }
        render()
        place(animate)
    }

    private fun entries(): List<String> =
        if (editing) draftAgents + NEW + draftPins.map { PIN + it }
        else shell.agents.map { it.id } + NEW + shownPins().map { PIN + it }

    /** The agent list or a state changed: redraws what changed. */
    fun render() {
        val ids = entries()
        if (at !in ids) at = shell.focus.takeIf { it in ids } ?: ids.first()
        // Slides for agents that went away are dropped; new ones are made.
        val gone = slides.keys - ids.toSet()
        gone.forEach { wheel.removeView(slides.remove(it)!!.view) }
        var made = false
        for (id in ids) {
            val s = slides.getOrPut(id) {
                made = true
                Slide(id).also { wheel.addView(it.view, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT)) }
            }
            val a = shell.agents.firstOrNull { it.id == id }
            when {
                a != null -> s.bind(a)
                id.startsWith(PIN) -> s.bindPin(id.removePrefix(PIN))
                else -> s.bindNew(shell.agents.size >= shell.max)
            }
            s.setEditing(editing)
        }
        // The order changed, or an agent came or went: everything moves to its new place.
        val order = ids.joinToString(",")
        if (made || gone.isNotEmpty() || order != lastOrder) place(false)
        else dots.set(ids.map { id -> shell.agents.firstOrNull { it.id == id } }, ids.indexOf(at))
        lastOrder = order
    }

    // ---- the wheel ----------------------------------------------------------------------------------

    /** Positions every slide at its offset from the one on screen. */
    private fun place(animate: Boolean) {
        val ids = entries()
        val sel = ids.indexOf(at).coerceAtLeast(0)
        val h = view.height.takeIf { it > 0 } ?: return
        // A little below the middle: the clock has the top.
        val centre = h / 2f + context.dp(8f)
        ids.forEachIndexed { i, id ->
            val s = slides[id] ?: return@forEachIndexed
            val d = i - sel
            val on = d == 0
            val near = kotlin.math.abs(d) <= WINDOW
            s.setOn(on && shown && !launcherOn)
            val y = centre - s.anchor() + d * step
            val scale = if (on) 1f else 0.5f
            val alpha = if (on) 1f else if (kotlin.math.abs(d) == 1) 0.42f else 0f
            val v = s.view
            if (near) v.visibility = View.VISIBLE
            if (animate && near) {
                v.animate().translationY(y).scaleX(scale).scaleY(scale).alpha(alpha).setDuration(340).setInterpolator(Theme.SETTLE)
                    .withEndAction { if (kotlin.math.abs(ids.indexOf(id) - ids.indexOf(at)) > WINDOW) v.visibility = View.INVISIBLE }.start()
            } else {
                v.animate().cancel()
                v.translationY = y; v.scaleX = scale; v.scaleY = scale; v.alpha = alpha
                if (!near) v.visibility = View.INVISIBLE
            }
            s.setLabels(on)
        }
        dots.set(ids.map { id -> shell.agents.firstOrNull { it.id == id } }, sel)
        launcherHint.animate().alpha(if (sel == ids.size - 1 && !editing) 0.55f else 0f).setDuration(200).start()
    }

    /**
     * A swipe up or down on home (T-156): one agent on, as a tick of the wheel; up past New agent, the launcher. Not
     * while editing or with the launcher up (they're the wheel's and their own).
     */
    fun swipe(d: Int) {
        if (editing || launcherOn) return
        if (d > 0 && entries().indexOf(at) >= entries().size - 1) showLauncher(fromWheel = false) else move(d)
    }

    private fun move(d: Int) {
        if (pull > 0) setPull(0)
        view.removeCallbacks(forgetPast)
        past = 0
        pastDir = 0
        val ids = entries()
        val i = (ids.indexOf(at) + d).coerceIn(0, ids.size - 1)
        if (ids[i] == at) return
        at = ids[i]
        Sounds.play(Sounds.S.TICK)
        place(true)
    }

    /**
     * A tick past an end: true once [GUARD_TICKS] have gone by and the pull may start. Until then the agent
     * (or New agent's ring) leans toward the edge, a little further each tick, and springs back, with a
     * soft bump.
     */
    private fun pastEnd(dir: Int): Boolean {
        view.removeCallbacks(forgetPast)
        view.postDelayed(forgetPast, RELAX_MS)
        if (pastDir != dir) {
            past = 0
            pastDir = dir
        }
        if (past >= GUARD_TICKS) {
            // The pull starts from where the agent really sits (the fold to sleep measures it).
            slides[at]?.lean(0f, animate = false)
            return true
        }
        past++
        Sounds.play(Sounds.S.BUMP)
        slides[at]?.lean(-dir * context.dp(LEAN_DP[(past - 1).coerceIn(0, LEAN_DP.size - 1)]))
        return false
    }

    /** A click (ADR-0022): the chat with the agent on screen (its current one, or a new one); a pinned app opens. */
    private fun talkTo() {
        if (at.startsWith(PIN)) { openPin(at.removePrefix(PIN)); return }
        Sounds.play(Sounds.S.POP)
        if (at == NEW) {
            if (shell.agents.size < shell.max) shell.open(Target.of("new-agent"))
            return
        }
        val agent = at
        currentChat(agent) { id -> shell.open(Target.of("chat", "agent" to agent, "id" to id)) }
    }

    /** On screen: the agent opens its space; New agent starts making one; a pinned app opens. */
    private fun openCurrent() {
        if (at.startsWith(PIN)) { openPin(at.removePrefix(PIN)); return }
        Sounds.play(Sounds.S.POP)
        if (at == NEW) {
            if (shell.agents.size < shell.max) shell.open(Target.of("new-agent"))
            return
        }
        shell.open(Target.of("agent", "id" to at))
    }

    // ---- the launcher (ADR-0018) ------------------------------------------------------------------

    private fun buildLauncher() {
        val keep = launcher.focus
        launcher.clear()
        launcher.item("plus", "new app", Theme.accent, bold = true) { shell.open(Target.of("create")) }
        if (apps.isNotEmpty()) launcher.section("Apps")
        for (a in apps) {
            val id = a.optString("id")
            val accent = Theme.COLORS[a.optString("accent")] ?: Color.parseColor("#9aa4b2")
            // A long press pins it to home, or takes it off (ADR-0044).
            launcher.item(a.optString("icon", "app-window"), a.optString("name", id), accent, trailing = if (id in pins) "on home" else null,
                onLong = { togglePin(id) }) { shell.open(Target.of("app", "id" to id)) }
        }
        launcher.item("list", "manage apps") { shell.open(Target.of("manage")) }
        if (androidApps.isNotEmpty() || hasStore) launcher.section("Android apps")
        for (a in androidApps) {
            val pin = "android:${a.pkg}"
            launcher.item(null, a.name, image = a.icon(context), trailing = if (pin in pins) "on home" else null, onLong = { togglePin(pin) }) { shell.openAndroid(a.pkg) }
        }
        if (hasStore) launcher.item("storefront", "get apps", Theme.colorOf("green")) { shell.openAndroid(AndroidApps.STORE) }
        val notes = dev.r1os.host.Notifications.count
        if (notes > 0) launcher.item("bell", "notifications", trailing = "$notes") { shell.push(NotificationsScreen(shell)) }
        launcher.section("Tools")
        // A long press pins one to home, or takes it off (T-091), as for apps.
        for ((id, name, icon) in TOOLS) {
            val pin = "$TOOL$id"
            launcher.item(icon, name, trailing = if (pin in pins) "on home" else null, onLong = { togglePin(pin) }) { openTool(id) }
        }
        launcher.section("Settings")
        for ((page, name, icon) in SETTINGS) launcher.item(icon, name) { shell.open(Target.of("settings", "page" to page)) }
        launcher.item(if (Sounds.enabled) "speaker-high" else "speaker-slash", "sounds", trailing = if (Sounds.enabled) "on" else "off") {
            Sounds.enabled = !Sounds.enabled
            dev.r1os.host.DeviceSettings.reportSoon()
            buildLauncher()
        }
        launcher.item("android-logo", "android settings") { shell.open(Target.of("android")) }
        launcher.item("moon", "sleep") { shell.sleep() }
        launcher.section("")
        if (launcherOn && keep >= 0) launcher.setFocus(keep.coerceAtMost(launcher.size - 1), animate = false)
    }

    /**
     * The pull past New agent (ADR-0018): each tick short of [PULL_TICKS] raises the sheet a little
     * less than the one before, with a springy overshoot, and lifts the agents. A pause, or a turn
     * back up, lets it fall back.
     */
    private fun setPull(n: Int) {
        pull = n
        view.removeCallbacks(relax)
        val h = view.height.toFloat()
        if (n == 0) {
            // It fell back: starting again takes the guard ticks again.
            past = 0
            pastDir = 0
            launcherPage.animate().translationY(h).setDuration(260).setInterpolator(Theme.SETTLE)
                .withEndAction { if (!launcherOn && pull == 0) launcherPage.visibility = View.GONE }.start()
            return
        }
        if (launcherPage.visibility != View.VISIBLE) {
            launcherPage.translationY = h
            launcherPage.visibility = View.VISIBLE
            launcher.scrollTo(0, 0)
            launcher.blur()
        }
        Sounds.play(if (n == 1) Sounds.S.PULL1 else Sounds.S.PULL2)
        val peek = context.dp(PEEK_DP.getOrElse(n - 1) { PEEK_DP.last() })
        launcherPage.animate().translationY(h - peek).setDuration(320).setInterpolator(OvershootInterpolator(2.2f)).start()
        view.postDelayed(relax, RELAX_MS)
    }

    fun showLauncher(fromWheel: Boolean) {
        if (launcherOn) return
        launcherOn = true
        Sounds.play(Sounds.S.OPEN)
        pull = 0
        view.removeCallbacks(relax)
        val h = view.height.toFloat()
        if (launcherPage.visibility != View.VISIBLE) {
            launcherPage.translationY = h
            launcherPage.visibility = View.VISIBLE
        }
        launcherPage.animate().translationY(0f).setDuration(340).setInterpolator(Theme.SETTLE).start()
        launcher.scrollTo(0, 0)
        if (fromWheel) launcher.setFocus(0, animate = false) else launcher.blur()
        slides[at]?.setOn(false)
    }

    /** The launcher's list is at its top: a swipe down there closes it, as a sheet's does (T-156). */
    fun launcherAtTop() = launcherOn && launcher.scrollY == 0

    /** A swipe up from the bottom edge may raise the launcher (T-126): it isn't up, and home isn't being arranged. */
    fun canDragLauncher() = !launcherOn && !editing && pull == 0

    /**
     * The finger's raised the launcher `up` px from the bottom edge (T-126, Max: "Swiping up from the bottom of the screen
     * on the home screen should show the app launcher"): the same sheet the wheel's pull raises, following it.
     */
    fun dragLauncher(up: Float) {
        view.removeCallbacks(relax)
        val h = view.height.toFloat()
        if (launcherPage.visibility != View.VISIBLE) {
            launcherPage.translationY = h
            launcherPage.visibility = View.VISIBLE
            launcher.scrollTo(0, 0)
            launcher.blur()
        }
        launcherPage.animate().cancel()
        launcherPage.translationY = (h - up).coerceIn(0f, h)
        roundSheet()
    }

    /** Let go: up it goes (far enough, or flung), else back down. */
    fun releaseLauncher(open: Boolean) {
        if (open) {
            showLauncher(fromWheel = false)
            return
        }
        val h = view.height.toFloat()
        launcherPage.animate().translationY(h).setDuration(220).setInterpolator(Theme.SETTLE)
            .withEndAction { if (!launcherOn && pull == 0) launcherPage.visibility = View.GONE }.start()
    }

    /** Back up to your agents, where you were. */
    fun hideLauncher() {
        if (!launcherOn) return
        launcherOn = false
        Sounds.play(Sounds.S.CLOSE)
        val h = view.height.toFloat()
        launcherPage.animate().translationY(h).setDuration(340).setInterpolator(Theme.SETTLE)
            .withEndAction { if (!launcherOn && pull == 0) launcherPage.visibility = View.GONE }.start()
        slides[at]?.setOn(shown)
    }

    /** Home again from another screen: the agents, not the launcher. */
    fun reset() {
        view.removeCallbacks(relax)
        if (!launcherOn && pull == 0) return
        launcherOn = false
        pull = 0
        launcherPage.animate().cancel()
        launcherPage.visibility = View.GONE
        launcherPage.translationY = view.height.toFloat()
    }

    // ---- the glance screen's way in (ADR-0019) ----------------------------------------------------

    /** Where the glance screen's clock morphs to. */
    val clockView: TextClock get() = clock

    /** Where its battery and Wi-Fi row goes. */
    val statusView: View get() = statusRow

    /** Where its agent goes: the character on screen (null on New agent). */
    fun currentCharacter(): View? = slides[at]?.character

    /** The agent arrives from the glance screen: it squashes as it lands, and springs back up. */
    fun landCharacter() {
        val c = slides[at]?.character ?: return
        c.pivotX = c.width / 2f
        c.pivotY = c.height * 0.8f
        fun kf(prop: android.util.Property<View, Float>, vararg v: Pair<Float, Float>) =
            android.animation.PropertyValuesHolder.ofKeyframe(prop, *v.map { (t, x) -> android.animation.Keyframe.ofFloat(t, x) }.toTypedArray())
        android.animation.ObjectAnimator.ofPropertyValuesHolder(c,
            kf(View.SCALE_Y, 0f to 1f, 0.25f to 0.9f, 0.55f to 1.05f, 0.8f to 0.98f, 1f to 1f),
            kf(View.SCALE_X, 0f to 1f, 0.25f to 1.08f, 0.55f to 0.97f, 0.8f to 1.01f, 1f to 1f),
        ).apply {
            duration = 520
            interpolator = Theme.EASE_IN_OUT
            start()
        }
    }

    /** Hides what the glance screen is morphing onto, so there aren't two of each mid-way. */
    fun setGlanceHidden(hidden: Boolean) {
        val a = if (hidden) 0f else 1f
        clock.alpha = a
        statusRow.alpha = a
        slides[at]?.character?.alpha = a
    }

    // ---- input --------------------------------------------------------------------------------------

    override fun onEvent(ev: Ev): Boolean {
        // Edit mode (ADR-0044): the wheel moves the one on screen, a click keeps the order, a double click doesn't.
        if (editing) {
            when {
                ev.type == "wheel" -> { editMove(if (ev.int("delta") > 0) 1 else -1); return true }
                ev.isTap -> { saveEdit(); return true }
                ev.isDoubleTap -> { cancelEdit(); return true }
            }
            return false
        }
        if (launcherOn) {
            when {
                ev.type == "wheel" -> {
                    val d = if (ev.int("delta") > 0) 1 else -1
                    if (!launcher.move(d) && d < 0) hideLauncher()
                    return true
                }
                ev.isTap -> return launcher.activate()
                ev.isDoubleTap -> { hideLauncher(); return true }
            }
            return false
        }
        // Mid-pull: a turn back up, a click or a double click lets the launcher fall back.
        if (pull > 0 && (ev.isTap || ev.isDoubleTap || (ev.type == "wheel" && ev.int("delta") < 0))) {
            setPull(0)
            return true
        }
        when {
            ev.type == "wheel" -> {
                val d = if (ev.int("delta") > 0) 1 else -1
                val last = entries().indexOf(at) >= entries().size - 1
                when {
                    d > 0 && last && pull + 1 >= PULL_TICKS -> showLauncher(fromWheel = true)
                    // Down past New agent: after the guard ticks, the launcher's pull (ADR-0018).
                    d > 0 && last -> if (pull > 0 || pastEnd(1)) setPull(pull + 1)
                    // Up past the first agent: after the guard ticks, fold back into the glance screen
                    // and sleep (ADR-0022).
                    d < 0 && entries().indexOf(at) <= 0 -> if (pastEnd(-1)) shell.pullToSleep()
                    else -> move(d)
                }
                return true
            }
            ev.isTap -> { talkTo(); return true }
            ev.isDoubleTap -> { openCurrent(); return true }
        }
        return false
    }

    override fun onAnyEvent(ev: Ev) {
        when (ev.type) {
            "chat" -> if (ev.str("kind") == "build" && ev.json.has("build")) {
                val agent = ev.str("agent") ?: return
                val id = ev.json.optLong("build")
                buildOwner[id] = agent
                builds[agent] = Build(id, "writing", null)
                render()
            }
            "build" -> {
                val id = ev.json.optLong("id")
                val agent = buildOwner[id] ?: return
                val stage = ev.str("stage") ?: return
                builds[agent] = Build(id, stage, ev.str("section"))
                render()
                if (stage == "done" || stage == "failed") {
                    val r = Runnable { if (builds[agent]?.id == id) { builds.remove(agent); render() } }
                    timers += r
                    R1d.main.postDelayed(r, 8000)
                }
            }
            "account" -> if (ev.str("state") == "signed_in" || ev.str("state") == "signed_out") loadAccounts()
            "app_installed", "app_removed" -> { loadApps(); loadPins() }
        }
    }

    override fun onShow() {
        shown = true
        slides[at]?.setOn(!launcherOn)
        dots.setActive(true)
    }

    override fun onHide() {
        if (editing) exitEdit()
        backdrop.settle()
        shown = false
        slides.values.forEach { it.setOn(false) }
        dots.setActive(false)
    }

    override fun onReconnect() {
        loadAccounts()
        loadApps()
        loadPins()
    }

    /** Battery and Wi-Fi changed (the shell watches them). */
    fun onDeviceStatus(pct: Int, charging: Boolean, wifiLevel: Int) {
        battery.text = if (pct >= 0) "$pct%" else ""
        bolt.visibility = if (charging) View.VISIBLE else View.GONE
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

    // ---- pieces -------------------------------------------------------------------------------------

    private fun charState(a: Agent, b: Build?): String = when {
        !shell.connected -> "sleeping"
        b != null && b.stage != "done" && b.stage != "failed" -> "working"
        b?.stage == "failed" -> "error"
        else -> a.state
    }

    private fun activity(a: Agent, b: Build?): String? =
        if (b != null && b.stage != "done" && b.stage != "failed") "build" else CharacterView.activityOf(a.tool)

    private fun statusOf(a: Agent, b: Build?): Pair<String, Int> {
        val idle = 0
        val busy = 1
        val bad = 2
        if (!shell.connected) return "Starting up…" to busy
        if (b != null) return when (b.stage) {
            "repairing" -> "Fixing up the app…" to busy
            "done" -> "Your app is ready" to busy
            "failed" -> "That build didn't work" to bad
            else -> "Building an app${b.section?.let { " · $it" } ?: "…"}" to busy
        }
        val acc = accounts
        if (acc != null && !acc.optBoolean("claude") && !acc.optBoolean("codex")) return "Not connected: Settings › Accounts" to bad
        // Another agent's message (ADR-0012): who it's answering, or what's waiting from whom.
        val by = a.by?.let { shell.agent(it)?.name }
        if (by != null && a.state in BUSY) return "Answering $by…" to busy
        val mail = a.mail
        if (mail != null && a.state == "idle") {
            val from = shell.agent(mail.optString("from"))?.name ?: "An agent"
            val words = mail.optString("text").let { if (it.length > 40) it.take(39).trimEnd() + "…" else it }
            return (if (mail.optString("kind") == "tell") "$from: “$words”" else "$from asks: “$words”") to busy
        }
        // It works on a linked computer (docs/specs/companion.md §7.2): where, and whether it's there.
        val pc = a.computer
        if (pc != null) return when (a.state) {
            "thinking", "working" -> "Working on $pc…" to busy
            "speaking" -> "Talking…" to busy
            "waiting" -> "Waiting for $pc…" to busy
            "error" -> "Something went wrong" to bad
            else -> (if (a.computerHere) "On $pc" else "$pc is offline") to idle
        }
        return when (a.state) {
            "thinking" -> "Thinking…" to busy
            "speaking" -> "Talking…" to busy
            "working" -> "Working on it…" to busy
            "waiting" -> "Waiting for room to run…" to busy
            "error" -> "Something went wrong" to bad
            else -> "" to idle // nothing to say: the agent speaks for itself (Max, 2026-10-09)
        }
    }

    // ---- edit mode (ADR-0044) ------------------------------------------------------------------------

    private fun enterEdit() {
        if (editing || launcherOn) return
        editing = true
        if (pull > 0) setPull(0)
        draftAgents = shell.agents.map { it.id }.toMutableList()
        draftPins = shownPins().toMutableList()
        Sounds.play(Sounds.S.POP)
        topLeft?.animate()?.alpha(0f)?.setDuration(160)?.start()
        for (v in listOf<View>(cancelPill, editHint)) {
            v.visibility = View.VISIBLE
            v.alpha = 0f
            v.animate().alpha(1f).setDuration(200).start()
        }
        render()
        place(true)
        wiggle(true)
    }

    /** The one on screen moves a place among its kind (agents among agents, pinned apps among pinned apps). */
    private fun editMove(d: Int) {
        val group = when {
            at.startsWith(PIN) -> draftPins
            at == NEW -> null
            else -> draftAgents
        }
        if (group == null) {
            // New agent stays put: the wheel moves along instead.
            val ids = entries()
            val i = (ids.indexOf(at) + d).coerceIn(0, ids.size - 1)
            if (ids[i] != at) {
                at = ids[i]
                Sounds.play(Sounds.S.TICK)
                place(true)
            }
            return
        }
        val i = group.indexOf(at.removePrefix(PIN))
        val j = i + d
        if (i < 0 || j !in group.indices) {
            Sounds.play(Sounds.S.BUMP)
            slides[at]?.lean(-d * context.dp(9f))
            return
        }
        java.util.Collections.swap(group, i, j)
        Sounds.play(Sounds.S.TICK)
        place(true)
    }

    /** A pinned app's badge in edit mode: off home (kept only if the click saves). */
    private fun removePin(p: String) {
        val ids = entries()
        val i = ids.indexOf(PIN + p)
        draftPins.remove(p)
        if (at == PIN + p) at = ids.getOrNull(i - 1) ?: NEW
        Sounds.play(Sounds.S.BACK)
        render()
        place(true)
        wiggle(true)
    }

    private fun saveEdit() {
        val agents = draftAgents.toList()
        if (agents != shell.agents.map { it.id }) {
            R1d.put("/api/agents/order", JSONObject().put("ids", org.json.JSONArray(agents))) { shell.reloadAgents() }
        }
        // Pins whose app isn't here now (an Android app removed for a while) keep their place at the end.
        val kept = draftPins + pins.filter { it !in shownPins() }
        if (kept != pins) {
            pins = kept
            R1d.put("/api/home", JSONObject().put("pins", org.json.JSONArray(kept))) { r -> setPins(r.optJSONArray("pins")) }
        }
        Sounds.play(Sounds.S.POP)
        exitEdit()
        shell.flash("saved")
    }

    private fun cancelEdit() {
        Sounds.play(Sounds.S.BACK)
        exitEdit()
    }

    private fun exitEdit() {
        if (!editing) return
        editing = false
        wiggle(false)
        topLeft?.animate()?.alpha(1f)?.setDuration(200)?.start()
        cancelPill.visibility = View.GONE
        editHint.visibility = View.GONE
        if (at !in entries()) at = shell.focus
        render()
        place(true)
    }

    /** Edit mode's wiggle: each slide rocks a little, out of step with the others. Only while editing. */
    private fun wiggle(on: Boolean) {
        wiggles.forEach { it.cancel() }
        wiggles.clear()
        slides.values.forEach { it.view.rotation = 0f }
        if (!on) return
        slides.values.forEachIndexed { i, s ->
            wiggles += android.animation.ObjectAnimator.ofFloat(s.view, View.ROTATION, -1.8f, 1.8f).apply {
                duration = 130L + (i % 3) * 18
                repeatMode = android.animation.ValueAnimator.REVERSE
                repeatCount = android.animation.ValueAnimator.INFINITE
                startDelay = (i * 37L) % 90
                start()
            }
        }
    }

    /** One entry on the wheel: name, character (or New agent's ring, or a pinned app's tile), and status. */
    private inner class Slide(val id: String) {
        val view = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            clipChildren = false
        }
        /** Its name, in the shell's big bold type (ADR-0021), as its owner wrote it. */
        private val name = context.text(30f, 700, Theme.FG).apply {
            letterSpacing = -0.01f
            gravity = Gravity.CENTER
            includeFontPadding = false
        }
        val character: CharacterView? = if (id == NEW || id.startsWith(PIN)) null else CharacterView(context, context.dpi(CHAR_DP))
        private val ring: FrameLayout? = if (id == NEW) FrameLayout(context).apply {
            background = DashedRing(context)
            addView(ImageView(context).apply { setImageDrawable(Icons.drawable("plus", Theme.MUTED)) }, FrameLayout.LayoutParams(context.dpi(64f), context.dpi(64f), Gravity.CENTER))
        } else null
        private val status = context.text(17f, 650).apply {
            gravity = Gravity.CENTER
            setPadding(context.dpi(28f), 0, context.dpi(28f), 0)
            minHeight = context.dpi(24f)
        }
        /** A pinned app's tile (ADR-0044): its icon, big, on its accent; edit mode's badge takes it off home. */
        private val tileIcon = ImageView(context)
        private val unpin: View? = if (id.startsWith(PIN)) FrameLayout(context).apply {
            background = roundRect(Theme.BAD, context.dp(16f))
            addView(ImageView(context).apply { setImageDrawable(Icons.drawable("minus", Theme.ON_ACCENT)) }, FrameLayout.LayoutParams(context.dpi(18f), context.dpi(18f), Gravity.CENTER))
            visibility = View.GONE
            setOnClickListener { removePin(this@Slide.id.removePrefix(PIN)) }
        } else null
        private val tile: FrameLayout? = if (id.startsWith(PIN)) FrameLayout(context).apply {
            clipChildren = false
            addView(tileIcon, FrameLayout.LayoutParams(context.dpi(TILE_DP * 0.5f), context.dpi(TILE_DP * 0.5f), Gravity.CENTER))
            addView(unpin, FrameLayout.LayoutParams(context.dpi(32f), context.dpi(32f), Gravity.TOP or Gravity.START).apply {
                leftMargin = -context.dpi(8f); topMargin = -context.dpi(8f)
            })
        } else null
        private var on = false
        private var dragY = 0f

        /** A bump at an end: the character (or the ring) leans `dy` toward the edge, and springs back. */
        fun lean(dy: Float, animate: Boolean = true) {
            val v: View = character ?: ring ?: tile ?: return
            v.animate().cancel()
            if (!animate) {
                v.translationY = dy
                return
            }
            v.animate().translationY(dy).setDuration(90).setInterpolator(Theme.EASE_OUT)
                .withEndAction { v.animate().translationY(0f).setDuration(320).setInterpolator(OvershootInterpolator(1.6f)).start() }
                .start()
        }

        init {
            view.addView(name, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { bottomMargin = context.dpi(6f) })
            if (character != null) view.addView(character, LinearLayout.LayoutParams(context.dpi(CHAR_DP), context.dpi(CHAR_DP)))
            // New agent's ring sits where a character's body would (its shapes fill ~85% of their box).
            val ringDp = CHAR_DP * 0.85f
            val m = context.dpi((CHAR_DP - ringDp) / 2)
            if (ring != null) view.addView(ring, LinearLayout.LayoutParams(context.dpi(ringDp), context.dpi(ringDp)).apply { setMargins(m, m, m, m) })
            val tm = context.dpi((CHAR_DP - TILE_DP) / 2)
            if (tile != null) view.addView(tile, LinearLayout.LayoutParams(context.dpi(TILE_DP), context.dpi(TILE_DP)).apply { setMargins(tm, tm, tm, tm) })
            view.addView(status, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
            view.setOnClickListener {
                if (id == at) { if (!editing) openCurrent() } else { at = id; place(true) }
            }
            // A long press anywhere on home: edit mode (ADR-0044).
            view.setOnLongClickListener {
                if (!editing) enterEdit()
                true
            }
            // The agent on screen (ADR-0043): a tap pokes it, a stroke pets it; a long press is edit mode. A
            // neighbour's tap still selects it (the slide's click). Docked, the tap goes into its chat as well (T-156; Max,
            // 2026-10-13: "a single tap on the agent to also register as going into the chat"), as a click does.
            character?.let { c ->
                c.pokeable(enabled = { id == at && !editing }, onLong = { enterEdit() }, onTap = {
                    c.poke()
                    if (docked) talkTo()
                })
            }
            // Edit mode: the one on screen can be dragged up or down a place at a time, as the wheel moves it.
            view.setOnTouchListener { _, e ->
                // Where every touch starts: the long press that begins edit mode starts before it's on.
                if (e.actionMasked == android.view.MotionEvent.ACTION_DOWN) dragY = e.rawY
                if (!editing || id != at) return@setOnTouchListener false
                when (e.actionMasked) {
                    android.view.MotionEvent.ACTION_MOVE -> {
                        val dy = e.rawY - dragY
                        if (kotlin.math.abs(dy) > step * 0.4f) {
                            dragY = e.rawY
                            editMove(if (dy > 0) 1 else -1)
                            return@setOnTouchListener true
                        }
                    }
                }
                false
            }
            view.addOnLayoutChangeListener { v, _, _, _, _, _, _, _, _ ->
                v.pivotX = v.width / 2f
                v.pivotY = anchor()
            }
            name.alpha = 0f
            status.alpha = 0f
        }

        /** The character's centre, from the slide's top: what the wheel lines up. */
        fun anchor(): Float {
            val fm = name.paint.fontMetrics
            val nameH = (if (name.height > 0) name.height.toFloat() else fm.descent - fm.ascent) + context.dp(6f)
            return nameH + context.dp(CHAR_DP / 2)
        }

        fun bind(a: Agent) {
            name.text = a.name
            val b = builds[a.id]
            character!!.set(a.form, a.color, a.finish, charState(a, b), activity(a, b), a.unread, eyes = a.eyes, wear = a.wear)
            val (text, kind) = statusOf(a, b)
            status.text = text
            when (kind) {
                0 -> { status.setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 14f); status.typeface = Theme.font(500); status.setTextColor(Theme.MUTED) }
                2 -> { status.setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 17f); status.typeface = Theme.font(650); status.setTextColor(Theme.BAD) }
                else -> { status.setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 17f); status.typeface = Theme.font(650); status.setTextColor(Theme.FG) }
            }
        }

        /** A pinned app: its name, its icon on its accent (an Android app's own icon), and what it is. */
        fun bindPin(p: String) {
            if (p.startsWith(TOOL)) {
                // One of r1os's tools (T-091), in the look's accent.
                val t = TOOLS.firstOrNull { it.first == p.removePrefix(TOOL) }
                name.text = t?.second ?: p.removePrefix(TOOL)
                tileIcon.setImageDrawable(Icons.drawable(t?.third ?: "app-window", Theme.accent))
                tile?.background = roundRect(Theme.withAlpha(Theme.accent, 0.2f), context.dp(TILE_DP * 0.26f))
                status.text = "r1os"
            } else if (p.startsWith("android:")) {
                val app = allAndroid.firstOrNull { it.pkg == p.removePrefix("android:") }
                name.text = app?.name ?: p.removePrefix("android:")
                tileIcon.setImageDrawable(app?.icon(context))
                (tileIcon.layoutParams as FrameLayout.LayoutParams).apply { width = context.dpi(TILE_DP * 0.62f); height = width }
                tile?.background = roundRect(Theme.CARD, context.dp(TILE_DP * 0.26f))
                status.text = "android app"
            } else {
                val a = apps.firstOrNull { it.optString("id") == p }
                val accent = Theme.COLORS[a?.optString("accent").orEmpty()] ?: Theme.accent
                name.text = a?.optString("name") ?: p
                tileIcon.setImageDrawable(Icons.drawable(a?.optString("icon")?.ifEmpty { null } ?: "app-window", accent))
                tile?.background = roundRect(Theme.withAlpha(accent, 0.2f), context.dp(TILE_DP * 0.26f))
                status.text = a?.optString("description").orEmpty()
            }
            status.setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 14f)
            status.typeface = Theme.font(500)
            status.setTextColor(Theme.MUTED)
            status.maxLines = 2
        }

        fun setEditing(v: Boolean) {
            unpin?.visibility = if (v) View.VISIBLE else View.GONE
        }

        fun bindNew(full: Boolean) {
            name.text = "new agent"
            status.text = if (full) "That's ${shell.max}, the most there's room for" else ""
            status.setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, 14f)
            status.typeface = Theme.font(500)
            status.setTextColor(Theme.MUTED)
        }

        /** Only the agent on screen animates (the A53's budget). */
        fun setOn(v: Boolean) {
            character?.setActive(v)
            (ring?.background as? DashedRing)?.lit = v
        }

        fun setLabels(v: Boolean) {
            if (v == on) return
            on = v
            if (v) {
                name.animate().alpha(1f).setStartDelay(80).setDuration(260).start()
                status.animate().alpha(1f).setStartDelay(80).setDuration(260).start()
            } else {
                name.animate().alpha(0f).setStartDelay(0).setDuration(140).start()
                status.animate().alpha(0f).setStartDelay(0).setDuration(140).start()
            }
        }
    }

    companion object {
        /** The distance between agents on the wheel. */
        const val STEP_DP = 244f
        /** The agent on screen (Max, 2026-10-09: larger). Its neighbours are half this. */
        const val CHAR_DP = 212f
        /** Agents drawn on each side of the one on screen. */
        const val WINDOW = 2
        /** The id of the "New agent" entry after the agents on the wheel. */
        const val NEW = "new"
        /** A pinned app's entry on the wheel (ADR-0044): this, then its pin. */
        const val PIN = "pin:"
        /** A pinned app's tile. */
        const val TILE_DP = 150f
        /** A pin that's one of r1os's tools (T-091): this, then its id. */
        const val TOOL = "tool:"
        /** Closer to the top than this, the launcher sheet's corners square off. */
        const val SQUARE_DP = 120f
        /** Ticks past New agent that open the launcher; the ones before only pull (ADR-0018). */
        const val PULL_TICKS = 3
        /** Ticks past either end that only bump, before a pull starts (Max, 2026-10-09). */
        const val GUARD_TICKS = 3
        /** How far each guard tick leans the agent toward the edge. */
        val LEAN_DP = floatArrayOf(5f, 9f, 12f)
        /** The mic on home, docked (T-156). */
        const val MIC_DP = 48f
        /** How far each pulling tick raises the sheet: less each time, so it resists. */
        val PEEK_DP = floatArrayOf(64f, 112f)
        /** A pause this long lets a pull fall back. */
        const val RELAX_MS = 1100L

        val TOOLS = listOf(Triple("camera", "camera", "camera"), Triple("photos", "photos", "images"), Triple("recorder", "recorder", "microphone"),
            Triple("journal", "journal", "notebook"), Triple("translator", "translator", "translate"), Triple("browser", "browser", "globe"), Triple("whats_up", "what's up", "sparkle"), Triple("alarms", "alarms", "alarm"), Triple("reminders", "reminders", "check-square"), Triple("timer", "timer", "timer"), Triple("routines", "routines", "repeat"), Triple("inputs", "inputs", "hand-pointing"), Triple("clock", "clock", "clock"), Triple("system", "system", "cpu"))
        val SETTINGS = listOf(Triple("you", "about you", "user"), Triple("agents", "agents", "users-three"), Triple("look", "look", "palette"), Triple("home", "home", "house"), Triple("lock", "lock", "lock-simple"), Triple("accounts", "accounts", "key"), Triple("connections", "connections", "plugs-connected"), Triple("builds", "app builds", "hammer"), Triple("time", "date & time", "clock"), Triple("display", "display & sound", "sun"), Triple("controls", "controls", "hand-pointing"), Triple("computers", "computers", "desktop"), Triple("wifi", "wi-fi", "wifi-high"), Triple("bluetooth", "bluetooth", "bluetooth"), Triple("about", "about", "info"), Triple("power", "power", "power"))
        /** An agent's states with a turn in flight. */
        val BUSY = setOf("waiting", "thinking", "speaking", "working")
    }
}

/** New agent's dashed ring: brighter when it's on screen. */
private class DashedRing(context: Context) : Drawable() {
    private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = context.dp(3f)
        pathEffect = DashPathEffect(floatArrayOf(context.dp(9f), context.dp(6f)), 0f)
    }
    var lit = false
        set(v) { field = v; invalidateSelf() }

    override fun draw(c: Canvas) {
        p.color = Color.parseColor(if (lit) "#6a707b" else "#3a3e46")
        val b = bounds
        c.drawCircle(b.exactCenterX(), b.exactCenterY(), b.width() / 2f - p.strokeWidth, p)
    }

    override fun setAlpha(a: Int) {}
    override fun setColorFilter(cf: ColorFilter?) {}
    @Deprecated("Deprecated in Java")
    override fun getOpacity() = PixelFormat.TRANSLUCENT
}

/**
 * The wheel's page dots, on the right edge by the R1's wheel: the agent's colour, longer for the
 * one on screen, pulsing while it works, ringed while it has unread replies.
 */
private class Dots(context: Context) : View(context) {
    private var agents: List<Agent?> = emptyList()
    private var sel = 0
    private var heights = FloatArray(0)
    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val w = context.dp(6f)
    private val gap = context.dp(7f)
    private val tall = context.dp(18f)
    private var pulse = 1f
    private var active = false
    private val pulser = ValueAnimator.ofFloat(1f, 0.35f, 1f).apply {
        duration = 1200
        repeatCount = ValueAnimator.INFINITE
        interpolator = Theme.EASE_IN_OUT
        addUpdateListener { pulse = it.animatedValue as Float; invalidate() }
    }
    private var slide: ValueAnimator? = null

    fun set(list: List<Agent?>, selected: Int) {
        val changed = selected != sel || list.size != agents.size
        val from = if (heights.size == list.size) heights.copyOf() else FloatArray(list.size) { if (it == selected) tall else w }
        agents = list
        sel = selected
        if (changed) {
            slide?.cancel()
            slide = ValueAnimator.ofFloat(0f, 1f).apply {
                duration = 260
                interpolator = Theme.EASE_IN_OUT
                addUpdateListener { a ->
                    val t = a.animatedValue as Float
                    heights = FloatArray(list.size) { i -> from[i] + ((if (i == selected) tall else w) - from[i]) * t }
                    invalidate()
                }
                start()
            }
        } else {
            heights = FloatArray(list.size) { if (it == selected) tall else w }
        }
        updatePulse()
        invalidate()
    }

    fun setActive(on: Boolean) {
        active = on
        updatePulse()
    }

    private fun busy(a: Agent?) = a != null && a.state != "idle" && a.state != "error"

    private fun updatePulse() {
        val want = active && agents.any { busy(it) }
        if (want && !pulser.isStarted) pulser.start()
        if (!want && pulser.isStarted) { pulser.cancel(); pulse = 1f }
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        pulser.cancel()
    }

    override fun onDraw(c: Canvas) {
        if (agents.isEmpty()) return
        val total = heights.sum() + gap * (heights.size - 1)
        var y = height / 2f + context.dp(8f) - total / 2
        val cx = width / 2f
        val dark = Theme.SUBTLE
        agents.forEachIndexed { i, a ->
            val h = heights.getOrElse(i) { w }
            val colour = a?.let { Theme.colorOf(it.color) }
            val on = i == sel
            p.style = Paint.Style.FILL
            p.color = when {
                colour != null && (on || busy(a) || a.unread > 0) -> colour
                on -> Theme.MUTED
                else -> dark
            }
            p.alpha = if (busy(a) && !on) (pulse * 255).toInt() else 255
            val r = if (a == null) context.dp(1f) else w / 2
            c.drawRoundRect(cx - w / 2, y, cx + w / 2, y + h, r, r, p)
            if (a != null && a.unread > 0 && colour != null) {
                p.style = Paint.Style.STROKE
                p.strokeWidth = context.dp(1.5f)
                p.color = colour
                p.alpha = 255
                val o = context.dp(2.75f)
                c.drawRoundRect(cx - w / 2 - o, y - o, cx + w / 2 + o, y + h + o, r + o, r + o, p)
            }
            y += h + gap
        }
    }
}
