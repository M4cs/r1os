package dev.r1os.host.shell

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.SystemClock
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Agent
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.net.currentChat
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Look
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.traced
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * The native shell's root (ADR-0017; ported from shell/src/App.tsx).
 *
 *   first run → onboarding (OnboardingScreen)
 *   home      = your agents on the wheel, the launcher below them (HomeScreen)
 *   screens   = a chat, an app (its own WebView), an agent's space, settings, Manage apps, New app,
 *               a tool: stacked over home. Double tap goes back.
 *
 * Input comes only from r1d's event stream. The screen on top sees it first (home too). The press
 * that wakes the display is ignored: input is dropped while the shell is hidden and for
 * [WAKE_GUARD_MS] after it shows again.
 *
 * The glance screen (ADR-0019) is what the R1 wakes to: it goes up as the display goes off, and
 * takes the input until you turn up to home (or click to sleep again, or hold to talk).
 *
 * Hold to talk anywhere after onboarding: the overlay shows r1d's `voice` events. The final words
 * go to the screen on top first (home: the agent on screen; a chat: itself); otherwise to the agent
 * you last talked to, in its current conversation (or a new one).
 *
 * Over another Android app (ADR-0027), talking, the volume and reminders still work: they share a layer
 * ([Dock]) that moves into a window over that app. Your words go to the agent you last talked to, told
 * which app is in front, and its answer shows on a card there.
 */
class Shell(context: Context) : FrameLayout(context) {
    /** What the shell and its parts register with the process (r1d, notifications, the player): let go when it's
     *  destroyed (a change of look builds a new shell). First, so the parts made below can add to it. */
    private val offs = ArrayList<() -> Unit>()
    var agents: List<Agent> = emptyList()
        private set
    /** The agent you last talked to (r1d's focus). */
    var focus = ""
        private set
    var max = 8
        private set
    private var step: String? = null
    val connected get() = R1d.connected

    private val layer = FrameLayout(context)
    val home = HomeScreen(this)
    private val stack = ArrayList<Screen>()
    private val voice = VoiceOverlay(context)
    private val glance = GlanceView(context, home)
    private val volumeView = VolumeOverlay(context)
    /** Reminders, replies, talking and the volume: over other apps too (ADR-0027). */
    private val dock = Dock(this)
    /** The camera behind the words, while System One hears "what am I looking at" (ADR-0032). */
    private val peek = dev.r1os.host.camera.Viewfinder(context).apply { visibility = View.GONE }
    /** What the peek is open for: `look` or `selfie`. */
    private var peeking: String? = null
    /** Its photo is being taken: the peek stays until it's done. */
    private var shooting = false
    private val audio = context.getSystemService(android.media.AudioManager::class.java)
    /** The side button is down: the wheel belongs to the press (forgiven once, then volume). */
    private var buttonDown = false
    /** Net wheel ticks during this press, before it became a volume change. */
    private var pressTicks = 0
    private var volumeOn = false
    /**
     * The display went off: the glance screen goes up (declared before init, which registers it). On or off,
     * r1d hears of it: apps' background web data waits while it's off.
     */
    private val screenOff = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            val on = i.action == Intent.ACTION_SCREEN_ON
            R1d.post("/api/display", JSONObject().put("on", on))
            // The host's copy of Android's dim timer (T-036).
            if (on) dev.r1os.host.ui.Dim.on(c) else dev.r1os.host.ui.Dim.off()
            if (!on) wentOff() else cameOn()
        }
    }

    // ---- the PIN lock (ADR-0016) --------------------------------------------------------------------

    /** A PIN is set, and the R1 is locked now (r1d's word; it locks as the display goes off). */
    private var pinSet = false
    private var pinLocked = false
    /** "Lock after" (ms): how long the display may be off before it locks; and when it went off. */
    private var pinGrace = 0L
    private var offAt = 0L
    /** A double click woke the R1 before the shell was on screen: home (or the dial) once it is (ADR-0016). */
    private var wokeHome = 0L
    /** What waits for the code (Android asking to allow a computer). */
    private val afterUnlock = mutableListOf<() -> Unit>()

    private fun loadLock() = R1d.get("/api/lock") { r ->
        pinGrace = r.optLong("grace_s") * 1000
        setLocked(r.optBoolean("set"), r.optBoolean("locked"))
    }

    private fun setLocked(set: Boolean, locked: Boolean) {
        pinSet = set
        val was = pinLocked
        pinLocked = locked
        glance.locked = locked
        // r1d says locked (it restarted, or the shell did): the glance screen goes up.
        if (locked && !glance.up) lock()
        if (was && !locked) {
            glance.unlockedNow()
            afterUnlock.toList().also { afterUnlock.clear() }.forEach { it() }
            // An agent's request that waited for the code.
            loadApprovals()
        }
    }

    /** The code dialled on the glance screen: r1d says whether it's right. */
    private fun dialled(code: String) = R1d.post("/api/lock/unlock", JSONObject().put("pin", code), onError = { glance.dialWrong(it) }) {
        setLocked(pinSet, false)
    }

    /**
     * The display went off: the glance screen goes up. With a PIN it's locked at once (r1d too, as it hears), and r1os
     * comes in front of another app, so the R1 wakes to the dial and not to that app.
     */
    private fun wentOff() {
        bootAnimation()
        if (pinSet) {
            if (offAt == 0L) offAt = System.currentTimeMillis()
            if (pinGrace == 0L) {
                pinLocked = true
                glance.locked = true
            }
            if (!visible) comeBack()
        }
        lock()
    }

    // ---- the boot animation (T-029) -------------------------------------------------------------------

    private val renderer = java.util.concurrent.Executors.newSingleThreadExecutor()
    private var rendering = false

    /**
     * Your agents in the boot animation: drawn again (off the main thread) when the display goes off and their looks
     * have changed since it was last drawn, then kept by r1d (/api/bootanim) for the next boot. `default`: r1os's own
     * cast, for the image (tools/r1h bootanim).
     */
    private fun bootAnimation(default: Boolean = false) {
        if (rendering || (!default && agents.isEmpty())) return
        val cast = if (default) dev.r1os.host.ui.BootAnimation.DEFAULT
        else agents.map { dev.r1os.host.ui.BootAnimation.Look(it.form, it.color, it.finish, it.eyes, it.wear) }
        val sig = dev.r1os.host.ui.BootAnimation.signature(cast)
        val prefs = context.getSharedPreferences("r1os", Context.MODE_PRIVATE)
        if (!default && prefs.getString(BOOTANIM_SIG, null) == sig) return
        rendering = true
        val app = context.applicationContext
        renderer.execute {
            val zip = runCatching { dev.r1os.host.ui.BootAnimation.render(app, cast) }
                .onFailure { android.util.Log.w("r1os", "boot animation: couldn't draw it", it) }.getOrNull()
            R1d.main.post {
                if (zip == null) { rendering = false; return@post }
                R1d.upload("/api/bootanim" + if (default) "?default=1" else "", zip, "application/zip", onError = { rendering = false }) {
                    rendering = false
                    if (!default) prefs.edit().putString(BOOTANIM_SIG, sig).apply()
                }
            }
        }
    }

    /** The display came on: with a grace ("lock after"), locked if it was off longer than that (r1d agrees as it hears). */
    private fun cameOn() {
        if (pinSet && !pinLocked && offAt > 0 && System.currentTimeMillis() - offAt >= pinGrace) {
            pinLocked = true
            glance.locked = true
        }
        offAt = 0L
    }

    /** A double click woke it: past the glance screen, home; with the PIN on, the dial (ADR-0016). */
    private fun homeFromWake() {
        if (!glance.up || step != "done") return
        if (pinLocked) glance.openDial() else glance.complete()
    }
    private val toastView = LinearLayout(context)
    /** An app's reminder (ADR-0024) or an agent's (ADR-0028): over whatever is on screen, the glance screen included. */
    private val notice = NoticeBanner(
        context,
        onOpen = { app -> openFromNotice(app) },
        onSeen = { id -> seenReminder(id) },
        onOpenNote = { key -> openNote(key) },
        onOpenChat = { agent, chat -> openFromRoutine(agent, chat) },
        onSeenNotice = { id -> seenNotice(id) },
    )
    /** An alarm or a timer ringing (ADR-0028, ADR-0050): over everything, other apps too. */
    private val alarmView = AlarmView(context) { id, snooze, timer ->
        if (timer) R1d.post("/api/timers/${R1d.enc(id)}/${if (snooze) "add" else "stop"}")
        else R1d.post("/api/alarms/${R1d.enc(id)}/${if (snooze) "snooze" else "stop"}")
    }
    /** Your agent answering what you said over an app (ADR-0025): you stay in the app. */
    private val reply = AgentReplyCard(context) { openReplyChat() }
    /** The chat and agent that card is for. */
    private var overChat: String? = null
    private var overAgent: Agent? = null
    private val toastText = context.text(15f, 650)
    private val offline = context.text(13f, 600, Theme.MUTED)
    private val splash = context.text(16f, 500, Theme.MUTED)
    private var toastChat: Pair<String, String>? = null
    private val hideToast = Runnable { toastView.animate().alpha(0f).setDuration(200).withEndAction { toastView.visibility = View.GONE }.start() }
    private val errorTimers = HashMap<String, Runnable>()

    private var visible = false
    private var visibleSince = 0L
    private var wasConnected = false
    private var loadedAgents = false
    /** An agent to put on screen once the list has it (New agent). */
    private var selectNext: String? = null

    private var battery = -1
    private var charging = false
    private var wifiLevel = -1

    init {
        setBackgroundColor(Theme.BG)
        // The R1's browser while nobody's watching (ADR-0054): behind everything, so its pages still lay out and draw.
        addView(dev.r1os.host.AgentBrowser.backContainer(context), LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(layer, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        layer.addView(home.view, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        home.view.visibility = View.INVISIBLE

        splash.text = "Starting…"
        splash.gravity = Gravity.CENTER
        splash.setBackgroundColor(Theme.BG)
        addView(splash, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))

        toastView.apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(14f), context.dpi(8f), context.dpi(14f), context.dpi(8f))
            visibility = View.GONE
            setOnClickListener {
                val (agent, chat) = toastChat ?: return@setOnClickListener
                removeCallbacks(hideToast)
                hideToast.run()
                open(Target.of("chat", "agent" to agent, "id" to chat))
            }
        }
        addView(glance, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        glance.onCode = { dialled(it) }
        glance.onOpened = {
            dismissKeyguard()
            if (visible) home.onShow()
            updateDocked()
            // What asked while it was up (T-160).
            owedApprovals()
            // You're about to use it (T-043): the agent home lands on starts warming, so its first word comes sooner.
            warmFocus()
        }
        // Gone straight to something (T-128): Android's keyguard steps aside, and docked mode lets go.
        glance.onLeft = {
            dismissKeyguard()
            updateDocked()
            owedApprovals()
            // Opened while out of sight (an agent asked with the screen off): it's seen only now.
            owed?.takeIf { visible && it === top() }?.onShow()
            owed = null
        }
        // Pulled down from the top edge (T-124): the screen under it stops while it's open, and starts again after.
        voice.onCancel = { talkByTouch("cancel") }
        voice.onSend = { talkByTouch("send") }
        glance.onDrop = { open ->
            if (open) top().onHide() else if (visible) top().onShow()
            updateTilt()
        }
        dev.r1os.host.DeviceSettings.onDocked = { updateDocked() }
        // A music app came forward to play what an agent found (R7): r1os comes back once it's going.
        dev.r1os.host.MediaSessions.onLaunched = { if (visible) postDelayed({ comeBack() }, 3500) }

        addView(toastView, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER_HORIZONTAL).apply { topMargin = context.dpi(14f) })
        addView(dock, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        dock.addView(notice, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.TOP).apply {
            setMargins(context.dpi(10f), context.dpi(10f), context.dpi(10f), 0)
        })
        dock.addView(reply, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.BOTTOM).apply {
            setMargins(context.dpi(10f), 0, context.dpi(10f), context.dpi(10f))
        })
        dock.addView(alarmView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        dock.addView(peek, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        dock.addView(voice, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        dock.addView(volumeView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        // The pull to sleep (ADR-0022) folded home into the glance screen: lights out.
        glance.onLocked = {
            voice.hide()
            toHome()
            home.onHide()
            R1d.post("/api/screen", JSONObject().put("on", false))
        }

        offline.text = "Connecting to r1d…"
        offline.setPadding(context.dpi(12f), context.dpi(6f), context.dpi(12f), context.dpi(6f))
        offline.background = roundRect(Theme.CARD, context.dp(14f))
        addView(offline, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER_HORIZONTAL or Gravity.BOTTOM).apply { bottomMargin = context.dpi(76f) })

        // The display going off puts the glance screen up (and goes home under it), so it's
        // what the R1 wakes to. Registered for the shell's lifetime: it only fires on screen-off.
        context.registerReceiver(screenOff, IntentFilter(Intent.ACTION_SCREEN_OFF).apply { addAction(Intent.ACTION_SCREEN_ON) })
        dev.r1os.host.ui.Dim.roots = listOf(this, dock)
        val rest = { onRest() }
        dev.r1os.host.ui.Dim.watch(rest)
        keep { dev.r1os.host.ui.Dim.unwatch(rest) }
        if (displayOn()) dev.r1os.host.ui.Dim.on(context)

        // Android apps' notifications: a banner for one that alerts while the screen is on (it doesn't wake it: the
        // glance screen lists them when you do), over another app too.
        keep(dev.r1os.host.Notifications.onPosted { n ->
            if (step != "done" || !displayOn() || alarmView.ringing != null) return@onPosted
            if (!visible) dock.lift()
            placeNotice()
            notice.showNote(n)
        })

        // Android apps (ADR-0013): the launcher's list, and r1d's for agents and System One, follow installs.
        dev.r1os.host.AndroidApps.watch(context) {
            home.loadAndroid()
            if (R1d.connected) dev.r1os.host.AndroidApps.publish(context)
        }

        keep(R1d.onEvent(::handle))
        keep(R1d.onConnection(::connection))
        // Already connected (the process connects as it starts, T-142; or a change of look): what connecting does, now.
        if (R1d.connected) connection(true)
        // Agents' requests to the app window and the browser are the process's (T-142): the shell shows them.
        post { dev.r1os.host.Requests.attach(this) }
    }

    fun keep(off: () -> Unit) {
        offs += off
    }

    private fun connection(up: Boolean) {
            offline.visibility = if (up) View.GONE else View.VISIBLE
            if (!up) {
                home.render()
                return
            }
            // The look (ADR-0043): one changed while the shell was away (r1d kept it) builds it again.
            R1d.get("/api/look") { r -> r.optJSONObject("look")?.let { if (Look.update(context, it)) rebuild() } }
            loadAgents()
            loadStep()
            loadAlarms()
            loadTimers()
            loadLock()
            dev.r1os.host.MediaSessions.report()
            R1d.get("/api/glance") { glance.setWidgets(it.optJSONArray("widgets")) }
            dev.r1os.host.AndroidApps.publish(context)
            dev.r1os.host.Notifications.publishAll()
            R1d.post("/api/display", JSONObject().put("on", displayOn()))
            // The R1's own settings (ADR-0045): r1d hears them; an agent's request that's waiting shows.
            dev.r1os.host.DeviceSettings.connected(context)
            loadApprovals()
            // Talking and the volume over other apps (ADR-0027): an app-op only the system grants, so r1d does.
            if (!dock.allowed()) R1d.post("/api/android/overlay")
            // The camera (ADR-0031): a runtime permission r1d grants (a fresh install hasn't got it yet).
            if (context.checkSelfPermission(android.Manifest.permission.CAMERA) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                R1d.post("/api/android/permissions")
            }
            // r1d came back after a restart (a deploy, or a crash): pages may have changed under us.
            if (wasConnected) {
                // Kept app pages hold the old tokens.
                KeptPages.clear()
                home.onReconnect()
                stack.forEach { it.onReconnect() }
            }
            wasConnected = true
    }

    /** The look changed: the shell is built again in it, once this event's handling is done (MainActivity). */
    private fun rebuild() {
        android.os.Handler(android.os.Looper.getMainLooper()).post { (context as? dev.r1os.host.MainActivity)?.rebuildShell() }
    }

    /** The screens a rebuild opens again, bottom to top, with their focus: those a target opened. */
    fun saved(): List<Pair<Target, Int>> = stack.mapNotNull { s -> s.opener?.let { it to s.focusIndex() } }

    /** Opens them again, without animating, after a rebuild; the glance screen goes back up if it was (or the display
     *  is off), once the agents and setup's step are in. */
    fun restore(saved: List<Pair<Target, Int>>, glanceWasUp: Boolean) {
        for ((t, _) in saved) open(t, animate = false)
        saved.lastOrNull()?.let { (_, i) -> if (i >= 0) top().restoreFocus(i) }
        relock = glanceWasUp || !displayOn()
        maybeRelock()
    }

    /** The glance screen is up. */
    val glanceUp: Boolean get() = glance.up

    /** A rebuild left the glance screen to put back up. */
    private var relock = false

    private fun maybeRelock() {
        if (!relock || step != "done" || agents.isEmpty()) return
        relock = false
        lock()
    }

    // ---- data ---------------------------------------------------------------------------------------

    /** The agents from r1d; `then` runs once they're in (or the request failed). */
    private fun loadAgents(then: (() -> Unit)? = null) = R1d.get("/api/agents", onError = { then?.invoke() }) { r ->
        val list = r.optJSONArray("agents") ?: run { then?.invoke(); return@get }
        val old = agents.associateBy { it.id }
        agents = (0 until list.length()).mapNotNull { list.optJSONObject(it) }.map { j ->
            Agent.from(j).also { a -> old[a.id]?.let { a.tool = it.tool } }
        }
        focus = r.optString("focus")
        max = r.optInt("max", 8)
        selectNext?.let { id ->
            if (agent(id) != null) {
                selectNext = null
                home.select(id)
            }
        }
        if (!loadedAgents && agents.isNotEmpty()) {
            loadedAgents = true
            home.select(focus, animate = false)
            stack.forEach { it.onAgents() }
        } else {
            agentsChanged()
        }
        updateSplash()
        maybeRelock()
        then?.invoke()
    }

    /** Fetches the agents again (after onboarding or a redesign changed one); `then` once they're in. */
    fun reloadAgents(then: (() -> Unit)? = null) = loadAgents(then)

    /** One agent's state, tool or unread count changed, or the list did: everything showing them redraws. */
    private fun agentsChanged() {
        home.render()
        stack.forEach { it.onAgents() }
        if (glance.up) glance.setAgent(glanceAgent())
        glance.dockAgents()
        // An agent at work's step (T-151).
        glance.stepChanged()
    }

    private fun loadStep() = R1d.get("/api/onboarding", onError = { step = "done"; updateSplash() }) { r ->
        val s = r.optString("step", "done")
        step = s
        // First run: onboarding, over home.
        if (s != "done" && stack.none { it is OnboardingScreen }) {
            // Setup comes first: a glance screen still up from before (r1d reset while the R1 slept) goes.
            if (glance.up) {
                glance.dismiss()
                dismissKeyguard()
            }
            toHome()
            push(OnboardingScreen(this, s), animate = false)
        }
        updateSplash()
        maybeRelock()
    }

    /** Settings › About › Run setup again: onboarding from the start. */
    fun restartSetup() = loadStep()

    /** Onboarding finished: home, with the agent it set up. */
    fun setupDone() {
        step = "done"
        toHome()
        loadAgents()
    }

    /** r1d always has at least one agent: wait for the list rather than flash an empty wheel. */
    private fun updateSplash() {
        val ready = step != null && agents.isNotEmpty()
        splash.visibility = if (ready) View.GONE else View.VISIBLE
        splash.text = if (connected) "" else "Starting…"
        if (ready && home.view.visibility != View.VISIBLE) {
            home.view.visibility = View.VISIBLE
            if (stack.isEmpty() && visible) home.onShow()
            // Started while the display was off (a deploy, a crash): it'll wake to the glance screen.
            if (!displayOn()) lock()
        }
    }

    fun agent(id: String?): Agent? = agents.firstOrNull { it.id == id }

    /** The agent you last talked to: words spoken outside home or a chat go to it. */
    fun focusAgent(): Agent = agent(focus) ?: agents.firstOrNull() ?: Agent.DEFAULT

    /** Warms the focus agent's current chat (or a spare for a new one): its CLI starts while you decide what to say. */
    private fun warmFocus() {
        val a = focusAgent()
        if (a.id.isEmpty()) return
        currentChat(a.id) { c -> R1d.post("/api/chats/warm", JSONObject().put("agent", a.id).put("chat", c ?: JSONObject.NULL)) }
    }

    fun top(): Screen = stack.lastOrNull() ?: home

    // ---- navigation ---------------------------------------------------------------------------------

    fun open(t: Target, animate: Boolean = true) {
        when (t.t) {
            "android" -> { R1d.post("/api/android/open", JSONObject().put("screen", "settings")); return }
            "sleep" -> { sleep(); return }
            "home" -> {
                toHome()
                // A just-made agent: select it once the list has it.
                t.str("at")?.let { if (agent(it) != null) home.select(it) else { selectNext = it; loadAgents() } }
                return
            }
        }
        val id = t.str("id")
        val screen: Screen = traced("r1os open ${t.t}") { when (t.t) {
            "chat" -> ChatScreen(this, agent(t.str("agent")) ?: focusAgent(), id, t.str("say"), t.str("app"),
                t.str("images")?.split(',')?.filter { it.isNotEmpty() }.orEmpty())
            "camera" -> CameraScreen(this)
            // The R1's browser, yours (ADR-0054): "go to amazon.com" opens it on that page.
            "browser" -> { dev.r1os.host.AgentBrowser.openForYou(context, t.str("url")); BrowserScreen(this) }
            "app" -> AppScreen(this, id ?: return)
            "link" -> LinkScreen(this, t.str("url") ?: return)
            "agent" -> SpaceScreen(this, id?.takeIf { agent(it) != null } ?: return)
            "new-agent" -> newAgentPage(this)
            "agent-look" -> agentLookPage(this, id ?: return) ?: return
            "agent-can" -> AgentCanScreen(this, id?.takeIf { agent(it) != null } ?: return)
            "agent-where" -> AgentWhereScreen(this, id?.takeIf { agent(it) != null } ?: return)
            "chats" -> ManageChatsScreen(this, t.str("agent") ?: return)
            "memory" -> MemoryScreen(this, t.str("agent"))
            "manage" -> ManageAppsScreen(this)
            "app-info" -> AppInfoScreen(this, id ?: return, t.str("build")?.toLongOrNull())
            "create" -> CreateScreen(this)
            "tool" -> toolPage(this, id ?: return, t) ?: return
            "settings" -> settingsPage(this, t.str("page") ?: return) ?: return
            else -> return
        } }
        screen.opener = t
        // Voice can keep opening chats over apps: past a few screens, start again from home.
        if (stack.size >= MAX_DEPTH) toHome()
        // Under the push-to-talk overlay as it lifts away with your words: no slide of its own, so the chat is one
        // full-screen fade, not two at once (T-106: the GPU's slowest frames).
        val under = t.t == "chat" && voice.phase == VoiceOverlay.Phase.SENDING && voice.visibility == View.VISIBLE
        traced("r1os push") { push(screen, animate && !under) }
    }

    /** An Android app's notification: its app comes up, where it was (a chat, a delivery). */
    fun openNote(key: String) {
        if (pinLocked && glance.up) { glance.openDial { openNote(key) }; return }
        if (glance.up) {
            glance.dismiss()
            dismissKeyguard()
        }
        if (!dev.r1os.host.Notifications.open(key)) flash("couldn't open it")
    }

    /** An Android app full screen (ADR-0013); a double click comes back. */
    fun openAndroid(pkg: String) {
        if (pinLocked && glance.up) { glance.openDial { openAndroid(pkg) }; return }
        if (glance.up) {
            glance.dismiss()
            dismissKeyguard()
        }
        if (!dev.r1os.host.AndroidApps.open(context, pkg)) flash("that app isn't installed")
    }

    /** Android asks to allow a computer's adb key, or wireless debugging on a network: asked here, over everything. */
    fun askAdb(key: String?, fingerprints: String?, ssid: String?, bssid: String?) {
        if (key == null && bssid == null) return
        // Never past the lock: allowing a computer's adb would be a way in (ADR-0016).
        if (pinLocked) { afterUnlock += { askAdb(key, fingerprints, ssid, bssid) }; return }
        if (glance.up) {
            glance.dismiss()
            dismissKeyguard()
        }
        push(AdbConfirmScreen(this, key, fingerprints, ssid, bssid))
    }

    /**
     * An agent asks to change something on the R1 (ADR-0045): asked here, over everything. r1d woke the display; r1os
     * comes in front of another app for it. On the glance screen it waits as a row there instead (T-160), and the card
     * comes as you leave it (`now`: leaving it, or the row tapped).
     */
    private fun askApproval(a: JSONObject, now: Boolean = false) {
        val id = a.optString("id")
        if (step != "done" || stack.any { (it as? ApprovalScreen)?.approvalId == id }) return
        // It waits in r1d until the R1 is unlocked (setLocked loads it again).
        if (pinLocked) return
        // The glance screen up and r1os in front: a row there, like a notification (Max, 2026-10-13: "if its in the
        // background and we are in the glance screen it should be a notification. Otherwise it should show up and take
        // over the full screen like it does").
        if (glance.up && visible && !now) {
            if (approvalsWaiting.put(id, a) == null) {
                showWaiting()
                // At rest, it's back for it (T-157).
                dev.r1os.host.ui.Dim.poke()
            }
            return
        }
        approvalsWaiting.remove(id)?.let { showWaiting() }
        if (glance.up) {
            glance.dismiss()
            dismissKeyguard()
        }
        if (!visible) {
            runCatching {
                context.startActivity(android.content.Intent(context, dev.r1os.host.MainActivity::class.java)
                    .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK or android.content.Intent.FLAG_ACTIVITY_REORDER_TO_FRONT))
            }
        }
        push(ApprovalScreen(this, a))
    }

    /** Requests asked while the glance screen was up (T-160), oldest first: rows there, cards once it's left. */
    private val approvalsWaiting = LinkedHashMap<String, JSONObject>()

    /** The glance screen's left (T-160): what waited there comes up as cards, the newest on top. */
    private fun owedApprovals() {
        approvalsWaiting.values.toList().forEach { askApproval(it, now = true) }
    }

    /** What's waiting on the glance screen: requests first (T-160), then reminders and notices. */
    private fun showWaiting() {
        val asks = approvalsWaiting.values.map { a ->
            val ag = agent(a.optString("agent"))
            val order = a.optJSONObject("order")
            val total = order?.takeIf { !it.isNull("total") }?.optString("total")?.ifEmpty { null }
            GlanceView.Waiting(
                kind = "approval", id = a.optString("id"), agent = ag?.id, app = null,
                title = "${ag?.name ?: a.optString("name").ifEmpty { "an agent" }} is waiting for you",
                text = listOfNotNull(a.optString("title").ifEmpty { null }, total).joinToString(" · "),
                icon = if (order != null) "shopping-cart" else "hand-palm", color = Theme.colorOf(ag?.color),
                at = a.optLong("at").takeIf { it > 0 } ?: System.currentTimeMillis(),
            )
        }
        glance.setWaiting(asks + waiting)
    }

    /** The oldest request still waiting (on connecting, and on waking: one the display's sleeping put away). */
    private fun loadApprovals() {
        R1d.get("/api/approvals") { r -> r.optJSONArray("approvals")?.optJSONObject(0)?.let { askApproval(it) } }
    }

    /** A screen under the top one goes, without animating (an approval nobody answered in time). */
    fun remove(s: Screen) {
        if (s === top()) { back(); return }
        if (!stack.remove(s)) return
        layer.removeView(s.view)
        s.onDestroy()
    }

    /** A screen pushed while the shell was out of sight: shown once it's seen, past the glance screen ([glance]'s onLeft). */
    private var owed: Screen? = null

    fun push(s: Screen, animate: Boolean = true) {
        if (dockedOn) s.onDock(true)
        val under = top()
        if (visible) under.onHide()
        stack += s
        layer.addView(s.view, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        val v = s.view
        if (animate) {
            v.alpha = 0f
            v.translationX = context.dp(28f)
            // After its first frame (a post waits behind the traversal): building and measuring a page is one long
            // frame on the R1, and the slide shouldn't start in it and skip ahead (T-106).
            v.post {
                // Gone already (back before its first frame): its way out is animating, not this.
                if (s !in stack) return@post
                // As one GPU layer, drawn once: fading the whole page over home was an offscreen pass every frame,
                // ~45 ms each on the R1's GPU (T-106). Its rows are in by now, so the layer holds still.
                v.animate().alpha(1f).translationX(0f).setDuration(240).setInterpolator(Theme.SETTLE).withLayer()
                    .withEndAction { if (top() !== under) under.view.visibility = View.INVISIBLE }.start()
            }
        } else {
            under.view.visibility = View.INVISIBLE
        }
        if (visible) s.onShow() else owed = s
        updateTilt()
    }

    /** Back to whatever was underneath (double tap, Back). */
    fun back() {
        if (stack.isNotEmpty()) Sounds.play(Sounds.S.BACK)
        val s = stack.removeLastOrNull() ?: return
        s.onHide()
        val under = top()
        under.view.visibility = View.VISIBLE
        if (visible) under.onShow()
        updateTilt()
        val v = s.view
        // As one GPU layer: the screen going is still, so it's drawn once, not into an offscreen pass each frame (T-041:
        // back to home 19 → 11 ms a frame). Not on the way in, where a screen's rows still change as it slides.
        v.animate().alpha(0f).translationX(context.dp(28f)).setDuration(180).setInterpolator(Theme.EASE_IN_OUT).withLayer().withEndAction {
            layer.removeView(v)
            s.onDestroy()
        }.start()
    }

    /** Home, closing every screen. */
    fun toHome() {
        if (stack.isEmpty()) { home.reset(); return }
        val gone = ArrayList(stack)
        stack.clear()
        gone.last().onHide()
        home.reset()
        home.view.visibility = View.VISIBLE
        if (visible) home.onShow()
        updateTilt()
        for (s in gone) {
            s.view.animate().alpha(0f).setDuration(160).withEndAction {
                layer.removeView(s.view)
                s.onDestroy()
            }.start()
        }
    }

    /**
     * Turns the display off (r1d sends Android the key); the side button wakes it. The glance screen
     * goes up first, so that's what the R1 wakes to.
     */
    fun sleep() {
        Sounds.play(Sounds.S.SLEEP)
        lock()
        R1d.post("/api/screen", JSONObject().put("on", false))
    }

    /**
     * Turning up past the first agent (ADR-0022): home folds back into the glance screen, a tick at
     * a time, and the R1 sleeps when it's done.
     */
    fun pullToSleep() {
        if (step != "done") return
        val a = agent(home.agentId) ?: return
        glance.lockTick(a)
    }

    // ---- volume (ADR-0014, ADR-0022) ----------------------------------------------------------------

    private fun startVolume() {
        volumeOn = true
        voice.hide()
        val max = audio.getStreamMaxVolume(android.media.AudioManager.STREAM_MUSIC)
        volumeView.show(audio.getStreamVolume(android.media.AudioManager.STREAM_MUSIC), max)
        // The ticks that made it a volume change count: up is louder.
        stepVolume(-pressTicks)
        pressTicks = 0
    }

    /** `steps` up (positive) or down; Android's media volume, without its own panel. */
    private fun stepVolume(steps: Int) {
        if (steps == 0) return
        val stream = android.media.AudioManager.STREAM_MUSIC
        val max = audio.getStreamMaxVolume(stream)
        val cur = audio.getStreamVolume(stream)
        val next = (cur + steps).coerceIn(0, max)
        if (next != cur) audio.setStreamVolume(stream, next, 0)
        volumeView.set(next)
        if (next != cur) Sounds.level(next, max) else Sounds.play(Sounds.S.BUMP)
    }

    private fun endVolume() {
        volumeOn = false
        volumeView.hideSoon()
        dev.r1os.host.DeviceSettings.reportSoon()
    }

    /** Home under the glance screen (ADR-0019). Not during onboarding. */
    private fun lock() {
        if (step != "done" || agents.isEmpty()) return
        voice.hide()
        removeCallbacks(hideToast)
        toastView.visibility = View.GONE
        notice.hide(animate = false)
        reply.hide(animate = false)
        volumeView.hideNow()
        peekClose()
        dock.land()
        // Home, on the glance screen's agent: that's the character it morphs onto.
        val a = glanceAgent()
        // An agent's request keeps waiting in r1d: it shows again when the R1 wakes.
        stack.filterIsInstance<ApprovalScreen>().forEach { it.putAway() }
        toHome()
        if (agent(a.id) != null) home.select(a.id, animate = false)
        home.onHide()
        glance.show(a)
        glance.onDeviceStatus(battery, charging, wifiLevel)
        if (visible && displayOn()) glance.setDisplayOn(true)
        updateDocked()
    }

    /** The agent you were last with: the one on home's wheel, else the one you last talked to. */
    private fun glanceAgent(): Agent = agent(home.agentId) ?: focusAgent()

    /** Android's own (swipe) keyguard: the glance screen stands in for it, so it's dismissed. */
    private fun dismissKeyguard() {
        val a = context as? android.app.Activity ?: return
        val km = a.getSystemService(android.app.KeyguardManager::class.java)
        if (km.isKeyguardLocked && !km.isKeyguardSecure) km.requestDismissKeyguard(a, null)
    }

    // ---- events -------------------------------------------------------------------------------------

    private fun displayOn() = context.getSystemService(android.os.PowerManager::class.java).isInteractive

    private fun justWoke() = !visible || SystemClock.uptimeMillis() - visibleSince < WAKE_GUARD_MS

    private fun handle(ev: Ev) {
        // The player (ADR-0040): what r1d asks of it, and the song a station says is on.
        when (ev.type) {
            "android_notice" -> { dev.r1os.host.Notifications.onEvent(ev.str("op"), ev.str("key"), ev.str("text")); return }
            "media" -> { dev.r1os.host.Player.onEvent(context, ev); return }
            "player" -> { ev.json.optJSONObject("player")?.let { dev.r1os.host.Player.onState(it) }; return }
            // An Android app's music (R7): an agent's control_media or play_music.
            "android_media" -> { dev.r1os.host.MediaSessions.onEvent(context, ev); return }
            // The browser, the app window and the CPU held for a routine: the process takes them (T-142, Requests), and
            // calls [request] for what the shell shows.
            "browser", "app_window", "awake" -> return
            // How the harness says the R1 is held (T-156: docked mode tried without hands); null: the accelerometer again.
            "tilt" -> {
                forcedSide = if (ev.json.isNull("side")) null else ev.int("side").coerceIn(-1, 1)
                forcedSide?.let { side -> updateTilt(); if (tiltFor != null) setSideways(side) }
                return
            }
            // The harness holds the glance screen's rest at a step (T-157: each measured without waiting), and may try
            // another low brightness; null: its own timer again.
            "rest" -> {
                ev.json.optDouble("brightness").takeIf { !it.isNaN() }?.let {
                    restBrightness = it.toFloat().coerceIn(0f, 1f)
                    if (lowOn) { lowOn = false; restLow(true) }
                }
                dev.r1os.host.ui.Dim.force(if (ev.json.isNull("step") || !ev.json.has("step")) null else ev.int("step").coerceIn(0, 2))
                return
            }
            // One of r1os's own screens, by an agent's open_screen (R7).
            "open_screen" -> { ev.json.optJSONObject("target")?.let { openScreen(Target(it)) }; return }
            // The look (ADR-0043): the shell is built again in it.
            "look" -> { ev.json.optJSONObject("look")?.let { if (Look.update(context, it)) rebuild() }; return }
            // The glance screen's widgets (ADR-0042), all of them, whenever one changes.
            "glance" -> { glance.setWidgets(ev.json.optJSONArray("widgets")); return }
            // The apps pinned to home (ADR-0044).
            "home" -> { home.setPins(ev.json.optJSONArray("pins")); return }
            // An agent asks to change something (ADR-0045), or stopped waiting; a change you approved, made here.
            "approval_asked" -> { ev.json.optJSONObject("approval")?.let { askApproval(it) }; return }
            "approval_done" -> {
                stack.filterIsInstance<ApprovalScreen>().firstOrNull { it.approvalId == ev.str("id") }?.done()
                // Answered elsewhere, or nobody did in time: its row on the glance screen goes (T-160).
                ev.str("id")?.let { approvalsWaiting.remove(it) }?.let { showWaiting() }
                return
            }
            "device_set" -> { ev.json.optJSONObject("changes")?.let { dev.r1os.host.DeviceSettings.apply(context, it) }; return }
            // A linked computer changed something here (ADR-0048): a word at the top, so it's never a surprise.
            "changed_from" -> { flash("${ev.str("what")} changed from ${ev.str("computer")}"); return }
        }
        // The background's flow (ADR-0043): the wheel and the button nudge it, on the glance screen and on home.
        if (ev.type == "wheel" || ev.type == "button") {
            val wheel = ev.type == "wheel"
            when {
                glance.up -> if (wheel) glance.nudge(ev.int("delta")) else glance.press(ev.bool("down"))
                stack.isEmpty() -> if (wheel) home.nudge(ev.int("delta")) else home.press(ev.bool("down"))
            }
        }
        // A reminder goes as soon as you touch the wheel or the button (the input still does its job).
        if (notice.shown && (ev.type == "wheel" || ev.type == "gesture") && (!justWoke() || dock.over)) {
            notice.reminder?.let { seenReminder(it) }
            notice.appNotice?.let { seenNotice(it) }
            notice.hide()
        }
        // So does an agent's answer over an app, once it's in.
        if (reply.shown && reply.answered && ev.type == "wheel") reply.hide()
        // What you said over an app: the agent working, then its answer, on the card.
        val over = overAgent
        if (ev.type == "chat" && over != null && ev.str("chat") == overChat) {
            when (ev.str("kind")) {
                "tool" -> if (!reply.answered) reply.working(over, ev.str("text")?.takeIf { it.isNotEmpty() }?.let { ChatScreen.toolLabel(it) } ?: "Working on it…")
                "done" -> {
                    reply.reply(over, ev.str("text").orEmpty())
                    overChat?.let { R1d.post("/api/chats/$it/seen") }
                }
                "error" -> reply.reply(over, ev.str("text") ?: "Something went wrong", error = true)
            }
        }
        // What each agent is doing, and changes to the list. An error shows for a moment.
        when (ev.type) {
            "notify" -> if (step == "done") {
                // At rest, it's back for it (T-157).
                dev.r1os.host.ui.Dim.poke()
                // Another app in front: over it (ADR-0027).
                if (!visible && displayOn()) dock.lift()
                placeNotice()
                notice.show(ev)
                loadWaiting()
            }
            "alarm" -> {
                val id = ev.str("id").orEmpty()
                if (ev.bool("ringing")) ring(id, ev.str("label").orEmpty(), ev.str("agent")) else alarmView.stopped(id)
            }
            "reminder" -> {
                showReminder(ev.str("id").orEmpty(), ev.str("text").orEmpty(), ev.str("agent"))
                loadWaiting()
            }
            "alarms_changed" -> loadAlarms(again = false)
            // tools/r1h bootanim: r1os's own cast, drawn for the image (T-029).
            "bootanim" -> if (step == "done") bootAnimation(default = ev.bool("default"))
            "timer" -> {
                val id = ev.str("id").orEmpty()
                if (ev.bool("ringing")) ringTimer(id, ev.str("label").orEmpty(), ev.str("agent")) else alarmView.stopped(id)
            }
            "timer_phase" -> showPhase(ev)
            "routine" -> if (step == "done") {
                // Not over its own chat, already open in front of you: the reply is there (T-092).
                val looking = visible && displayOn() && !glance.up && (top() as? ChatScreen)?.chatId == ev.str("chat")
                if (!looking) agent(ev.str("agent"))?.let { a ->
                    if (!visible && displayOn()) dock.lift()
                    placeNotice()
                    notice.showRoutine(ev.str("title").orEmpty(), ev.str("text").orEmpty(), a, ev.str("chat").orEmpty())
                }
            }
            "timers_changed" -> loadTimers()
            "lock" -> loadLock()
            // System One while you talk: the camera gets ready behind the words (ADR-0032).
            "intent" -> if (step == "done") {
                val kind = ev.json.optJSONObject("intent")?.optString("kind")
                if (kind == "look" || kind == "selfie") peekOpen(kind) else peekClose()
            }
            // The Recorder keeps going with the screen off: the host holds the CPU awake while it does.
            "recording" -> dev.r1os.host.MicService.recording(context, ev.str("state") == "recording")
            // An app's link or copy step, checked by r1d: only from the app on screen.
            "open_link" -> if ((top() as? AppScreen)?.appId == ev.str("app") && visible) {
                ev.str("url")?.let { open(Target.of("link", "url" to it)) }
            }
            "copy" -> if ((top() as? AppScreen)?.appId == ev.str("app") && visible) copy(ev.str("text").orEmpty())
            "agents_changed" -> loadAgents()
            // The camera's motor: where it's going, and where it came to rest (ADR-0046).
            "motor" -> dev.r1os.host.camera.Motor.onEvent(ev)
            "chat" -> if (ev.str("kind") == "tool") agent(ev.str("agent"))?.let { it.tool = ev.str("text"); agentsChanged() }
            "unread" -> {
                val a = agent(ev.str("agent"))
                if (a != null) {
                    a.unread = ev.int("count")
                    // The glance screen shows the newest reply's agent (and its words), kept current while it's down
                    // too: read on home, a reply's page is gone when you turn back up (T-093).
                    glance.loadReplies()
                    // Another agent's message still waiting for you, or none now (ADR-0012): home's status line.
                    a.mail = ev.json.optJSONObject("mail")
                    agentsChanged()
                }
                val chat = ev.str("chat")
                // The browser that chat's agent is using shows its reply too (T-090).
                val onScreen = (top() as? ChatScreen)?.chatId ?: (top() as? BrowserScreen)?.chat ?: (top() as? AppWindowScreen)?.chat
                if (a != null && chat != null && ev.int("count") > 0 && chat != onScreen && chat != overChat && step == "done" && !glance.up) toast(a, chat, agent(ev.str("from")))
            }
            // One agent messaged another (ADR-0012): the recipient's status line shows it.
            "mail" -> loadAgents()
            "agent_state" -> agent(ev.str("agent"))?.let { a ->
                val state = ev.str("state") ?: "idle"
                errorTimers.remove(a.id)?.let { removeCallbacks(it) }
                // r1d makes the agent you talk to the focus; one another agent woke doesn't take it.
                if (state == "thinking" && ev.str("by") == null) focus = a.id
                a.state = state
                a.by = if (state == "idle" || state == "error") null else ev.str("by")
                a.chat = if (state == "idle") null else ev.str("chat")
                if (state == "idle" || state == "error") a.tool = null
                if (state == "error") {
                    val r = Runnable { if (a.state == "error") { a.state = "idle"; a.chat = null; agentsChanged() } }
                    errorTimers[a.id] = r
                    postDelayed(r, 2500)
                }
                agentsChanged()
            }
        }
        home.onAnyEvent(ev)
        stack.toList().forEach { it.onAnyEvent(ev) }
        KeptPages.onEvent(ev)

        // Hold + wheel (ADR-0014, ADR-0022): while the button is down the wheel belongs to the press.
        // One tick is forgiven; r1d makes the second a volume change (`volume`), and from then on
        // each tick steps the volume. None of it moves what's on screen.
        when (ev.type) {
            "button" -> {
                buttonDown = ev.bool("down")
                if (buttonDown) pressTicks = 0
            }
            "volume" -> {
                if (ev.bool("on")) {
                    // Over another app (ADR-0027).
                    if (!visible && displayOn()) dock.lift()
                    startVolume()
                } else {
                    endVolume()
                }
                return
            }
            "wheel" -> if (buttonDown) {
                if (volumeOn) stepVolume(-ev.int("delta")) else pressTicks += ev.int("delta")
                return
            }
        }

        // An alarm ringing takes the click (stop) and the double click (snooze), wherever you are (ADR-0028).
        val ringing = alarmView.ringing
        if (ringing != null && displayOn()) {
            when {
                ev.isTap -> { alarmView.end(ringing, snooze = false); return }
                ev.isDoubleTap -> { alarmView.end(ringing, snooze = true); return }
                ev.type == "wheel" -> return
            }
        }

        // Another app is in front (Android's settings, an app you installed).
        if (!visible && step == "done" && displayOn()) {
            when {
                // A double click comes back to r1os, from anywhere (architecture.md): into the chat if an
                // agent's answer is showing over the app.
                ev.isDoubleTap -> {
                    Sounds.play(Sounds.S.BACK)
                    // r1os's keyboard is up (ADR-0055): it goes first, and you stay in the app.
                    val kb = dev.r1os.host.ime.R1Keyboard.current
                    if (kb != null && kb.shown) { kb.hideKeyboard(); return }
                    if (reply.shown && overChat != null) openReplyChat() else comeBack()
                    return
                }
                // A click is the app's DPAD center (Q24); the wheel already reaches it as DPAD up and down.
                ev.isTap -> { R1d.post("/api/android/key", JSONObject().put("key", "center")); return }
                // Hold to talk works over it too (ADR-0027).
                ev.type == "voice" -> { onVoice(ev, top()); return }
                ev.type == "voice_level" -> { voice.level(ev.num("level").toFloat()); return }
                ev.gesture == "hold_end" -> { voice.released(); return }
            }
        }
        // A triple press: its shortcut (r1d sends it with the gesture), asleep or not.
        if (ev.gesture == "triple_tap") {
            ev.str("shortcut")?.let { if (alarmView.ringing == null) shortcut(it) }
            return
        }
        // A double click that woke the R1 (r1d marks it: its first press came with the display off) goes straight home,
        // past the glance screen, or to the dial with the PIN on (ADR-0016). The wake guard below would drop it.
        if (ev.isDoubleTap && ev.bool("woke")) {
            if (alarmView.ringing == null) {
                if (visible) homeFromWake() else wokeHome = SystemClock.uptimeMillis()
            }
            return
        }
        // The wake guard, and the wheel too: a turn while the display is off (in a pocket) mustn't
        // move anything you can't see, the glance screen included.
        if ((ev.type == "gesture" || ev.type == "button" || ev.type == "wheel") && justWoke()) return
        if (step == null || agents.isEmpty()) return
        // r1os's keyboard is up over one of r1os's own screens (ADR-0055): the wheel moves the cursor, a click is the
        // field's action, a double click puts it away. Holding the button still talks (it types what you say).
        dev.r1os.host.ime.R1Keyboard.current?.takeIf { it.shown && visible }?.let { kb ->
            when {
                ev.type == "wheel" -> { kb.moveCursor(ev.int("delta")); return }
                ev.isTap -> { kb.action(); return }
                ev.isDoubleTap -> { kb.hideKeyboard(); return }
            }
        }
        val top = top()
        // Onboarding: it takes the input (there's no talking until it's done).
        if (step != "done") {
            if (ev.type in INPUT_EVENTS) top.onEvent(ev)
            return
        }

        when (ev.type) {
            "gesture" -> if (ev.gesture == "hold_end") voice.released()
            "voice_level" -> { voice.level(ev.num("level").toFloat()); return }
            "voice" -> { onVoice(ev, top); return }
        }
        // The glance screen takes the input while it's up. Turning up pulls it, against resistance.
        if (glance.up) {
            // Locked (ADR-0016): the dial takes the wheel and the button while it's open. Turning up or a double click
            // opens it (the way home); a click still sleeps, and turning down still shows the next widget.
            // Docked on its side (T-117): the wheel, along the bottom now, turns the pages left and right; it never pulls.
            if (glance.docked && ev.type == "wheel" && !glance.dialOpen) {
                glance.turnPage(if (ev.int("delta") > 0) 1 else -1)
                return
            }
            if (pinLocked && !glance.locking) {
                if (glance.dialOpen) { glance.dialInput(ev); return }
                when {
                    ev.type == "wheel" && ev.int("delta") < 0 -> glance.openDial()
                    ev.type == "wheel" -> glance.pullDown()
                    ev.isTap -> sleep()
                    ev.isDoubleTap -> glance.openDial()
                }
                return
            }
            // Mid-fold toward sleep: up folds further, down unfolds, a click lets it go back to home.
            if (glance.locking) {
                when {
                    ev.type == "wheel" && ev.int("delta") < 0 -> glance.lockTick(glance.agent)
                    ev.type == "wheel" -> glance.lockBack()
                    ev.isTap || ev.isDoubleTap -> glance.relaxLock()
                }
                return
            }
            when {
                ev.type == "wheel" && ev.int("delta") < 0 -> glance.pullUp()
                ev.type == "wheel" -> glance.pullDown()
                ev.isTap -> sleep()
                ev.isDoubleTap -> glance.complete()
            }
            return
        }
        if (ev.type == "open") {
            ev.str("app")?.let { open(Target.of("app", "id" to it)) }
            return
        }
        if (top.onEvent(ev)) return
        if (ev.isDoubleTap && top !== home) back()
    }

    /**
     * Who an utterance goes to, fixed the moment the hold starts: the agent whose face the overlay
     * shows. Deciding again when the words land (~0.7 s after you let go) let the two disagree, and
     * words chosen for one agent went to the one you'd last talked to (2026-10-09).
     */
    private var talkTo: Agent? = null
    /** The hold started with r1os's keyboard up: the words are typed (ADR-0055). */
    private var dictating = false
    private var talkOnGlance = false
    /** The hold started over another Android app (ADR-0027). */
    private var talkOverAndroid = false

    private fun onVoice(ev: Ev, top: Screen) {
        val text = ev.str("text")?.trim().orEmpty()
        // The player dips while you talk (ADR-0040): the mic hears you, not it.
        dev.r1os.host.Player.duck(ev.str("state") == "listening" || ev.str("state") == "partial")
        when (ev.str("state")) {
            "listening" -> {
                // Over another app: the one you last talked to. Else the glance screen's agent; else the
                // agent on screen (home's, a chat's, a space's); else (New agent, the launcher, an app,
                // settings) the one you last talked to.
                val android = !visible && displayOn()
                val a = when {
                    android -> focusAgent()
                    glance.up -> glance.agent
                    else -> agent(top.agentId) ?: focusAgent()
                }
                talkTo = a
                talkOnGlance = glance.up && !android
                talkOverAndroid = android
                // The keyboard's up: what you say is typed into the field, not sent to an agent (ADR-0055).
                dictating = dev.r1os.host.ime.R1Keyboard.current?.shown == true
                // Without Android's leave to draw over other apps, r1os comes forward to listen, and your
                // words go to a chat as usual.
                if (android && !dock.lift()) comeBack()
                voice.setAgent(a)
                // Over the R1's browser or the app window, small in a corner: what you're talking about stays in view (T-138).
                voice.listen(small = !android && !glance.up && (top is BrowserScreen || top is AppWindowScreen))
                // Docked (T-156): cancel on the overlay, and send for a talk the mic on home started.
                voice.touchControls(cancel = dockedOn || touchTalking, send = touchTalking)
                // Start it while you talk, so the answer comes sooner. A chat and a space warm their
                // own session; on New app and an app's page the words go to a build, not a chat.
                val own = !android && !glance.up && (top is ChatScreen || top is SpaceScreen || top is CreateScreen || top is AppInfoScreen || top is NewReminderScreen)
                if (!own && a.id.isNotEmpty()) currentChat(a.id) { c -> R1d.post("/api/chats/warm", JSONObject().put("agent", a.id).put("chat", c ?: JSONObject.NULL)) }
            }
            "partial" -> voice.partial(text)
            "cancelled" -> { voice.hide(); talkTo = null; talkOverAndroid = false; touchTalking = false; peekClose() }
            "error" -> { voice.error(text.ifEmpty { "Something went wrong" }, 2600); talkTo = null; talkOverAndroid = false; touchTalking = false; peekClose() }
            "final" -> { touchTalking = false; if (text.isEmpty()) voice.error("Didn't catch that", 1600) else voice.sending(text) }
        }
        if (ev.str("state") != "final" || text.isEmpty()) return
        // Dictation: the words go into the field where the cursor is; nothing else happens with them. Decided when the
        // hold starts, or now if the keyboard's up (words that came without a hold: the harness).
        if (dictating || dev.r1os.host.ime.R1Keyboard.current?.shown == true) {
            dictating = false
            talkTo = null
            talkOverAndroid = false
            voice.hide()
            dev.r1os.host.ime.R1Keyboard.current?.dictate(text)
            return
        }
        // A new reminder (T-069): the words are what to remember.
        if (top is NewReminderScreen) {
            talkTo = null
            talkOverAndroid = false
            voice.hide()
            top.hear(text)
            return
        }
        // An agent's approval card (T-055): the words answer it ("allow", "always", "don't"), or it asks again.
        if (top is ApprovalScreen) {
            talkTo = null
            talkOverAndroid = false
            top.say(text) { ok -> if (ok) voice.hide() else voice.error(if (top.offersAlways) "Say “allow”, “always” or “don't”" else "Say “allow” or “don't”", 2600) }
            return
        }
        // The R1's browser on screen (ADR-0054): System One does what you said at once ("scroll down", "click sign in",
        // "open the second one"), or the agent using it gets the words, in the chat it's browsing for.
        if (top is BrowserScreen) {
            talkTo = null
            talkOverAndroid = false
            R1d.post("/api/browser/command", JSONObject().put("text", text), onError = { voice.error(it, 2400) }) { r ->
                if (r.optBoolean("handled")) {
                    voice.hide()
                    flash(r.optString("said"))
                } else {
                    val owner = agent(dev.r1os.host.AgentBrowser.agent) ?: focusAgent()
                    val chat = dev.r1os.host.AgentBrowser.chat
                    // That chat right underneath (you opened the browser from it): the words go there and you stay
                    // on the page, its reply in the bubble (T-090).
                    val under = stack.getOrNull(stack.size - 2) as? ChatScreen
                    when {
                        chat != null && under?.chatId == chat && under.say(text) -> voice.hide()
                        chat != null -> open(Target.of("chat", "agent" to owner.id, "id" to chat, "say" to text))
                        else -> talk(owner, text, null)
                    }
                }
            }
            return
        }
        // The app window on screen (ADR-0061): the words go to the chat using the app, and you stay on the app, its reply
        // in the bubble over it (T-138), as in the browser.
        if (top is AppWindowScreen) {
            talkTo = null
            talkOverAndroid = false
            val owner = agent(dev.r1os.host.AgentApps.agent) ?: focusAgent()
            val chat = dev.r1os.host.AgentApps.chat
            val under = stack.getOrNull(stack.size - 2) as? ChatScreen
            when {
                chat != null && under?.chatId == chat && under.say(text) -> voice.hide()
                chat != null -> R1d.post("/api/chats/$chat/messages", JSONObject().put("text", text), onError = { voice.error(it, 2400) }) { voice.hide() }
                else -> talk(owner, text, null)
            }
            return
        }
        val a = talkTo ?: agent(top.agentId) ?: focusAgent()
        val onGlance = talkOnGlance
        talkTo = null
        // System One was sure what you meant (ADR-0032): done without waiting for an agent. Not over a camera
        // screen, which takes the words with its own photo.
        val intent = ev.json.optJSONObject("intent")
        if (intent != null && top !is CameraScreen) {
            talkOverAndroid = false
            val go = { act(intent, text, a, top) }
            if (onGlance && glance.up && intent.optString("kind") !in setOf("look", "selfie", "journal", "record", "stop_record", "run", "media", "timer", "pomodoro")) {
                glance.complete { go() }
            } else {
                go()
            }
            return
        }
        peekClose()
        // Over another app you stay in it: the answer shows on a card over it (ADR-0027). Unless r1os came
        // forward meanwhile (the hold woke the R1 into it): then as usual.
        val android = talkOverAndroid && !visible
        talkOverAndroid = false
        if (android) {
            talkOver(a, text, null)
            return
        }
        // From the glance screen: home first (the morph), then the chat opens over it.
        if (onGlance && glance.up) {
            glance.complete { talk(a, text, null) }
            return
        }
        // A chat, a space, New app and an app's page take the words themselves (to the same agent).
        if (top !== home && top.onEvent(ev)) return
        // Over an open app you stay in it: the agent is told which app, and answers on a card (ADR-0025).
        val app = (top as? AppScreen)?.appId
        if (app != null) talkOver(a, text, app) else talk(a, text, null)
    }

    /**
     * Sends `text` to `a`'s current chat (or a new one) without leaving what's on screen: one of your
     * apps (`app`), or another Android app (null: r1d looks up which).
     */
    private fun talkOver(a: Agent, text: String, app: String?) {
        if (a.id.isEmpty()) return
        overAgent = a
        overChat = null
        reply.working(a)
        val failed = { msg: String -> reply.reply(a, msg, error = true) }
        val send = { id: String ->
            overChat = id
            val body = JSONObject().put("text", text)
            if (app != null) body.put("app", app) else body.put("android", true)
            R1d.post("/api/chats/$id/messages", body, onError = failed)
        }
        currentChat(a.id) { id ->
            if (id != null) {
                send(id)
            } else {
                R1d.post("/api/chats", JSONObject().put("agent", a.id), onError = failed) { r ->
                    val made = r.optJSONObject("chat")?.optString("id").orEmpty()
                    if (made.isEmpty()) failed("Couldn't start a chat") else send(made)
                }
            }
        }
    }

    /** A touch on the agent's answer: the whole conversation. */
    private fun openReplyChat() {
        val a = overAgent ?: return
        val chat = overChat ?: return
        reply.hide()
        open(Target.of("chat", "agent" to a.id, "id" to chat))
        // Over another app: r1os comes forward, in that chat.
        if (!visible) comeBack()
    }

    // ---- alarms and reminders (ADR-0028) ----------------------------------------------------------

    /** Alarm `id` rings, over whatever is in front. */
    private fun ring(id: String, label: String, agentId: String?) {
        if (!visible && displayOn()) dock.lift()
        alarmView.ring(id, label, agent(agentId) ?: focusAgent())
    }

    /**
     * One of r1os's own screens (crate::screens, R7), opened for you: past the glance screen (or the dial with the PIN on),
     * and over another app.
     */
    private fun openScreen(t: Target) {
        if (step != "done" || agents.isEmpty()) return
        val go = { open(t) }
        when {
            pinLocked && glance.up -> glance.openDial(go)
            glance.up -> glance.complete(go)
            else -> go()
        }
        if (!visible) comeBack()
    }

    /**
     * The browser while an agent works in it: small, as a live preview in that agent's chat (T-090; from home, that chat
     * opens), and full screen when it asks you to take over (past the glance screen and the PIN). The full screen goes
     * when the agent closes it.
     */
    /** An agent's request the process took (T-142, Requests): the app window or the browser, shown as it works. */
    fun request(ev: Ev) {
        when (ev.type) {
            "app_window" -> showAppWindow(ev)
            "browser" -> showBrowser(ev)
        }
    }

    /** The app window as the agent works (ADR-0061): its preview in that chat, or the whole window when you're asked. */
    private fun showAppWindow(ev: Ev) {
        val op = ev.str("op")
        val top = top()
        if (op == "close" || op == "stopped" || top is AppWindowScreen) return
        if (op != "ask_user") {
            val chat = top as? ChatScreen
            val here = displayOn() && visible && !glance.up && step == "done"
            if (chat != null && chat.chatId == ev.str("chat") && visible) chat.appWindowPreview()
            else if (top === home && here && op == "open") {
                openScreen(Target.of("chat", "agent" to ev.str("agent").orEmpty(), "id" to ev.str("chat").orEmpty()))
            }
            return
        }
        val go = { push(AppWindowScreen(this)) }
        if (glance.up) glance.complete(go) else go()
        if (!visible) comeBack()
    }

    private fun showBrowser(ev: Ev) {
        val op = ev.str("op")
        val top = top()
        if (op == "close" || op == "stopped") return
        if (top is BrowserScreen) return
        val ask = op == "ask_user"
        // In that agent's chat, a little live preview of the page in the corner, not the whole screen (T-090); a touch
        // there opens the full browser. From home, that chat, with its preview. Asked to take over, the full browser.
        if (!ask) {
            val chat = top as? ChatScreen
            val here = displayOn() && visible && !glance.up && step == "done"
            if (chat != null && chat.chatId == ev.str("chat") && visible) chat.browserPreview()
            else if (top === home && here && op == "open") {
                openScreen(Target.of("chat", "agent" to ev.str("agent").orEmpty(), "id" to ev.str("chat").orEmpty()))
            }
            return
        }
        val go = { push(BrowserScreen(this)) }
        when {
            glance.up -> glance.complete(go)
            else -> go()
        }
        if (!visible) comeBack()
    }

    /** The reply bubble over the browser touched: back to its chat (usually right underneath), else that chat opens. */
    fun toChat(agentId: String?, chat: String) {
        val under = stack.getOrNull(stack.size - 2) as? ChatScreen
        back()
        if (under?.chatId != chat && agentId != null) open(Target.of("chat", "agent" to agentId, "id" to chat))
    }

    /** A routine's note touched: its chat, past the glance screen. */
    private fun openFromRoutine(agentId: String, chat: String) {
        notice.hide()
        openScreen(Target.of("chat", "agent" to agentId, "id" to chat))
    }

    /** Timer `id` rang (ADR-0050): the alarm screen, over whatever is in front. */
    private fun ringTimer(id: String, label: String, agentId: String?) {
        if (!visible && displayOn()) dock.lift()
        alarmView.ring(id, label, agent(agentId) ?: focusAgent(), timer = true)
    }

    /** A pomodoro moved on (ADR-0050): one chime, and what's next at the top ("break · 5 min"). */
    private fun showPhase(ev: Ev) {
        if (step != "done" || alarmView.ringing != null) return
        if (!visible && displayOn()) dock.lift()
        Sounds.chime(context)
        val len = lengthText(ev.num("length").toLong())
        val what = when (ev.str("phase")) {
            "focus" -> "focus · $len"
            "long" -> "long break · $len"
            else -> "break · $len"
        }
        val round = ev.int("round", 1)
        val rounds = ev.int("rounds", 1)
        val detail = if (ev.str("phase") == "focus") "round $round of $rounds" else "round $round of $rounds done"
        placeNotice()
        notice.showTimer(what, detail, agent(ev.str("agent")) ?: focusAgent())
    }

    /**
     * The soonest timer, counting down on the glance screen; a timer still ringing (r1d or the shell restarted while
     * it rang).
     */
    private fun loadTimers() = R1d.get("/api/timers") { r ->
        val a = r.optJSONArray("timers") ?: return@get
        val list = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        val now = r.optLong("now", System.currentTimeMillis())
        glance.setTimer(list.firstOrNull { !it.has("ringing") }, now - System.currentTimeMillis())
        list.firstOrNull { it.has("ringing") }?.let { t ->
            val id = t.optString("id")
            if (now - t.optLong("ringing") > RING_FOR_MS) R1d.post("/api/timers/${R1d.enc(id)}/stop")
            else if (alarmView.ringing == null) ringTimer(id, t.optString("title"), t.optString("agent"))
        }
    }

    /** Agent `agentId`'s reminder `id`, over whatever is in front. */
    private fun showReminder(id: String, text: String, agentId: String?) {
        if (id.isEmpty()) return
        if (!visible && displayOn()) dock.lift()
        placeNotice()
        notice.showReminder(id, text, agent(agentId) ?: focusAgent())
    }

    private fun seenReminder(id: String) = R1d.post("/api/alarms/${R1d.enc(id)}/seen") { loadWaiting() }

    /** An app's reminder seen (T-049): it won't come back on wake. */
    private fun seenNotice(id: Long) = R1d.post("/api/notices/$id/seen") { loadWaiting() }

    // ---- what's waiting (T-123) ---------------------------------------------------------------------

    /**
     * The reminders you haven't seen, newest first: an agent's (r1d's fired ones, ADR-0028) and apps' (T-049). Banners
     * go by themselves after a few seconds (T-123); these stay on the glance screen, and in Notifications, until seen.
     */
    var waiting: List<GlanceView.Waiting> = emptyList()
        private set
    private val waitingListeners = ArrayList<() -> Unit>()

    /** Called whenever [waiting] changes; the returned function stops it. */
    fun onWaiting(f: () -> Unit): () -> Unit {
        waitingListeners += f
        return { waitingListeners -= f }
    }

    /** Looks again: `alarms` when r1d's list just came (else it's fetched too). */
    private fun loadWaiting(alarms: List<JSONObject>? = null) {
        val withNotices = { reminders: List<GlanceView.Waiting> ->
            R1d.get("/api/notices") { r ->
                val a = r.optJSONArray("notices") ?: org.json.JSONArray()
                val notices = (0 until a.length()).mapNotNull { a.optJSONObject(it) }.map { n ->
                    GlanceView.Waiting(
                        kind = "notice", id = n.optLong("id").toString(), agent = null, app = n.optString("app"),
                        title = n.optString("title").ifEmpty { n.optString("name") }, text = n.optString("body"),
                        icon = n.optString("icon").ifEmpty { "bell" }, color = Theme.COLORS[n.optString("accent")] ?: Theme.accent,
                        at = n.optLong("at"),
                    )
                }
                waiting = (reminders + notices).sortedByDescending { it.at }
                showWaiting()
                waitingListeners.toList().forEach { it() }
            }
        }
        val fromAlarms = { list: List<JSONObject> ->
            list.filter { it.optString("kind") == "reminder" && it.has("fired") }.map { rem ->
                val a = agent(rem.optString("agent"))
                GlanceView.Waiting(
                    kind = "reminder", id = rem.optString("id"), agent = a?.id, app = null, title = a?.name ?: "reminder",
                    text = rem.optString("label"), icon = "bell-ringing", color = Theme.colorOf(a?.color), at = rem.optLong("fired"),
                )
            }
        }
        if (alarms != null) withNotices(fromAlarms(alarms)) else R1d.get("/api/alarms") { r ->
            val a = r.optJSONArray("alarms") ?: return@get
            withNotices(fromAlarms((0 until a.length()).mapNotNull { a.optJSONObject(it) }))
        }
    }

    /** One of what's waiting, opened from the glance screen (T-128): an agent's reminder its chat, an app's the app; seen. */
    fun openWaiting(w: GlanceView.Waiting) {
        // A request (T-160): its card, as the glance screen goes.
        if (w.kind == "approval") {
            glance.complete()
            return
        }
        if (w.kind != "reminder") {
            seenWaiting(w, open = true)
            return
        }
        seenWaiting(w, open = false)
        val a = w.agent ?: return
        currentChat(a) { id -> open(Target.of("chat", "agent" to a, "id" to id)) }
    }

    /** One of what's waiting, seen (Notifications): an app's opens too, as its banner would. */
    fun seenWaiting(w: GlanceView.Waiting, open: Boolean) {
        // A request swiped away is a no, as leaving its card is (ADR-0045), so the agent hears now, not in 2 minutes.
        if (w.kind == "approval") {
            approvalsWaiting.remove(w.id)
            showWaiting()
            R1d.post("/api/approvals/${w.id}", JSONObject().put("allow", false))
            return
        }
        if (w.kind == "reminder") {
            seenReminder(w.id)
            if (notice.reminder == w.id) notice.hide()
            return
        }
        w.id.toLongOrNull()?.let { seenNotice(it) }
        if (notice.appNotice?.toString() == w.id) notice.hide()
        if (open) w.app?.takeIf { it.isNotEmpty() }?.let { openFromNotice(it) }
    }

    /** The newest app reminder you haven't seen (T-049), shown again as the R1 wakes, as an agent's is. */
    private fun loadNotices() = R1d.get("/api/notices") { r ->
        if (step != "done" || notice.shown) return@get
        val n = r.optJSONArray("notices")?.optJSONObject(0) ?: return@get
        if (!visible && displayOn()) dock.lift()
        placeNotice()
        notice.show(Ev(JSONObject(n.toString()).put("type", "notify")))
    }

    /**
     * The next alarm (on the glance screen); an alarm still ringing (r1d or the shell restarted while it
     * rang); and, `again`, a reminder you haven't seen yet (each time the R1 wakes, until you have).
     */
    private fun loadAlarms(again: Boolean = true) = R1d.get("/api/alarms") { r ->
        val a = r.optJSONArray("alarms") ?: return@get
        val list = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        val now = r.optLong("now", System.currentTimeMillis())
        val next = list.filter { it.optString("kind") == "alarm" && it.optBoolean("on") && it.has("at") }.minOfOrNull { it.optLong("at") }
        glance.setNextAlarm(next?.let { whenText(context, it) })
        list.firstOrNull { it.has("ringing") }?.let { al ->
            val id = al.optString("id")
            if (now - al.optLong("ringing") > RING_FOR_MS) R1d.post("/api/alarms/${R1d.enc(id)}/stop")
            else if (alarmView.ringing == null) ring(id, al.optString("label"), al.optString("agent"))
        }
        // On the glance screen, what you haven't seen is its first page (T-130): no banner again over it.
        if (again && step == "done" && !notice.shown && !(glance.up && !Look.hides("notifications"))) {
            val rem = list.filter { it.optString("kind") == "reminder" && it.has("fired") }.maxByOrNull { it.optLong("fired") }
            // An agent's reminder first; else an app's (T-049).
            if (rem != null) showReminder(rem.optString("id"), rem.optString("label"), rem.optString("agent")) else loadNotices()
        }
        loadWaiting(list)
    }

    /** r1os to the front (Android won't let a background app bring itself forward, so r1d does it). */
    private fun comeBack() = R1d.post("/api/android/open", JSONObject().put("screen", "home"))

    /**
     * Where a reminder sits: over the glance screen, under the time and date (where they usually end, if
     * the screen hasn't been laid out since the app started); elsewhere at the top.
     */
    private fun placeNotice() {
        val under = glance.infoBottom().takeIf { it > 0 } ?: context.dpi(GLANCE_INFO_DP)
        (notice.layoutParams as LayoutParams).topMargin = if (glance.up && !dock.over) under + context.dpi(14f) else context.dpi(10f)
        notice.requestLayout()
    }

    // ---- System One (ADR-0032) ------------------------------------------------------------------------

    /** The camera opens behind the words: toward the world (`look`), or around to you (`selfie`). */
    private fun peekOpen(kind: String) {
        if (peeking == kind) return
        if (!visible && displayOn()) dock.lift()
        peeking = kind
        dev.r1os.host.camera.Motor.face(user = kind == "selfie")
        peek.visibility = View.VISIBLE
        peek.resume()
        peek.fit()
        voice.seeThrough(true)
    }

    private fun peekClose() {
        if (peeking == null || shooting) return
        peeking = null
        peek.pause()
        peek.visibility = View.GONE
        voice.seeThrough(false)
        dev.r1os.host.camera.Motor.putBack()
    }

    /** Does what System One heard. True when it's done (else the words go to the agent, as usual). */
    private fun act(intent: JSONObject, words: String, a: Agent, top: Screen): Boolean {
        val app = intent.optString("app").ifEmpty { null }
        when (intent.optString("kind")) {
            "look", "selfie" -> {
                // The photo as you let go, then your words with it to the agent.
                if (peeking == null) peekOpen(intent.optString("kind"))
                peek.flash()
                shooting = true
                dev.r1os.host.camera.Photos.take(context, "look", onDone = { id ->
                    shooting = false
                    peekClose()
                    val chat = top as? ChatScreen
                    if (chat != null) chat.sendPhoto(words, id) else talkAbout(id, words, a)
                }, onError = { msg -> shooting = false; peekClose(); voice.error(msg, 2600) })
            }
            "camera" -> { peekClose(); open(Target.of("camera")) }
            "photos" -> open(Target.of("tool", "id" to "photos"))
            "journal" -> {
                val t = intent.optString("text").ifEmpty { words }
                R1d.post("/api/journal", JSONObject().put("text", t), onError = { voice.error(it, 2600) }) { flash("in your journal") }
            }
            "record" -> R1d.post("/api/recorder/start", onError = { voice.error(it, 2600) }) { open(Target.of("tool", "id" to "recorder")) }
            "stop_record" -> R1d.post("/api/recorder/stop", onError = { voice.error(it, 2600) }) { flash("recording saved") }
            // r1os's own screens by name (R7): "open the translator for English to Portuguese", "open bluetooth".
            "screen" -> open(Target(intent.optJSONObject("args")?.optJSONObject("target") ?: return false))
            // "Set a timer for ten minutes", "start a pomodoro" (ADR-0050): started at once; the glance screen counts down.
            "timer", "pomodoro" -> {
                val body = JSONObject(intent.optJSONObject("args")?.toString() ?: "{}").put("agent", a.id)
                if (intent.optString("kind") == "pomodoro") body.put("pomodoro", true)
                intent.optString("text").takeIf { it.isNotBlank() }?.let { body.put("label", it) }
                R1d.post("/api/timers", body, onError = { voice.error(it, 2600) }) { r ->
                    val t = r.optJSONObject("timer") ?: return@post
                    val len = lengthText(t.optLong("length"))
                    flash(when {
                        t.optString("kind") == "pomodoro" -> "focus · $len"
                        t.optString("label").isNotBlank() -> "${t.optString("label")} · $len"
                        else -> "$len timer"
                    })
                }
            }
            "open" -> open(Target.of("app", "id" to (app ?: return false)))
            "android" -> openAndroid(app ?: return false)
            // "pause", "next song", "what's playing" (ADR-0040; R7): r1os's own player, else an Android app's; "play Daft
            // Punk on Spotify": a music app finds it and plays.
            "media" -> {
                val p = dev.r1os.host.Player
                val ms = dev.r1os.host.MediaSessions
                val op = intent.optString("action")
                when {
                    p.active && op in setOf("pause", "resume", "stop") -> when (op) {
                        "pause" -> { p.pause(); flash("paused") }
                        "resume" -> { p.resume(); flash("playing") }
                        else -> { p.stop(); flash("stopped") }
                    }
                    p.active && op == "now" -> reply.reply(a, listOf(p.now.title, p.now.song).filter { it.isNotBlank() }.joinToString(": ").ifEmpty { "Something's playing." })
                    ms.now != null -> when (op) {
                        "pause" -> { ms.pause(); flash("paused") }
                        "resume" -> { ms.play(); flash("playing") }
                        "stop" -> { ms.stop(); flash("stopped") }
                        "next" -> { ms.next(); flash("next") }
                        "previous" -> { ms.previous(); flash("back a song") }
                        "now" -> { if (!visible && displayOn()) dock.lift(); reply.reply(a, ms.describe() ?: return false) }
                        else -> return false
                    }
                    else -> return false
                }
            }
            "edit" -> {
                val id = app ?: return false
                R1d.post("/api/apps/${R1d.enc(id)}/edit", JSONObject().put("request", intent.optString("text").ifEmpty { words }),
                    onError = { voice.error(it, 2600) }) { r ->
                    open(Target.of("app-info", "id" to id, "build" to r.optLong("build").toString()))
                }
            }
            "run" -> {
                val id = app ?: return false
                val body = JSONObject().put("action", intent.optString("action")).put("params", intent.optJSONObject("args") ?: JSONObject())
                R1d.post("/api/apps/${R1d.enc(id)}/run", body, onError = { voice.error(it, 2600) }) { r ->
                    val said = r.optJSONArray("said")?.let { s -> (0 until s.length()).joinToString(" · ") { s.optString(it) } }.orEmpty()
                    if (!visible && displayOn()) dock.lift()
                    reply.reply(a, said.ifEmpty { "Done in ${r.optString("name")}." })
                }
            }
            else -> return false
        }
        return true
    }

    /** Sends `words` to the agent you last talked to, in its current chat, which opens. */
    fun talkText(words: String) = talk(focusAgent(), words, null)

    /** Asks the agent you last talked to about photo `id` (`words`), in its current chat, which opens. */
    fun talkAbout(id: String, words: String, to: Agent? = null) {
        val a = to ?: focusAgent()
        if (a.id.isEmpty()) return
        currentChat(a.id) { chat -> open(Target.of("chat", "agent" to a.id, "id" to chat, "say" to words, "images" to id)) }
    }

    /** Sends `text` to `a`'s current chat (or a new one), which opens. */
    private fun talk(a: Agent, text: String, app: String?) {
        if (a.id.isEmpty()) return
        currentChat(a.id) { id -> open(Target.of("chat", "agent" to a.id, "id" to id, "say" to text, "app" to app)) }
    }

    /** A touch on a reminder: its app, past the glance screen if that's up. */
    private fun openFromNotice(app: String) {
        notice.hide()
        val go = { open(Target.of("app", "id" to app)) }
        if (glance.up) glance.complete(go) else go()
        if (!visible) comeBack()
    }

    /** An app's `copy` step: the text on Android's clipboard, and a word that it's there. */
    private fun copy(text: String) {
        val cm = context.getSystemService(android.content.ClipboardManager::class.java)
        runCatching { cm.setPrimaryClip(android.content.ClipData.newPlainText("r1os", text)) }
            .onSuccess { flash("copied") }
            .onFailure { flash("couldn't copy") }
    }

    /** A word at the top for a moment ("copied"), in the toast's place. */
    fun flash(word: String) {
        Sounds.play(Sounds.S.POP)
        toastChat = null
        toastView.removeAllViews()
        toastText.text = word
        (toastText.parent as? android.view.ViewGroup)?.removeView(toastText)
        toastView.addView(toastText)
        toastView.background = roundRect(Theme.SURFACE, context.dp(18f))
        toastView.visibility = View.VISIBLE
        toastView.alpha = 0f
        toastView.translationY = -context.dp(10f)
        toastView.animate().alpha(1f).translationY(0f).setDuration(240).setInterpolator(Theme.POP).start()
        removeCallbacks(hideToast)
        postDelayed(hideToast, 1600)
    }

    /** "Nova replied": a reply landed in a chat that isn't on screen. Touch opens it. */
    /** `a` has something new in `chat`: its reply, or (`from`) another agent's message. */
    private fun toast(a: Agent, chat: String, from: Agent? = null) {
        Sounds.play(Sounds.S.NOTE)
        toastChat = a.id to chat
        val accent = Theme.colorOf(a.color)
        toastView.removeAllViews()
        toastView.addView(View(context).apply { background = roundRect(accent, context.dp(4f)) }, LinearLayout.LayoutParams(context.dpi(8f), context.dpi(8f)).apply { rightMargin = context.dpi(8f) })
        toastText.text = if (from != null) "${from.name} → ${a.name}" else "${a.name} replied"
        (toastText.parent as? android.view.ViewGroup)?.removeView(toastText)
        toastView.addView(toastText)
        toastView.background = roundRect(Theme.SURFACE, context.dp(18f), context.dpi(1.5f), Theme.withAlpha(accent, 0.6f))
        toastView.visibility = View.VISIBLE
        toastView.alpha = 0f
        toastView.translationY = -context.dp(10f)
        toastView.animate().alpha(1f).translationY(0f).setDuration(280).setInterpolator(Theme.POP).start()
        removeCallbacks(hideToast)
        postDelayed(hideToast, 3500)
    }

    // ---- lifecycle and device status ----------------------------------------------------------------

    private val batteryReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) = onBattery(i)
    }

    /** The network r1d last heard came up: the time zone is looked up once per network (ADR-0030). */
    private var reportedNetwork: Network? = null

    private val netCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
            val level = if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) levelOf(caps.signalStrength) else -1
            val online = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            post {
                setWifi(level)
                if (online && network != reportedNetwork) {
                    reportedNetwork = network
                    R1d.post("/api/time/auto")
                    // A new network: its DNS servers, for r1d's link (ADR-0048).
                    dev.r1os.host.DeviceSettings.reportSoon()
                }
            }
        }

        override fun onLost(network: Network) {
            post { setWifi(-1) }
        }
    }

    /** The activity started or stopped (the display went on or off). */
    fun setShown(on: Boolean) {
        if (on == visible) return
        visible = on
        val cm = context.getSystemService(ConnectivityManager::class.java)
        if (on) {
            visibleSince = SystemClock.uptimeMillis()
            if (dock.over) {
                dock.land()
                if (notice.shown) placeNotice()
            }
            if (wasConnected) {
                loadAlarms()
                loadTimers()
                loadApprovals()
            }
            context.registerReceiver(batteryReceiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))?.let { onBattery(it) }
            runCatching { cm.registerDefaultNetworkCallback(netCallback) }
            if (glance.up) glance.setDisplayOn(true)
            else if (home.view.visibility == View.VISIBLE || stack.isNotEmpty()) { top().onShow(); owed = null }
            updateTilt()
            // A double click that woke it came before this was on screen.
            if (wokeHome > 0 && SystemClock.uptimeMillis() - wokeHome < WOKE_HOME_MS) post { homeFromWake() }
            wokeHome = 0L
        } else {
            glance.setDisplayOn(false)
            // The accelerometer only with the screen on (T-103).
            updateTilt()
            volumeView.hideNow()
            // Stopped because the display went off: the glance screen goes up, even if the screen-off
            // broadcast came before this process was listening.
            if (!displayOn() && !glance.up) wentOff()
            top().onHide()
            voice.hide()
            runCatching { context.unregisterReceiver(batteryReceiver) }
            runCatching { cm.unregisterNetworkCallback(netCallback) }
        }
    }

    private fun onBattery(i: Intent) {
        val level = i.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = i.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
        val status = i.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
        battery = if (level >= 0 && scale > 0) level * 100 / scale else -1
        charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
        home.onDeviceStatus(battery, charging, wifiLevel)
        glance.onDeviceStatus(battery, charging, wifiLevel)
        updateDocked()
        passChargeEstimate()
    }

    /**
     * Android's estimate of the time to full charge, passed to r1d as the battery changes (T-152): agents asked how long
     * until it's charged answer with it (steadier than the battery's own). Only when it changes by a minute or more.
     */
    private fun passChargeEstimate() {
        val ms = if (charging) runCatching { context.getSystemService(BatteryManager::class.java).computeChargeTimeRemaining() }.getOrDefault(-1L) else -1L
        val minutes = if (ms > 0) ms / 60_000 else -1
        if (minutes == passedEstimate) return
        passedEstimate = minutes
        R1d.post("/api/battery", JSONObject().put("charge_ms", if (ms > 0) ms else JSONObject.NULL), onError = {}) {}
    }

    /** The estimate last passed (minutes; -1 none). */
    private var passedEstimate = -2L

    /**
     * Docked mode (Settings › Display & sound): on its side the glance screen turns, plugged in or not (T-117), and
     * while it charges it keeps the display on. On its side it's kept on unplugged too, above [REST_FLOOR]% (T-157).
     */
    private fun updateDocked() {
        updateRest()
        updateTilt()
    }

    /**
     * The glance screen as an always-on display (T-157; Max, Q67: "I like the glance screen being always on/semi-always
     * on somehow"): docked, it keeps the display on while it charges (T-103), and on its side unplugged too while the
     * battery's above [REST_FLOOR]%; nobody touching it, it steps down ([dev.r1os.host.ui.Dim]: still, then low).
     */
    private fun updateRest() {
        val on = dev.r1os.host.DeviceSettings.docked && glance.up && step == "done" && (charging || (dockedOn && battery > REST_FLOOR))
        glance.keepScreenOn = on
        dev.r1os.host.ui.Dim.rest(on)
    }

    /** The rest's low step is showing: the backlight down. */
    private var lowOn = false
    /** How low the backlight goes at rest (0..1, Android's float brightness; the harness may try another). */
    private var restBrightness = REST_BRIGHTNESS

    /**
     * The rest changed step, or Android's dim came (T-036, T-157). Low: the backlight down to [restBrightness] (never
     * up), the accelerometer off; anything brings both back. Docked on its side, Android's dim before its timeout puts
     * the glance screen up instead of the display going dark: there it stays on, and rests (not with a PIN: that locks
     * as the display goes off).
     */
    private fun onRest() {
        val d = dev.r1os.host.ui.Dim
        restLow(d.low)
        if (d.dimmed && !d.resting && dockedOn && !pinSet && !glance.up && visible && step == "done" &&
            (charging || battery > REST_FLOOR)) lock()
    }

    private fun restLow(on: Boolean) {
        if (on == lowOn) return
        lowOn = on
        val w = (context as? android.app.Activity)?.window ?: return
        val mine = runCatching {
            android.provider.Settings.System.getInt(context.contentResolver, android.provider.Settings.System.SCREEN_BRIGHTNESS) / 255f
        }.getOrDefault(1f)
        w.attributes = w.attributes.apply {
            screenBrightness = if (on) minOf(restBrightness, mine) else android.view.WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
        }
        updateTilt()
    }

    // ---- docked on its side (T-103) ------------------------------------------------------------------

    private var tilt: android.view.OrientationEventListener? = null
    /** Which side it's held on, as the screen shows: 1 its right side down, -1 its left, 0 upright. */
    private var sideways = 0
    /** Since when it's been the other way from how the screen shows (0: it isn't). */
    private var tiltSince = 0L
    /** Since when it's lain flat while docked on its side (0: it isn't), T-165. */
    private var flatSince = 0L
    /** What turns with it: the docked glance screen, or the chat on screen (T-089); null: nothing. */
    private var tiltFor: Any? = null
    /** Docked mode's tilt target (T-156): everything turns with the R1, not only the glance screen. */
    private val dockTarget = Any()
    /** On its side with docked mode on (T-156). */
    var dockedOn = false
        private set
    /** How the harness says the R1 is held (`/harness/tilt`); null: the accelerometer says. */
    private var forcedSide: Int? = null

    /** The chat on screen, which types on its side (T-089): not under the glance screen, not in onboarding. */
    private fun typingChat(): ChatScreen? = (top() as? ChatScreen)?.takeIf { step == "done" && !glance.up && visible }

    /**
     * The R1's tilt is watched (the accelerometer, only then) with the display on and something to turn: docked mode on
     * and the glance screen up (its right side down, T-103 and T-117), or a chat on screen (either side, T-089: the box
     * and r1os's keyboard). Anything else, or something else on screen, puts it back upright.
     */
    private fun updateTilt() {
        val target: Any? = when {
            // Docked mode on (T-156): r1os as ever, turned with it, home and chats too, not only the glance screen. It
            // stays turned with the display off, so it wakes as it slept (Max, 2026-10-13: "locking and unlocking resets
            // the orientation, with a delay as it shifts to glance mode when it turns back on").
            dev.r1os.host.DeviceSettings.docked && step == "done" -> dockTarget
            !displayOn() -> null
            else -> typingChat()
        }
        if (target !== tiltFor) {
            setSideways(0)
            tiltSince = 0L
            tiltFor = target
        }
        // The accelerometer only with the display on (T-103), resting too: stopped there, the R1 lifted off its side
        // without a touch never knew, and kept the screen on all night (T-165, 2026-10-14).
        val listen = target != null && displayOn()
        if (listen && tilt == null) {
            tilt = object : android.view.OrientationEventListener(context, android.hardware.SensorManager.SENSOR_DELAY_NORMAL) {
                override fun onOrientationChanged(deg: Int) = tilted(deg)
            }.also { if (it.canDetectOrientation()) it.enable() }
        }
        if (!listen) {
            tilt?.disable()
            tilt = null
            tiltSince = 0L
        }
    }

    /**
     * 90°: its left side up, so its right side down (`OrientationEventListener`'s); 270°: its left side down. Wider to
     * stay than to turn, and it turns once it's held the new way [TILT_SETTLE_MS]; flat (unknown) changes nothing. The
     * docked glance screen turns only on its right side.
     */
    private fun tilted(deg: Int) {
        if (forcedSide != null) return
        // Flat (no side up or down) a while, docked: it isn't on its side any more, so it's upright as far as docking
        // goes, and the always-on glance screen lets the display sleep unplugged (T-165: laid flat on a bed, it stayed on
        // all night). On its side again, it turns back.
        if (deg == android.view.OrientationEventListener.ORIENTATION_UNKNOWN) {
            tiltSince = 0L
            if (tiltFor !== dockTarget || sideways == 0) return
            val now = android.os.SystemClock.uptimeMillis()
            if (flatSince == 0L) flatSince = now else if (now - flatSince >= FLAT_MS) {
                flatSince = 0L
                setSideways(0)
            }
            return
        }
        flatSince = 0L
        fun near(c: Int, staying: Boolean) = Math.abs(deg - c) <= if (staying) 50 else 25
        val side = when {
            near(90, sideways == 1) -> 1
            tiltFor !== dockTarget && tiltFor !is GlanceView && near(270, sideways == -1) -> -1
            else -> 0
        }
        if (side == sideways) {
            tiltSince = 0L
            return
        }
        val now = android.os.SystemClock.uptimeMillis()
        if (tiltSince == 0L) {
            tiltSince = now
        } else if (now - tiltSince >= TILT_SETTLE_MS) {
            tiltSince = 0L
            setSideways(side)
        }
    }

    private fun setSideways(side: Int) {
        if (side == sideways) return
        sideways = side
        dev.r1os.host.ime.Rotation.hold(context, side)
        when (val t = tiltFor) {
            dockTarget -> dockTo(side != 0)
            is GlanceView -> glance.setDock(side != 0)
            is ChatScreen -> t.typeOn(side != 0)
        }
    }

    /**
     * Docked on its side, or upright again (T-156; Max, 2026-10-13: "what we had for normal mode, but in landscape, and
     * supporting touch controls"): the glance screen's landscape layout, and every screen told (a chat shows its box).
     */
    private fun dockTo(on: Boolean) {
        if (on == dockedOn) return
        dockedOn = on
        glance.setDock(on)
        home.onDock(on)
        stack.forEach { it.onDock(on) }
        // On its side, the glance screen may stay on unplugged (T-157).
        updateRest()
    }

    /** A talk started by touch (the mic on home, docked: T-156) is listening: the overlay sends it as well as cancels. */
    private var touchTalking = false

    /**
     * Talking by touch (T-156; Max, 2026-10-13: "On the home screen in dock mode, add a little button on the bottom right
     * corner with a mic to speak to an agent"): `start`, then the overlay's `send` or `cancel` (r1d's `/api/voice/talk`).
     */
    fun talkByTouch(op: String) {
        if (op == "start") touchTalking = true
        R1d.post("/api/voice/talk", JSONObject().put("op", op))
    }

    // ---- edge swipes (ADR-0059) ---------------------------------------------------------------------

    /**
     * What a touch from an edge is doing: [EDGE_DOWN] the glance screen pulled down from the top (T-124),
     * [EDGE_LAUNCHER] home's launcher up from the bottom (T-126), [EDGE_AWAY] a pulled-down glance screen pushed back up;
     * 0 nothing of the shell's (the screen's own touch).
     */
    private var edgeKind = 0
    private var edgeX = 0f
    private var edgeY = 0f
    /** Past the slop the touch is the swipe's: the screen under the finger got a cancel. */
    private var edgeOn = false
    private var edgeVelocity: android.view.VelocityTracker? = null
    /** Where every touch went down, for a tap on a page's header and a swipe on home (T-156). */
    private var downX = 0f
    private var downY = 0f
    /** A tap on a page's header a moment ago (its time and place): a second goes back (T-156). */
    private var headerTapAt = 0L
    private var headerTapX = 0f
    /** A vertical swipe on home, past the slop: it moves the wheel when it lets go (T-156). */
    private var homeSwipe = false
    /** This touch brought the glance screen back from its low step: it's swallowed (T-157). */
    private var wakeTouch = false
    /** The touch began with the launcher at its top: pulled down far enough, it closes (T-156). */
    private var launcherTop = false

    /** Ahead of every view in r1os (a WebView's own scrolling included): a swipe from an edge is the shell's. */
    override fun dispatchTouchEvent(e: MotionEvent): Boolean {
        // At rest, low (T-157): the touch that brings it back only does that, nothing under the finger.
        if (e.actionMasked == MotionEvent.ACTION_DOWN) wakeTouch = dev.r1os.host.ui.Dim.wasLow
        if (wakeTouch) {
            if (e.actionMasked == MotionEvent.ACTION_UP || e.actionMasked == MotionEvent.ACTION_CANCEL) wakeTouch = false
            return true
        }
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = e.x
                downY = e.y
                homeSwipe = false
                launcherTop = top() === home && home.launcherAtTop()
                edgeEnd()
                edgeKind = edgeAt(e)
                if (edgeKind != 0) {
                    edgeX = e.x
                    edgeY = e.y
                    edgeVelocity = android.view.VelocityTracker.obtain().also { it.addMovement(e) }
                }
            }
            MotionEvent.ACTION_MOVE -> if (edgeKind != 0) {
                edgeVelocity?.addMovement(e)
                val dx = e.x - edgeX
                val dy = e.y - edgeY
                if (!edgeOn) {
                    val slop = android.view.ViewConfiguration.get(context).scaledTouchSlop
                    val along = if (edgeKind == EDGE_DOWN) dy else -dy
                    if (along > slop && Math.abs(dy) > Math.abs(dx)) {
                        edgeOn = true
                        // The screen under the finger lets go of this touch.
                        val cancel = MotionEvent.obtain(e).apply { action = MotionEvent.ACTION_CANCEL }
                        super.dispatchTouchEvent(cancel)
                        cancel.recycle()
                    } else if (Math.abs(dx) > slop * 2 || along < -slop) {
                        edgeEnd() // not a swipe from the edge: the screen's own
                    }
                }
                if (edgeOn) {
                    edgeMove(dy)
                    return true
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> if (edgeKind != 0) {
                if (edgeOn) {
                    val v = edgeVelocity
                    v?.addMovement(e)
                    v?.computeCurrentVelocity(1000)
                    edgeRelease(e.y - edgeY, v?.yVelocity ?: 0f, e.actionMasked == MotionEvent.ACTION_CANCEL)
                    edgeEnd()
                    return true
                }
                edgeEnd()
            }
        }
        if (homeTouch(e)) return true
        if (e.actionMasked == MotionEvent.ACTION_UP) headerTap(e)
        val taken = super.dispatchTouchEvent(e)
        // Kept where nothing under it takes touches (home's clock): else Android sends none of the touch's moves.
        return taken || (e.actionMasked == MotionEvent.ACTION_DOWN && edgeKind != 0)
    }

    /** Nothing in the way of a swipe of the shell's: onboarding, an alarm ringing, talking, typing (the keyboard's own). */
    private fun canSwipe() = step == "done" && agents.isNotEmpty() && visible && alarmView.ringing == null &&
        voice.visibility != View.VISIBLE && dev.r1os.host.ime.R1Keyboard.current?.shown != true

    /**
     * A vertical swipe on home, not from an edge (T-156): past the slop it's the shell's (what's under the finger gets a
     * cancel), and letting go moves the wheel a step: up for the next agent, as turning it down does. A sideways stroke
     * stays the character's (petting it). True: taken.
     */
    private fun homeTouch(e: MotionEvent): Boolean {
        // The launcher at its top, pulled down: it closes, as a sheet does (its list scrolls the rest of the time).
        if (launcherTop && e.actionMasked == MotionEvent.ACTION_UP && home.launcherOn && !glance.up) {
            val dy = e.y - downY
            if (dy > context.dp(LAUNCHER_CLOSE_DP) && dy > Math.abs(e.x - downX) * 1.5f) {
                home.hideLauncher()
                return false
            }
        }
        if (edgeKind != 0 || glance.up || top() !== home || home.launcherOn || !canSwipe()) return false
        when (e.actionMasked) {
            MotionEvent.ACTION_MOVE -> if (!homeSwipe) {
                val dy = e.y - downY
                val slop = android.view.ViewConfiguration.get(context).scaledTouchSlop * 2
                if (Math.abs(dy) > slop && Math.abs(dy) > Math.abs(e.x - downX) * 1.5f) {
                    homeSwipe = true
                    val cancel = MotionEvent.obtain(e).apply { action = MotionEvent.ACTION_CANCEL }
                    super.dispatchTouchEvent(cancel)
                    cancel.recycle()
                }
            }
            MotionEvent.ACTION_UP -> if (homeSwipe) {
                homeSwipe = false
                if (Math.abs(e.y - downY) > context.dp(HOME_SWIPE_DP)) home.swipe(if (e.y < downY) 1 else -1)
                return true
            }
            MotionEvent.ACTION_CANCEL -> homeSwipe = false
        }
        return homeSwipe
    }

    /**
     * Two taps on a page's header go back (T-156; Max, 2026-10-13: "double tapping the header of pages to go back"),
     * on any page over home, upright or docked: the touch's way to what a double click does.
     */
    private fun headerTap(e: MotionEvent) {
        // A page with a header lights it at the first tap and goes back at the second, by itself (BackHeader).
        if (stack.isEmpty() || glance.up || !canSwipe() || top()?.backHeader != null) return
        val slop = android.view.ViewConfiguration.get(context).scaledTouchSlop
        if (e.y > context.dp(HEADER_DP) || Math.abs(e.x - downX) > slop || Math.abs(e.y - downY) > slop) return
        val near = Math.abs(e.x - headerTapX) < context.dp(HEADER_DP)
        if (e.eventTime - headerTapAt <= android.view.ViewConfiguration.getDoubleTapTimeout() && near) {
            headerTapAt = 0L
            post { back() }
        } else {
            headerTapAt = e.eventTime
            headerTapX = e.x
        }
    }

    /** What a touch starting here may become: the top edge anywhere in r1os; the bottom edge on home. */
    private fun edgeAt(e: MotionEvent): Int {
        if (!canSwipe()) return 0
        // The glance screen as the R1 wakes: a swipe up from its bottom edge unlocks it, as a double click does (T-156),
        // and nothing else (a pocket can't open it). Pulled down, a swipe up puts it back.
        if (glance.up) return when {
            glance.dropped && !glance.dialOpen -> EDGE_AWAY
            !glance.dropped && !glance.dialOpen && !glance.locking && e.y >= height - context.dp(UNLOCK_EDGE_DP) -> EDGE_UNLOCK
            else -> 0
        }
        if (e.y <= context.dp(EDGE_DP)) return EDGE_DOWN
        if (top() === home && home.canDragLauncher() && e.y >= height - context.dp(EDGE_DP)) return EDGE_LAUNCHER
        return 0
    }

    /** The glance screen starts down over this screen, with its agent (else the one you were last with). */
    private fun edgeBegin() {
        glance.dropBegin(agent(top().agentId) ?: glanceAgent())
        glance.onDeviceStatus(battery, charging, wifiLevel)
    }

    private fun edgeMove(dy: Float) {
        when (edgeKind) {
            EDGE_DOWN -> {
                if (!glance.dropped) edgeBegin()
                glance.dropTo(dy)
            }
            EDGE_LAUNCHER -> home.dragLauncher(-dy)
            EDGE_AWAY -> glance.dropTo(height + dy)
            // It rides up with the finger as a sheet (T-189); with the PIN on, only a little.
            EDGE_UNLOCK -> glance.liftTo(dy, guarded = pinLocked)
        }
    }

    /** Let go: far enough (or flung) and it goes; else it springs back. */
    private fun edgeRelease(dy: Float, vy: Float, cancelled: Boolean) {
        val fling = context.dp(FLING_DP)
        when (edgeKind) {
            EDGE_DOWN -> glance.dropSettle(!cancelled && (dy > height * 0.3f || vy > fling))
            EDGE_LAUNCHER -> home.releaseLauncher(!cancelled && (-dy > height * 0.3f || -vy > fling))
            EDGE_AWAY -> if (!cancelled && (-dy > height * 0.2f || -vy > fling)) glance.dropAway() else glance.dropSettle(true)
            // Far enough up, or flung: unlocked, as a double click does, the sheet carried on up (T-189); with the PIN
            // on, it settles and the dial opens. Else it drops back.
            EDGE_UNLOCK -> {
                val far = !cancelled && (-dy > height * 0.2f || -vy > fling)
                if (pinLocked) {
                    glance.liftSettle(false)
                    if (far) glance.openDial()
                } else {
                    glance.liftSettle(far)
                }
            }
        }
    }

    private fun edgeEnd() {
        edgeKind = 0
        edgeOn = false
        edgeVelocity?.recycle()
        edgeVelocity = null
    }

    /**
     * A triple press's shortcut (Settings › Controls), from anywhere: past the glance screen (or the dial, with the PIN
     * on), and over another app.
     */
    private fun shortcut(s: String) {
        if (step != "done" || agents.isEmpty()) return
        val target = when (s) {
            "recorder", "journal", "timer", "whats_up" -> Target.of("tool", "id" to s)
            "camera" -> Target.of("camera")
            else -> return
        }
        Sounds.play(Sounds.S.OPEN)
        val go = { open(target) }
        when {
            pinLocked && glance.up -> glance.openDial(go)
            glance.up -> glance.complete(go)
            else -> go()
        }
        if (!visible) comeBack()
    }

    private fun setWifi(level: Int) {
        wifiLevel = level
        home.onDeviceStatus(battery, charging, wifiLevel)
        glance.onDeviceStatus(battery, charging, wifiLevel)
    }

    fun destroy() {
        dev.r1os.host.Requests.detach(this)
        runCatching { context.unregisterReceiver(screenOff) }
        runCatching { context.unregisterReceiver(batteryReceiver) }
        runCatching { context.getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(netCallback) }
        offs.forEach { runCatching { it() } }
        offs.clear()
        dock.land()
        setShown(false)
        stack.forEach { it.onDestroy() }
        stack.clear()
    }

    private companion object {
        /** How long it's held on its side (or upright again) before the docked screen turns (T-103). */
        const val TILT_SETTLE_MS = 700L
        /** Lying flat this long, docked: off its side (T-165). */
        const val FLAT_MS = 60_000L
        /** At rest, low (T-157): the backlight (Android's float brightness, 0..1; 51/255 is 0.2). */
        const val REST_BRIGHTNESS = 0.02f
        /** Unplugged, the glance screen stays on (on its side) only above this battery level (%). */
        const val REST_FLOOR = 20
        /**
         * A swipe from the top (or, on home, the bottom) starts this close to the edge (ADR-0059). Android's own swipe
         * there (showing its hidden, empty bars) is off: its `system_gestures_start_threshold` is 0 on r1os (T-127).
         */
        const val EDGE_DP = 40f
        /** A quick flick does it however far it went (dp a second). */
        const val FLING_DP = 700f
        const val EDGE_DOWN = 1
        const val EDGE_LAUNCHER = 2
        const val EDGE_AWAY = 3
        /** A swipe up from the glance screen's bottom edge: unlock (T-156). */
        const val EDGE_UNLOCK = 4
        /** How near the bottom a swipe up on the glance screen starts, to unlock. */
        const val UNLOCK_EDGE_DP = 72f
        /** A page's header, for two taps to go back (T-156). */
        const val HEADER_DP = 64f
        /** How far a swipe on home goes to move the wheel a step. */
        const val HOME_SWIPE_DP = 36f
        /** How far the launcher, at its top, is pulled down to close. */
        const val LAUNCHER_CLOSE_DP = 72f
        /** The cast the boot animation r1d has was drawn from (T-029). */
        const val BOOTANIM_SIG = "bootanim_sig"
        const val WAKE_GUARD_MS = 700L
        /** A double click that woke the R1 still goes home if the shell shows within this. */
        const val WOKE_HOME_MS = 2000L
        /** An alarm still ringing after this long (the shell was gone meanwhile) has given up. */
        const val RING_FOR_MS = 10 * 60_000L
        /** Where the glance screen's time, date and status usually end. */
        const val GLANCE_INFO_DP = 150f
        /** Screens deeper than this go back home first. */
        const val MAX_DEPTH = 6

        /** RSSI (dBm) to 0–4 bars, like Android's own steps. */
        fun levelOf(rssi: Int): Int = when {
            rssi == NetworkCapabilities.SIGNAL_STRENGTH_UNSPECIFIED -> 3
            rssi >= -55 -> 4
            rssi >= -66 -> 3
            rssi >= -77 -> 2
            rssi >= -88 -> 1
            else -> 0
        }
    }
}
