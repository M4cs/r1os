package dev.r1os.host

import android.annotation.SuppressLint
import android.app.ActivityOptions
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.graphics.Point
import android.graphics.Rect
import android.hardware.display.DisplayManager
import android.hardware.HardwareBuffer
import android.hardware.display.VirtualDisplay
import android.hardware.input.InputManager
import android.media.Image
import android.media.ImageReader
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.provider.Settings
import android.util.Base64
import android.view.Display
import android.view.InputDevice
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * The app window (ADR-0061, T-113): an Android app you installed, on a display of the host's own, which an agent drives
 * through r1d (`app_window {id, op, agent, chat, args}`; crate::appwin), answering each request at
 * `POST /api/apps/window/result`. Its ops: open, read (the screen as lines with refs like `[a12]`), describe,
 * screenshot, click, type, key, scroll, back, wait, ask_user (you take over: "sign in to Uber", then done) and close.
 *
 * - **The display:** the R1's own size and density, trusted, with its own focus (the wheel and the button stay
 *   r1os's) and its own power group, no lock screen on it, drawn into an [ImageReader] whose latest frame is the
 *   agent's screenshot and the picture you watch ([dev.r1os.host.shell.AppWindowScreen], and the chat's preview).
 * - **Tapping and typing:** real touches and keys injected into that display (the platform signature's
 *   `INJECT_EVENTS`); typing sets the field's text through the reader.
 * - **Reading:** [AppReader], an accessibility service on only while a window's open.
 * - **Yours alone:** password fields refuse the agent's typing, and a screen Android marks secure comes out black: its
 *   words are withheld too, and the agent asks you to take over.
 * - **Lives only while used:** made on first use; gone two minutes after the last request, unless you're watching
 *   or it's waiting for you.
 */
@SuppressLint("StaticFieldLeak")
object AgentApps {
    private val main = Handler(Looper.getMainLooper())
    private lateinit var app: Context
    @Volatile private var display: VirtualDisplay? = null
    private var reader: ImageReader? = null
    private var frameThread: HandlerThread? = null
    private val lock = Any()
    /** The latest frame, held until a newer one comes (an ImageReader keeps only a few). */
    private var image: Image? = null
    private var lastFrameAt = 0L
    private var width = 0
    private var height = 0
    /** Whose it is now: the agent and chat of the last request. */
    @Volatile var agent: String? = null
        private set
    @Volatile var chat: String? = null
        private set
    /** The app on it: its package and name. */
    @Volatile var pkg: String? = null
        private set
    @Volatile var name: String? = null
        private set
    /** You're asked to do something (ask_user): its words. */
    @Volatile var asking: String? = null
        private set
    private var askId: String? = null
    private var lastUse = 0L
    /** Refs from the last read: where each element is on the display, and its node. */
    private val refs = HashMap<String, Pair<Rect, AccessibilityNodeInfo>>()
    /** The last screen read, as its lines: an act that leaves it so waits a little longer for the app ([afterAct]). */
    private var lastScreen: String? = null

    /**
     * The last read's elements (place and words) and the picture they went with: a tap by place is checked against them
     * while the picture hasn't changed since, instead of walking the screen again, 2 to 6.6 s on Uber Eats' item sheets
     * (T-147). [stale]: the read's words lagged the picture; [cut]: it stopped at [MAX_LINES], so what's at the bottom
     * (an add-to-cart bar) may be missing.
     */
    private class Snapshot(val mark: Motion.Mark?, val els: List<Pair<Rect, String>>, val cut: Boolean, var stale: Boolean = false)
    private var snapshot: Snapshot? = null

    /** This op is a step of an app_do before its last (T-147): it answers without reading the screen. */
    private var quick = false
    /** What each ref was called when read: a node an app has since rebuilt (Compose, a web page) is found again by it. */
    private val names = HashMap<String, String>()
    private val listeners = ArrayList<() -> Unit>()
    private val frameListeners = ArrayList<(Bitmap) -> Unit>()
    /** A request waiting for the reader to come on. */
    private val waitingForReader = ArrayList<() -> Unit>()
    /** The screen's showing it: it stays while you look. */
    @Volatile var watching = false

    val open: Boolean get() = display != null
    val displayId: Int get() = display?.display?.displayId ?: Display.INVALID_DISPLAY
    /** The window's size: the one it has, or the one it will have ([WINDOW_W] × [WINDOW_H], or the harness's). */
    val size: Point get() = if (width > 0) Point(width, height) else Point(sizeOverride?.x?.takeIf { it > 0 } ?: WINDOW_W, sizeOverride?.y?.takeIf { it > 0 } ?: WINDOW_H)

    /** The shell starts: a reader left on by an earlier run (a crash) goes off. */
    fun init(context: Context) {
        app = context.applicationContext
        if (display == null) readerOff()
    }

    /** What the screens show changed (an app opened, an ask came or went, it closed). */
    fun onChange(f: () -> Unit): () -> Unit {
        listeners.add(f)
        return { listeners.remove(f) }
    }

    private fun changed() = main.post { listeners.toList().forEach { it() } }

    /** Each new frame, while someone's watching (the full window or the chat's preview); off again when they stop. */
    fun onFrame(f: (Bitmap) -> Unit): () -> Unit {
        main.post { synchronized(lock) { frame() }?.let(f) }
        synchronized(frameListeners) { frameListeners.add(f) }
        return { synchronized(frameListeners) { frameListeners.remove(f) } }
    }

    // ---- requests from r1d ------------------------------------------------------------------------------

    /**
     * The app window's own thread (T-143). Reading an app's screen takes seconds on Uber Eats (2.4–3.3 s a read), and
     * on the UI thread it froze the R1's own screen for each one: frames at 2.75 s at the 90th percentile while an agent
     * worked. Everything the window does runs here; the pointer and the screens' updates go to the UI thread.
     */
    private val ops: Handler by lazy {
        // At background priority: the UI thread always gets the CPU first, and the window's work fills in around it.
        Handler(android.os.HandlerThread("r1os-app-ops", android.os.Process.THREAD_PRIORITY_BACKGROUND).also { it.start() }.looper)
    }

    /** `app_window {id, op, agent, chat, args}`: does it on the window's own thread, and answers r1d. */
    fun handle(context: Context, ev: Ev) {
        app = context.applicationContext
        // Whose it is, at once: the chat's preview asks as it shows.
        ev.str("agent")?.takeIf { it.isNotEmpty() }?.let { agent = it }
        ev.str("chat")?.takeIf { it.isNotEmpty() }?.let { chat = it }
        ops.post { handleNow(ev) }
    }

    private fun handleNow(ev: Ev) {
        val id = ev.str("id") ?: return
        val op = ev.str("op").orEmpty()
        if (op == "stopped") {
            if (askId != null && ev.str("chat") == chat) answerAskNow(false)
            return
        }
        val args = ev.json.optJSONObject("args") ?: JSONObject()
        quick = args.optBoolean("quick")
        ev.str("agent")?.takeIf { it.isNotEmpty() }?.let { agent = it }
        ev.str("chat")?.takeIf { it.isNotEmpty() }?.let { chat = it }
        lastUse = SystemClock.uptimeMillis()
        ops.removeCallbacks(idle)
        ops.postDelayed(idle, IDLE_MS)
        if (op == "close") {
            // The user said "let me use it": the app goes to the R1's own screen as it is.
            if (args.optBoolean("give") && display != null) {
                val app = name ?: "the app"
                if (handOverNow()) reply(id, "Moved $app onto the R1's own screen for the user, as it was; the app window's closed.")
                else { close(); reply(id, "Couldn't move $app to the R1's own screen, so the app window's just closed: the user can open it from the launcher.") }
                return
            }
            close()
            reply(id, "Closed the app window.")
            return
        }
        if (op == "resolve") {
            resolve(id, args.optString("link"))
            return
        }
        // For the harness (T-154): the window's size from the next open (0 or none: the R1's own), closing one that's open.
        if (op == "size") {
            val w = args.optInt("width")
            val h = args.optInt("height")
            sizeOverride = if (w > 0 || h > 0) Point(w, h) else null
            if (display != null) close()
            reply(id, JSONObject().put("text", "The next app window: ${sizeOverride?.let { "${it.x}x${it.y} (0: the R1's)" } ?: "the R1's own size"}."))
            return
        }
        if (op == "open") {
            // An app that says it's offline though the R1 isn't (T-145): closed first, so it starts afresh.
            if (args.optBoolean("restart")) closeApp(args.optString("package"))
            openApp(id, args.optString("package"), args.optString("name"), args.optString("link"))
            return
        }
        if (display == null) {
            fail(id, took?.let { "The user took $it to use it themselves (it's on the R1's own screen now): leave it to them unless they ask you to open it again." }
                ?: "No app is open: app_open first.")
            return
        }
        withReader {
            // A permission prompt is yours (T-139): the agent may look at it, and hands it to you.
            if (op in ANSWERS && prompting()) { fail(id, PROMPT_SAYS); return@withReader }
            when (op) {
                // `plain`: the lines alone, for r1d to find an element named in words (System One).
                "read" -> reply(id, if (args.optBoolean("plain")) readScreen() else step(readScreen()))
                "describe" -> findSoon(args) { hit -> reply(id, describe(args, hit)) }
                // For whoever's working on this (the harness, not agents): every node, its class, place and state.
                "tree" -> reply(id, JSONObject().put("text", tree(args.optInt("depth", 30), args.optInt("max", 400))))
                // For the harness too (T-146): each web page on the screen and how much of its tree Chrome has handed over,
                // asking nothing more; and one way of asking for a blank page's tree (`how`), or reads' own nudge off or on.
                "webs" -> reply(id, JSONObject().put("webs", webs()))
                "nudge" -> reply(id, JSONObject().put("text", nudgeBy(args.optString("how"))).put("webs", webs()))
                "screenshot" -> ops.postDelayed({ screenshot(id) }, 150)
                "click" -> click(id, args)
                "type" -> type(id, args)
                "key" -> { val was = lastScreen; pulse(); key(keyCode(args.optString("key"))); afterAct("Pressed ${args.optString("key")}.", was) { reply(id, step(it)) } }
                "scroll" -> scroll(id, args)
                "back" -> { val was = lastScreen; key(KeyEvent.KEYCODE_BACK); afterAct("Went back.", was) { reply(id, step(it)) } }
                "wait" -> settle((args.optDouble("seconds", 3.0) * 1000).toLong().coerceIn(500, 15_000), strict = true) { reply(id, step(readScreen())) }
                "ask_user" -> {
                    asking = args.optString("why").ifEmpty { "Take over ${name ?: "the app"}" }
                    askId = id
                    changed()
                }
                else -> fail(id, "Unknown app window op `$op`.")
            }
        }
    }

    /** You're done with what the agent asked (or not): its tool goes on. */
    fun answerAsk(done: Boolean) {
        ops.post { answerAskNow(done) }
    }

    private fun answerAskNow(done: Boolean) {
        val id = askId ?: return
        askId = null
        asking = null
        changed()
        if (done) reply(id, JSONObject().put("app", name).put("text", "The user says they're done. Read the screen again to see where things are."))
        else fail(id, "The user didn't want to do that now.")
    }

    // ---- the display ------------------------------------------------------------------------------------

    /** Which installed app opens a link (an app's own, or a web address it claims): r1d asks before an `app_open`. */
    private fun resolve(id: String, link: String) {
        val pm = app.packageManager
        val info = runCatching { pm.resolveActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link)), PackageManager.MATCH_DEFAULT_ONLY) }.getOrNull()
        val pkg = info?.activityInfo?.packageName
        // "android" is the chooser: several apps open it, so the agent names the one.
        when {
            pkg == null -> fail(id, "No app on the R1 opens that link.")
            pkg == "android" -> fail(id, "Several apps open that link: give the package too (list_android_apps).")
            else -> reply(id, JSONObject().put("package", pkg).put("name", info.loadLabel(pm).toString()))
        }
    }

    /**
     * Opens [pkg] in the window. Already open on the R1's own screen (Max, 2026-10-12: "bring it into the agent window
     * if its already open and running in the background"), its screens move here as they are (the same size and
     * density, so it doesn't restart); a [link] then opens on top of them. Only if that move fails is it closed there
     * and opened again here ([again]), since Android would bring it back on the R1's own screen instead.
     */
    private fun openApp(id: String, pkg: String, label: String, link: String, again: Boolean = false) {
        // On a link's page, or at the app's start.
        val launch = if (link.isNotEmpty()) Intent(Intent.ACTION_VIEW, Uri.parse(link)).setPackage(pkg)
        else app.packageManager.getLaunchIntentForPackage(pkg)
        if (launch == null) { fail(id, "`$pkg` can't be opened (it has no launcher screen)."); return }
        if (!ensureDisplay()) { fail(id, "The app window couldn't be made on this R1."); return }
        readerOn()
        this.pkg = pkg
        this.name = label.ifEmpty { pkg }
        took = null
        ops.removeCallbacks(free)
        freeing = null
        val moved = !again && bringHere(pkg)
        movedIn = moved || again
        if (!moved || link.isNotEmpty()) {
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            val opts = ActivityOptions.makeBasic().setLaunchDisplayId(displayId)
            runCatching { app.startActivity(launch, opts.toBundle()) }.onFailure { fail(id, "Couldn't open ${this.name}: ${it.message}"); return }
        }
        changed()
        val said = when {
            moved -> "${this.name} was open on the R1's own screen: it's here now, where it was."
            again -> "${this.name} was open on the R1's own screen: closed it there and opened it here."
            else -> "Opened ${this.name}."
        }
        withReader {
            // Where Android put it is settled as the launch returns; a moment more in case it isn't.
            until(ARRIVE_MS, { onDisplay(pkg) }) { here ->
                if (display == null) { fail(id, "The app window closed."); return@until }
                if (!here) {
                    if (!again && closeApp(pkg)) openApp(id, pkg, label, link, again = true)
                    else fail(id, "${this.name} stayed on the R1's own screen, not the app window. Ask the user to close it first.")
                    return@until
                }
                // Then its screen settles, and the reader sees something on it (a new display's windows take a moment to
                // reach it, and an app moved here remakes its screen); still loading, it waits for that too.
                settle(OPEN_SETTLE_MS) {
                    until(READABLE_MS, { readable(pkg) && content() }) {
                        val o = readScreen(said)
                        if (o.optBoolean("loading")) afterAct(said) { reply(id, step(it)) } else reply(id, step(o))
                    }
                }
            }
        }
    }

    /** Closes an app (as Android's app info does), so it can open again in the window. */
    private fun closeApp(pkg: String): Boolean = runCatching {
        val am = app.getSystemService(android.app.ActivityManager::class.java)
        android.app.ActivityManager::class.java.getMethod("forceStopPackage", String::class.java).invoke(am, pkg)
    }.isSuccess

    // ---- moving an app's screens between the window and the R1's own (T-139) ----------------------------------

    /** Android's task manager (`IActivityTaskManager`, hidden: the platform signature lets the host use it). */
    private val tasks: Any? by lazy { runCatching { Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null) }.getOrNull() }

    /** The screens (root tasks) on a display, top first; null if Android won't say (`MANAGE_ACTIVITY_TASKS`). */
    private fun tasksOn(display: Int): List<android.app.TaskInfo>? = runCatching {
        val atm = tasks ?: return null
        @Suppress("UNCHECKED_CAST")
        atm.javaClass.getMethod("getAllRootTaskInfosOnDisplay", Int::class.javaPrimitiveType).invoke(atm, display) as List<android.app.TaskInfo>
    }.getOrNull()

    /** Whose a task is: its first screen's app, or any of its child tasks' (`RootTaskInfo.childTaskNames`, "pkg/cls"). */
    private fun android.app.TaskInfo.of(pkg: String): Boolean {
        if ((baseActivity ?: baseIntent?.component)?.packageName == pkg) return true
        @Suppress("UNCHECKED_CAST")
        val names = runCatching { javaClass.getField("childTaskNames").get(this) as Array<String?> }.getOrNull()
        return names?.any { it?.substringBefore('/') == pkg } == true
    }

    /** Moves a task (with all its screens, as they are) onto a display, on top there (`INTERNAL_SYSTEM_WINDOW`). */
    private fun moveTask(taskId: Int, display: Int): Boolean = runCatching {
        val atm = tasks ?: return false
        atm.javaClass.getMethod("moveRootTaskToDisplay", Int::class.javaPrimitiveType, Int::class.javaPrimitiveType).invoke(atm, taskId, display)
    }.isSuccess

    /** [pkg]'s screens on the R1's own display, moved into the window, bottom first so they stay in order. */
    private fun bringHere(pkg: String): Boolean {
        val theirs = tasksOn(Display.DEFAULT_DISPLAY)?.filter { it.of(pkg) }.orEmpty()
        if (theirs.isEmpty()) return false
        return theirs.asReversed().map { moveTask(it.taskId, displayId) }.all { it }
    }

    /** Whom you took the app from, and which app: an agent's next step is told ("let me use it"). */
    private var took: String? = null

    /** The app in the window was yours, on the R1's own screen, before the agent took it into the window. */
    private var movedIn = false

    /** The app to close once the window's been shut for [FREE_MS]. */
    private var freeing: String? = null

    /**
     * An app opened only for the agent, a minute after the window closed: closed as Android's app info does, so its
     * memory comes back. Uber Eats and its web pages kept 2 GB, and the R1 swapped and stuttered under your hands
     * (T-148). Not while anything's playing, not once it's on the R1's own screen, and not if the window opened again.
     */
    private val free = Runnable {
        val p = freeing ?: return@Runnable
        freeing = null
        if (display != null) return@Runnable
        if (app.getSystemService(android.media.AudioManager::class.java)?.isMusicActive == true) return@Runnable
        if (tasksOn(Display.DEFAULT_DISPLAY)?.any { it.of(p) } == true) return@Runnable
        closeApp(p)
    }

    /**
     * You take the app for yourself (Q63: "let me use it", ADR-0061 decision 7): its screens move onto the R1's own,
     * as they are, and the window closes. From the full window's button, or the agent's `app_close` with `for_user`.
     */
    fun handOver() {
        ops.post { handOverNow() }
    }

    private fun handOverNow(): Boolean {
        val id = displayId
        val app = name ?: "the app"
        val theirs = if (display == null) null else tasksOn(id)
        val moved = !theirs.isNullOrEmpty() && theirs.asReversed().map { moveTask(it.taskId, Display.DEFAULT_DISPLAY) }.all { it }
        if (!moved) return false
        // An ask waiting for you is answered: you've taken it.
        askId?.let { ask ->
            askId = null
            asking = null
            reply(ask, JSONObject().put("app", name).put("text", "The user took $app to use it themselves: it's on the R1's own screen now, and the app window's closed."))
        }
        took = app
        close()
        return true
    }

    /** A size the harness asked for (T-154), instead of the R1's own. */
    private var sizeOverride: Point? = null

    private fun ensureDisplay(): Boolean {
        if (display != null) return true
        val dm = app.getSystemService(DisplayManager::class.java)
        // Taller and a little wider than the R1's 480×640, at its density: a large phone (533×1067 dp), so more of an
        // app is on its screen for the agent (ADR-0063, T-154). Under 600 dp wide, where apps turn to tablet layouts.
        width = sizeOverride?.x?.takeIf { it > 0 } ?: WINDOW_W
        height = sizeOverride?.y?.takeIf { it > 0 } ?: WINDOW_H
        val dpi = app.resources.displayMetrics.densityDpi
        // The frames come in at background priority too (below the UI thread's).
        val t = HandlerThread("r1os-app-window", android.os.Process.THREAD_PRIORITY_BACKGROUND).also { it.start() }
        val handler = Handler(t.looper)
        // Buffers the GPU can draw from as they are, so the screens show a frame with no copy and no upload (T-141), and
        // the CPU can still read (screenshots, settling). Plain ones if the R1's graphics won't make those.
        val r = runCatching {
            ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, IMAGES, HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_CPU_READ_OFTEN)
        }.onSuccess { hardware = true }.getOrElse {
            hardware = false
            ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, IMAGES)
        }
        r.setOnImageAvailableListener({ rd ->
            synchronized(lock) {
                // Closed meanwhile (closing the reader closes its images: taking one now would crash the host).
                if (reader !== rd) return@setOnImageAvailableListener
                val img = runCatching { rd.acquireLatestImage() }.getOrNull() ?: return@setOnImageAvailableListener
                // The one the screens show stays open until they show another ([wrapFrame]).
                image?.takeIf { it !== shownImage }?.close()
                image = img
                lastFrameAt = SystemClock.uptimeMillis()
                motion?.add(img, lastFrameAt)
                coarse?.add(img, lastFrameAt)
            }
            showLatest()
        }, handler)
        // At [WINDOW_FPS], not the panel's 60: the agent works from stills and the preview needs no more, and Uber Eats'
        // pages drawing at 60 took most of the R1's CPU from your own screen (T-148).
        val vd = runCatching {
            if (android.os.Build.VERSION.SDK_INT >= 34) {
                dm.createVirtualDisplay(
                    android.hardware.display.VirtualDisplayConfig.Builder("r1os-app-window", width, height, dpi)
                        .setSurface(r.surface).setFlags(FLAGS).setRequestedRefreshRate(WINDOW_FPS).build(),
                )
            } else {
                dm.createVirtualDisplay("r1os-app-window", width, height, dpi, r.surface, FLAGS)
            }
        }.getOrNull()
        if (vd == null) {
            r.close()
            t.quitSafely()
            return false
        }
        display = vd
        synchronized(lock) {
            reader = r
            // Cells of 10 and 20 pixels, whatever the size: what counts as a change stays the same size on the screen.
            motion = Motion(width, height, width / 10, height / 10, detailed = true)
            coarse = Motion(width, height, width / 20, height / 20)
        }
        frameThread = t
        frames = handler
        showPending = false
        dm.registerDisplayListener(screen, main)
        stayAwake()
        return true
    }

    // ---- awake while you can see it -----------------------------------------------------------------------

    /** The window's display held on (its own power group). */
    private var awake: PowerManager.WakeLock? = null

    /**
     * The window's display stays on while the R1's own does (T-113). It has a power group of its own, which sleeps after
     * the screen timeout with nothing tapping into it (the agent thinking, a card waiting, you watching): the picture
     * went black, the app paused, and the agent's next taps were dropped. With the R1's screen off it's let go, so the
     * R1 can sleep (working with the screen off is T-114's).
     */
    private fun stayAwake() {
        val dm = app.getSystemService(DisplayManager::class.java)
        val on = display != null && dm.getDisplay(Display.DEFAULT_DISPLAY)?.state == Display.STATE_ON
        val held = awake?.isHeld == true
        if (on && held) return
        if (!on) {
            if (held) awake?.release()
            awake = null
            // And the window's display sleeps now, not after its own timeout. While any display is awake, Android counts
            // the R1 as on and ignores the key that wakes it: an R1 locked with an app window open couldn't be woken
            // (2026-10-13, T-149). An agent's work in the app waits for the screen meanwhile (T-114).
            if (display != null) sleepWindow()
            return
        }
        // A wake lock tied to the window's display alone (hidden, Android 14+), waking it if it dozed meanwhile.
        val pm = app.getSystemService(PowerManager::class.java)
        @Suppress("DEPRECATION")
        val wl = runCatching {
            PowerManager::class.java.getMethod("newWakeLock", Int::class.javaPrimitiveType, String::class.java, Int::class.javaPrimitiveType)
                .invoke(pm, PowerManager.SCREEN_BRIGHT_WAKE_LOCK or PowerManager.ACQUIRE_CAUSES_WAKEUP, "r1os:app-window", displayId) as PowerManager.WakeLock
        }.getOrNull() ?: return
        wl.setReferenceCounted(false)
        wl.acquire()
        awake = wl
    }

    /** The window's display, and only it, asleep (hidden `PowerManager.goToSleep(displayId, …)`, `DEVICE_POWER`). */
    private fun sleepWindow() {
        val id = displayId
        if (id == Display.INVALID_DISPLAY) return
        runCatching {
            PowerManager::class.java.getMethod("goToSleep", Int::class.javaPrimitiveType, Long::class.javaPrimitiveType, Int::class.javaPrimitiveType, Int::class.javaPrimitiveType)
                .invoke(app.getSystemService(PowerManager::class.java), id, SystemClock.uptimeMillis(), 0, 0)
        }.onFailure { android.util.Log.w("r1os", "app window: couldn't put its display to sleep", it) }
    }

    /** The R1's own screen going on or off. */
    private val screen = object : DisplayManager.DisplayListener {
        override fun onDisplayChanged(id: Int) { if (id == Display.DEFAULT_DISPLAY) ops.post { stayAwake() } }
        override fun onDisplayAdded(id: Int) {}
        override fun onDisplayRemoved(id: Int) {}
    }

    // ---- frames for whoever's watching ------------------------------------------------------------------

    /** The frame thread's handler. */
    private var frames: Handler? = null
    /** Frames copied for whoever's watching: a few bitmaps reused in turn (each frame is 1.2 MB). */
    private val shown = arrayOfNulls<Bitmap>(3)
    private var turn = 0
    private var lastShown = 0L
    private var showPending = false

    /**
     * The latest frame to whoever's watching (frame thread), at most [WATCH_FPS] a second for the full window and
     * [PREVIEW_FPS] for the chat's small preview: apps can draw 60 a second, and copying and redrawing each one kept
     * the host at half a core (T-113). A frame held back goes when its turn comes, so the last one always shows.
     */
    private fun showLatest() {
        if (showPending || synchronized(frameListeners) { frameListeners.isEmpty() }) return
        val wait = lastShown + 1000L / (if (watching) WATCH_FPS else PREVIEW_FPS) - SystemClock.uptimeMillis()
        if (wait > 0) {
            showPending = true
            frames?.postDelayed(showNow, wait)
            return
        }
        showNow.run()
    }

    private val showNow = Runnable {
        showPending = false
        lastShown = SystemClock.uptimeMillis()
        val bmp = synchronized(lock) { wrapFrame() ?: copyFrame() } ?: return@Runnable
        main.post { synchronized(frameListeners) { frameListeners.toList() }.forEach { it(bmp) } }
    }

    /** The reader's buffers can be drawn from as they are ([wrapFrame]). */
    private var hardware = false
    /** The image the screens show now: held open while they may draw it, so the display can't draw into it. */
    private var shownImage: Image? = null

    /**
     * The latest frame as a bitmap over its own buffer (hold [lock]; T-141): no copy and no texture upload, which were
     * most of the third of a core the chat's preview cost (T-113). The image stays open while it's shown; the one shown
     * before is let go a moment later, once the screens have drawn the new one.
     */
    private fun wrapFrame(): Bitmap? {
        if (!hardware) return null
        val img = image ?: return null
        val b = runCatching { img.hardwareBuffer?.use { Bitmap.wrapHardwareBuffer(it, SRGB) } }.getOrNull() ?: return null
        val was = shownImage
        shownImage = img
        if (was != null && was !== img) {
            frames?.postDelayed({ synchronized(lock) { if (was !== image && was !== shownImage) was.close() } }, LET_GO_MS)
        }
        return b
    }

    /** The latest frame, copied into the next of [shown] (hold [lock]). */
    private fun copyFrame(): Bitmap? {
        val img = image ?: return null
        val plane = runCatching { img.planes[0] }.getOrNull() ?: return null
        if (plane.rowStride != img.width * plane.pixelStride) return frame()
        turn = (turn + 1) % shown.size
        val b = shown[turn]?.takeIf { it.width == img.width && it.height == img.height }
            ?: Bitmap.createBitmap(img.width, img.height, Bitmap.Config.ARGB_8888).also { shown[turn] = it }
        plane.buffer.rewind()
        b.copyPixelsFromBuffer(plane.buffer)
        return b
    }

    // ---- settling -----------------------------------------------------------------------------------------

    /**
     * Where the screen moves (hold [lock]): [motion] fine enough that a 20-pixel radio dot filling in lands on a sample,
     * for whether an act changed anything; [coarse] for settling, where the fine grid saw every caret blink and shimmer
     * as new motion and kept acts waiting the whole time (2026-10-13).
     */
    private var motion: Motion? = null
    private var coarse: Motion? = null

    /**
     * Where the screen moves, from a grid of samples of each frame: which cells changed in each quarter second, over
     * the last second. It's settled when what moved in the last half second moved in the half second before too (a
     * video, a progress bar, a spinner: in place) and covers less than half the screen; a transition or new content
     * moves somewhere new. Apps that never stop animating (Spotify's previews) cost every step the whole wait before.
     */
    private class Motion(private val w: Int, private val h: Int, private val COLS: Int, private val ROWS: Int, private val detailed: Boolean = false) {
        private val CELLS = COLS * ROWS
        private val last = IntArray(CELLS)
        private val buckets = Array(BUCKETS) { java.util.BitSet(CELLS) }
        private val stamps = LongArray(BUCKETS) { -1 }

        // [detailed]: each cell's pixels hashed (every other one, each way), not one sample: a counter's digit going from
        // 2 to 1 missed every sample (Uber Eats' syrup pumps, 2026-10-13), the act said nothing changed, and the agent
        // tapped − again. Worked out as frames come at most every [BUCKET_MS] (for what was moving anyway: a caret
        // blinking), and from the latest frame when asked (what an act changed).
        private var latest: Image? = null
        private var details: IntArray? = null
        private val detailBuckets = Array(BUCKETS) { java.util.BitSet(CELLS) }
        private val detailStamps = LongArray(BUCKETS) { -1 }
        private var detailAt = -1L

        // A row of the frame, reused (all of Motion runs under the window's lock), and where the samples are in one. Each
        // row read is copied in one go: read a pixel at a time, sampling was most of the host's CPU while an app drew
        // (half the samples, 2026-10-13, T-141).
        private val row = IntArray(w)
        private val xs = IntArray(COLS) { (it * 2 + 1) * w / (COLS * 2) }
        private val colOf = IntArray(w) { it * COLS / w }

        /** [img]'s pixels as ints from its first row, and a row's length in ints; null if the frame's gone. */
        private fun ints(img: Image?): Pair<java.nio.IntBuffer, Int>? {
            val plane = runCatching { img?.planes?.get(0) }.getOrNull() ?: return null
            if (plane.pixelStride != 4 || plane.rowStride % 4 != 0) return null
            val b = runCatching { plane.buffer.duplicate() }.getOrNull() ?: return null
            b.position(0)
            return b.asIntBuffer() to plane.rowStride / 4
        }

        fun add(img: Image, now: Long) {
            latest = img
            val (ints, stride) = ints(img) ?: return
            val q = now / BUCKET_MS
            val b = bucket(buckets, stamps, q)
            runCatching {
                for (r in 0 until ROWS) {
                    ints.position((r * 2 + 1) * h / (ROWS * 2) * stride)
                    ints.get(row, 0, w)
                    for (c in 0 until COLS) {
                        val i = r * COLS + c
                        val p = row[xs[c]]
                        if (p != last[i]) {
                            last[i] = p
                            buckets[b].set(i)
                        }
                    }
                }
            }
            if (detailed && now - detailAt >= BUCKET_MS) {
                detailAt = now
                val d = detail(img) ?: return
                val was = details
                details = d
                if (was == null) return
                val db = bucket(detailBuckets, detailStamps, q)
                for (i in 0 until CELLS) if (d[i] != was[i]) detailBuckets[db].set(i)
            }
        }

        /** The bucket for quarter-second [q], emptied if it held an older one. */
        private fun bucket(of: Array<java.util.BitSet>, at: LongArray, q: Long): Int {
            val b = (q % BUCKETS).toInt()
            if (at[b] != q) {
                of[b].clear()
                at[b] = q
            }
            return b
        }

        /** Each cell of [img] hashed (every other pixel, each way); null if the frame's gone. */
        private fun detail(img: Image?): IntArray? {
            val (ints, stride) = ints(img) ?: return null
            val out = IntArray(CELLS) { 17 }
            return runCatching {
                var y = 0
                while (y < h) {
                    ints.position(y * stride)
                    ints.get(row, 0, w)
                    val base = y * ROWS / h * COLS
                    var x = 0
                    while (x < w) {
                        val i = base + colOf[x]
                        out[i] = out[i] * 31 + row[x]
                        x += 2
                    }
                    y += 2
                }
                out
            }.getOrNull()
        }

        /** The picture as it is now, and what was already moving: to tell later how much an act changed it. */
        class Mark(val cells: IntArray, val moving: java.util.BitSet, val detail: IntArray?)

        fun mark(now: Long): Mark {
            val q = now / BUCKET_MS
            val moving = java.util.BitSet(CELLS)
            for (k in 0 until BUCKETS) if (stamps[k] >= 0 && q - stamps[k] in 0 until BUCKETS) moving.or(buckets[k])
            for (k in 0 until BUCKETS) if (detailStamps[k] >= 0 && q - detailStamps[k] in 0 until BUCKETS) moving.or(detailBuckets[k])
            return Mark(last.copyOf(), moving, if (detailed) detail(latest) else null)
        }

        /** How many cells changed since [m] (a sample, or with [detailed], any pixel), leaving out what was moving then. */
        fun changed(m: Mark): Int {
            val d = if (m.detail != null) detail(latest) else null
            var n = 0
            for (i in 0 until CELLS) if (!m.moving[i] && (last[i] != m.cells[i] || (d != null && d[i] != m.detail!![i]))) n++
            return n
        }

        /** How much of the picture changed since [m], leaving out what was moving then (0 to 1). */
        fun moved(m: Mark): Float {
            var n = 0
            var of = 0
            for (i in 0 until CELLS) {
                if (m.moving[i]) continue
                of++
                if (last[i] != m.cells[i]) n++
            }
            return if (of == 0) 0f else n / of.toFloat()
        }

        fun still(now: Long): Boolean {
            val q = now / BUCKET_MS
            val recent = java.util.BitSet(CELLS)
            val before = java.util.BitSet(CELLS)
            for (k in 0 until BUCKETS) {
                val age = q - stamps[k]
                if (stamps[k] < 0 || age < 0 || age >= BUCKETS) continue
                if (age < BUCKETS / 2) recent.or(buckets[k]) else before.or(buckets[k])
            }
            if (recent.cardinality() * 2 >= CELLS) return false
            recent.andNot(before)
            return recent.isEmpty
        }

        companion object {
            const val BUCKETS = 4
            const val BUCKET_MS = 250L
        }
    }

    fun close() {
        ops.removeCallbacks(idle)
        askId?.let { answerAskNow(false) }
        if (display != null) runCatching { app.getSystemService(DisplayManager::class.java).unregisterDisplayListener(screen) }
        awake?.let { if (it.isHeld) it.release() }
        awake = null
        // Releasing the display ends the app's screens on it (they don't move onto the R1's own).
        val was = pkg.takeIf { display != null }
        display?.release()
        display = null
        if (was != null && took == null && !movedIn) {
            freeing = was
            ops.removeCallbacks(free)
            ops.postDelayed(free, FREE_MS)
        }
        // Under the frame thread's lock: a frame it's taking mustn't have its reader closed under it.
        synchronized(lock) {
            image?.close()
            image = null
            shownImage?.close()
            shownImage = null
            reader?.close()
            reader = null
            motion = null
            coarse = null
        }
        frameThread?.quitSafely()
        frameThread = null
        frames = null
        refs.clear()
        names.clear()
        pkg = null
        name = null
        readerOff()
        changed()
    }

    private val idle = Runnable { if (askId == null && SystemClock.uptimeMillis() - lastUse >= IDLE_MS && !watching) close() }

    /** The latest frame as a bitmap (hold [lock]). */
    private fun frame(): Bitmap? {
        val img = image ?: return null
        val plane = runCatching { img.planes[0] }.getOrNull() ?: return null
        val stride = plane.rowStride / plane.pixelStride
        val padded = Bitmap.createBitmap(stride, img.height, Bitmap.Config.ARGB_8888)
        plane.buffer.rewind()
        padded.copyPixelsFromBuffer(plane.buffer)
        if (stride == img.width) return padded
        return Bitmap.createBitmap(padded, 0, 0, img.width, img.height).also { padded.recycle() }
    }

    /** A secure screen (Android blacks those out on a display like this one): every sampled pixel black. */
    private fun secure(bmp: Bitmap?): Boolean {
        bmp ?: return false
        var y = 0
        while (y < bmp.height) {
            var x = 0
            while (x < bmp.width) {
                val p = bmp.getPixel(x, y)
                if ((p shr 16 and 0xff) > 4 || (p shr 8 and 0xff) > 4 || (p and 0xff) > 4) return false
                x += 12
            }
            y += 12
        }
        return true
    }

    /**
     * After an act: the screen settles, up to [max] ms, then [done]. Settled is only what was moving still moving, in
     * place ([Motion]); [strict] (`app_wait`): no new frame at all for a moment.
     */
    private fun settle(max: Long = SETTLE_MS, strict: Boolean = false, done: () -> Unit) {
        val start = SystemClock.uptimeMillis()
        val check = object : Runnable {
            override fun run() {
                val now = SystemClock.uptimeMillis()
                val quiet = synchronized(lock) { if (strict) now - lastFrameAt >= QUIET_MS else coarse?.still(now) != false }
                if ((quiet && now - start >= MIN_SETTLE_MS) || now - start >= max) done() else ops.postDelayed(this, 100)
            }
        }
        ops.postDelayed(check, MIN_SETTLE_MS)
    }

    // ---- the reader -------------------------------------------------------------------------------------

    private val me: String get() = ComponentName(app, AppReader::class.java).flattenToString()

    private fun readerOn() {
        val cr = app.contentResolver
        val cur = Settings.Secure.getString(cr, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES).orEmpty()
        val list = cur.split(':').filter { it.isNotEmpty() }
        if (me !in list) Settings.Secure.putString(cr, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES, (list + me).joinToString(":"))
        Settings.Secure.putInt(cr, Settings.Secure.ACCESSIBILITY_ENABLED, 1)
    }

    private fun readerOff() {
        if (!::app.isInitialized) return
        val cr = app.contentResolver
        val cur = Settings.Secure.getString(cr, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES).orEmpty()
        val list = cur.split(':').filter { it.isNotEmpty() }
        if (me !in list) return
        val rest = list - me
        Settings.Secure.putString(cr, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES, rest.joinToString(":"))
        if (rest.isEmpty()) Settings.Secure.putInt(cr, Settings.Secure.ACCESSIBILITY_ENABLED, 0)
    }

    /** Called by [AppReader] once it's on: what was waiting for it goes ahead. */
    fun readerConnected() = ops.post {
        val waiting = waitingForReader.toList()
        waitingForReader.clear()
        waiting.forEach { it() }
    }

    /** [f] once the reader's on (at once if it is; else when it connects, or after a few seconds without it). */
    private fun withReader(f: () -> Unit) {
        if (AppReader.instance != null) { f(); return }
        waitingForReader.add(f)
        ops.postDelayed({ if (waitingForReader.remove(f)) f() }, READER_WAIT_MS)
    }

    /** The windows on the app window's display, top first. */
    private fun roots(fresh: Boolean = true): List<AccessibilityNodeInfo> {
        val svc = AppReader.instance ?: return emptyList()
        // Its cache of the screen is dropped first ([fresh]): some apps don't say every change. And a whole fresh fetch
        // comes in big prefetched batches, where a cache half emptied by a web page's changes was filled again a node
        // at a time: reusing it made the check before each act slower, not faster (1 s to 4.4 s at the median,
        // 2026-10-13).
        if (fresh) svc.clearCache()
        fillsSeen.clear()
        kidsSeen.clear()
        cost.children = 0; cost.childNanos = 0; cost.nodes = 0; cost.trips = 0; cost.tripNanos = 0; cost.start = SystemClock.uptimeMillis()
        val r0 = System.nanoTime()
        val wins = runCatching { svc.windowsOnAllDisplays.get(displayId) }.getOrNull() ?: return emptyList()
        return wins.mapNotNull { w ->
            (if (android.os.Build.VERSION.SDK_INT >= 33) runCatching { w.getRoot(PREFETCH) }.getOrNull() else null) ?: runCatching { w.root }.getOrNull()
        }.also { cost.rootNanos = System.nanoTime() - r0 }
    }

    /**
     * A node's child, with the next 50 nodes under it in the same call to the app (Android 13+), there before the walk
     * asks for them: each call waits on the app's busy UI thread, and the default prefetch arrives too late to help
     * (a read of Spotify's 207 nodes took 0.8 s).
     */
    private fun child(n: AccessibilityNodeInfo, i: Int): AccessibilityNodeInfo? {
        if (kidsAt != inputs) { kidsSeen.clear(); kidsAt = inputs }
        val kids = kidsSeen.getOrPut(n) { arrayOfNulls<Any>(n.childCount).also { it.fill(UNFETCHED) } }
        if (i >= kids.size) return fetchChild(n, i)
        val k = kids[i]
        if (k !== UNFETCHED) return k as AccessibilityNodeInfo?
        return fetchChild(n, i).also { kids[i] = it }
    }

    /**
     * Children fetched since [roots] last fetched the screen, or the last input: a read's walks (what's shown, areas,
     * pages that fill the screen, labels) each asked for the same ones again, four times a node, and each answer from
     * Android's cache is a fresh copy: 85% of a read's time (2026-10-13).
     */
    private val kidsSeen = HashMap<AccessibilityNodeInfo, Array<Any?>>()
    private val UNFETCHED = Any()

    /** Inputs injected so far, and as of which [kidsSeen] holds. */
    @Volatile private var inputs = 0
    private var kidsAt = 0

    private fun fetchChild(n: AccessibilityNodeInfo, i: Int): AccessibilityNodeInfo? {
        val t = System.nanoTime()
        cost.children++
        try {
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                // Android's own check of a prefetched batch throws on a tree with a gap in it (Uber Eats, 2026-10-12: a
                // NullPointerException in checkFindAccessibilityNodeInfoResultIntegrity that killed the host): then plainly.
                runCatching { return n.getChild(i, PREFETCH) }
            }
            return runCatching { n.getChild(i) }.getOrNull()
        } finally {
            val d = System.nanoTime() - t
            cost.childNanos += d
            // A call that went to the app (from Android's cache, it's well under a millisecond).
            if (d > 2_000_000) { cost.trips++; cost.tripNanos += d }
        }
    }

    /** What reading the screen cost, since [roots] last fetched it: for the read's answer (`cost`) and tuning. */
    private class Cost { var children = 0; var childNanos = 0L; var nodes = 0; var start = 0L; var trips = 0; var tripNanos = 0L; var rootNanos = 0L }
    private val cost = Cost()

    /** Whether [pkg]'s screens are on the window's display: by Android's tasks, else by the reader's windows. */
    private fun onDisplay(pkg: String): Boolean = tasksOn(displayId)?.any { it.of(pkg) } ?: readable(pkg)

    /** Whether the app's screen has anything on it yet to read: words, or something to tap. */
    private fun content(): Boolean {
        var seen = 0
        fun any(n: AccessibilityNodeInfo, depth: Int): Boolean {
            if (seen++ > CONTENT_NODES || depth > 40) return false
            if (depth > 0 && n.isVisibleToUser && (n.isClickable || words(n, ids = false).isNotEmpty())) return true
            for (i in 0 until n.childCount) child(n, i)?.let { if (any(it, depth + 1)) return true }
            return false
        }
        return roots().any { any(it, 0) }
    }

    /** Whether the reader sees a window of [pkg]'s on the window's display. */
    private fun readable(pkg: String) = roots().any { it.packageName?.toString() == pkg }

    /** [test] every 100 ms until it's true or [max] ms pass, then [done] with the last answer. */
    private fun until(max: Long, test: () -> Boolean, done: (Boolean) -> Unit) {
        val start = SystemClock.uptimeMillis()
        val check = object : Runnable {
            override fun run() {
                val ok = display != null && test()
                if (ok || display == null || SystemClock.uptimeMillis() - start >= max) done(ok) else ops.postDelayed(this, 100)
            }
        }
        check.run()
    }

    /** Android asking for a permission on the window's display (Uber's location): yours to answer, never the agent's. */
    private fun prompting(): Boolean = roots().any { it.packageName?.toString() in PROMPTS }

    /** The screen as lines with refs (what you can tap or type into), withheld on a secure screen. */
    private fun readScreen(before: String? = null): JSONObject {
        refs.clear()
        names.clear()
        val out = JSONObject().put("app", name)
        if (secure(synchronized(lock) { frame() })) {
            snapshot = null
            return out.put("secure", true).put("text", listOfNotNull(before, "A secure screen (a card, a password or a code): Android hides it from you. Ask the user to take over (app_ask_user).").joinToString("\n"))
        }
        val lines = ArrayList<String>()
        var n = 0
        // `named`: inside something you can act on that took its words from its children (a chip whose label is a
        // child's text): those words aren't repeated as lines of their own.
        // Still loading: a spinner on screen, or words that say so.
        var loading = false
        // What's off the screen (a web page's elements with no place: Chipotle's item sheet had 400 of its 500 below the
        // fold, and each costs Chrome 4 ms to hand over on the R1) is read up to [OFF_NODES], nearest first; then a line
        // says there's more. Only inside something already off the screen: a link with no place of its own whose
        // children are on the screen is read whole.
        var offLeft = OFF_NODES
        var more = false
        fun walk(node: AccessibilityNodeInfo, depth: Int, named: Boolean, offParent: Boolean = false) {
            if (lines.size >= MAX_LINES || depth > 40) return
            val off = Rect().also { node.getBoundsInScreen(it) }.isEmpty
            if (off && offParent && offLeft-- <= 0) { more = true; return }
            cost.nodes++
            var inside = named
            if (node.isVisibleToUser) {
                // A web page with nothing in it yet is loading too (Uber Eats' home feed draws blank first).
                val cls = node.className?.toString().orEmpty()
                val blank = cls == "android.webkit.WebView" && node.childCount == 0
                // A spinner, not a progress bar: Spotify's player bar is a ProgressBar with a range, and counting it kept
                // every Spotify step waiting the whole loading time (2026-10-12).
                val spinner = cls.contains("ProgressBar") && node.rangeInfo == null
                if (!loading && (spinner || blank || words(node, ids = false).lowercase().trimEnd('.', '…') == "loading")) loading = true
                // Chrome builds a web page's tree only once something asks for it; a drawn page can stay blank to us for
                // half a minute (Uber Eats' store, 2026-10-12). Accessibility focus on it, as a screen reader gives,
                // asks; it draws nothing without touch exploration.
                if (blank) nudge(node)
                val own = words(node, ids = false)
                // A box named only by what's in it, with things to tap inside (Uber Eats' tab bar, "Home tab, 1 of 5 ·
                // Pickup tab…"): those are listed, so it isn't. A list that scrolls stays, by a short name, for app_scroll.
                val acting = node.isClickable || node.isEditable || node.isCheckable || node.isLongClickable || node.isScrollable
                // Only asked of what can be acted on: it walks what's inside, and most of a page has no words of its own.
                val boxed = acting && own.isEmpty() && !node.isEditable && !node.isCheckable && actsInside(node)
                val acts = acting && !(boxed && !node.isScrollable)
                if (acts) {
                    val name = if (boxed) label(node).take(LIST_NAME) else label(node)
                    val sb = StringBuilder(role(node))
                    if (name.isNotEmpty()) sb.append(" \"").append(name).append('"')
                    if (node.isCheckable) sb.append(if (node.isChecked) " (on)" else " (off)") else if (node.isChecked) sb.append(" (on)")
                    // The app's own word for its state ("selected", "checked", "expanded"): a web page's radios say it only so.
                    node.stateDescription?.toString()?.trim()?.takeIf { it.isNotEmpty() && it.length <= 40 }?.let { sb.append(" (").append(it).append(')') }
                    if (!node.isEnabled) sb.append(" (disabled)")
                    if (node.isSelected) sb.append(" (selected)")
                    val at = area(node)
                    // No place on the screen (a web page's tab scrolled out of its strip): a tap brings it into view first.
                    if (at.isEmpty) sb.append(" (off screen)")
                    val ref = "a${++n}"
                    refs[ref] = at to node
                    names[ref] = name
                    sb.append(" [").append(ref).append(']')
                    lines.add(sb.toString())
                    if (own.isEmpty() && name.isNotEmpty() && name != words(node)) inside = true
                } else if (own.isNotEmpty() && !named) {
                    lines.add("${role(node)} \"$own\"")
                }
            }
            for (c in shownChildren(node)) walk(c, depth + 1, inside, off)
        }
        val wins = roots()
        wins.forEach { walk(it, 0, false) }
        if (loading) out.put("loading", true)
        // Nothing at all to read is a screen between screens: still loading too.
        if (lines.isEmpty()) loading = true
        lastScreen = lines.joinToString("\n")
        snapshot = Snapshot(
            synchronized(lock) { motion?.mark(SystemClock.uptimeMillis()) },
            refs.entries.mapNotNull { (r, p) -> names[r]?.takeIf { it.isNotEmpty() && !p.first.isEmpty }?.let { Rect(p.first) to it } },
            cut = lines.size >= MAX_LINES,
        )
        if (loading) lines.add(0, "(Still loading: app_wait, then choose from it.)")
        // Android 15 cuts a background app off the network (and Doze does, unplugged): back in front, an app can go on
        // saying it's offline for a while, or for good (Spotify, T-145).
        if (lines.any { l -> OFFLINE.any { l.contains(it, ignoreCase = true) } } && online()) {
            lines.add("(The app says it's offline, but the R1 is online: it was cut off while in the background and usually reconnects within a minute. app_wait, then read again; still offline, app_open it with restart: true, which starts it afresh and loses its place.)")
        }
        if (more) lines.add("(More below, off the screen: scroll, or app_find what you're after.)")
        val text = lines.joinToString("\n").ifEmpty { "(nothing readable: try app_screenshot)" }
        val prompt = if (wins.any { it.packageName?.toString() in PROMPTS }) PROMPT_SAYS else null
        if (prompt != null) out.put("prompt", true)
        out.put("cost", JSONObject().put("nodes", cost.nodes).put("children", cost.children).put("fetch_ms", cost.childNanos / 1_000_000)
            .put("all_ms", SystemClock.uptimeMillis() - cost.start).put("lines", lines.size)
            .put("trips", cost.trips).put("trip_ms", cost.tripNanos / 1_000_000).put("root_ms", cost.rootNanos / 1_000_000))
        return out.put("text", listOfNotNull(before, prompt, text).joinToString("\n"))
    }

    /** A node's own words: its text, description or hint; its view id only when it's something you act on ([ids]). */
    private fun words(n: AccessibilityNodeInfo, ids: Boolean = true): String {
        val t = if (n.isPassword) "" else n.text?.toString().orEmpty()
        var w = t.ifBlank { n.contentDescription?.toString().orEmpty() }.ifBlank { n.hintText?.toString().orEmpty() }
        if (w.isBlank() && ids) w = n.viewIdResourceName?.substringAfterLast('/')?.replace('_', ' ').orEmpty()
        // Web pages' images can carry their whole address in their names (Uber Eats: "srcb64=aHR0cHM6…").
        return w.replace(SRC, "").replace(Regex("\\s+"), " ").trim().take(80)
    }

    /** What a node you can act on is called: its own words, else its children's, else its view id (as the lines show). */
    private fun label(n: AccessibilityNodeInfo): String = words(n, ids = false).ifEmpty { childWords(n) }.ifEmpty { words(n) }

    /** The words of what's inside a node (a button whose label is a child's text), up to a few. */
    private fun childWords(n: AccessibilityNodeInfo): String {
        val out = ArrayList<String>()
        fun walk(c: AccessibilityNodeInfo, depth: Int) {
            if (out.size >= 4 || depth > 6) return
            if (c !== n && c.isVisibleToUser) words(c, ids = false).takeIf { it.isNotEmpty() }?.let { out.add(it) }
            for (i in 0 until c.childCount) child(c, i)?.let { walk(it, depth + 1) }
        }
        walk(n, 0)
        return out.joinToString(" · ").take(80)
    }

    private fun role(n: AccessibilityNodeInfo): String {
        if (n.isPassword) return "password"
        val c = n.className?.toString()?.substringAfterLast('.').orEmpty()
        return when {
            n.isEditable || c.contains("EditText") || c.contains("TextField") -> "textbox"
            c.contains("CheckBox") -> "checkbox"
            c.contains("Switch") || c.contains("Toggle") -> "switch"
            c.contains("RadioButton") -> "radio"
            c.contains("Button") -> "button"
            c.contains("Image") -> if (n.isClickable) "button" else "img"
            c.contains("Recycler") || c.contains("ListView") || c.contains("ScrollView") || c.contains("Pager") -> "list"
            c.contains("Tab") -> "tab"
            n.isClickable -> "button"
            else -> "text"
        }
    }

    /** An element by its ref, or by its words (the best fit among those you can act on), or none. */
    private fun find(args: JSONObject): Pair<Rect, AccessibilityNodeInfo>? {
        args.optString("ref").takeIf { it.isNotEmpty() }?.let { r ->
            // r1d sends the name the agent saw for a ref (`named`): if the screen's been read since and the ref now means
            // something else (steps in one app_do, tools sent at once), the name decides. Refs are numbered per read.
            val hit = refs[r]
            if (hit != null && (!args.optBoolean("named") || names[r] == args.optString("target"))) return hit
            if (!args.optBoolean("named")) return null
        }
        val target = args.optString("target").trim().lowercase()
        if (target.isEmpty()) return null
        // The screen as it is now: refs from an older read may point where things were.
        readScreen()
        val scored = refs.values.map { (r, node) ->
            val w = label(node).lowercase()
            val s = when {
                w.isEmpty() -> 0
                w == target -> 4
                w.startsWith(target) -> 3
                w.contains(target) -> 2
                // Every word of it, not just one ("See more" isn't "More options").
                target.split(' ').filter { it.length > 2 }.let { ws -> ws.isNotEmpty() && ws.all { w.contains(it) } } -> 1
                else -> 0
            }
            Triple(s, r, node)
        }.filter { it.first > 0 }.sortedByDescending { it.first }
        return scored.firstOrNull()?.let { it.second to it.third }
    }

    /** The screen's nodes as indented lines (class, bounds, visible, children, words), all of them, hidden ones too. */
    private fun tree(maxDepth: Int, max: Int): String {
        val out = StringBuilder()
        var n = 0
        fun walk(node: AccessibilityNodeInfo, depth: Int) {
            if (depth > maxDepth || n++ >= max) return
            val r = Rect().also { node.getBoundsInScreen(it) }
            out.append("  ".repeat(depth)).append(node.className?.toString()?.substringAfterLast('.')).append(' ')
                .append(r.toShortString()).append(if (node.isVisibleToUser) "" else " hidden").append(" kids=").append(node.childCount)
                .append(" d=").append(node.drawingOrder)
                .append(' ').append(words(node).take(60).replace('\n', ' ')).append('\n')
            for (i in 0 until node.childCount) child(node, i)?.let { walk(it, depth + 1) }
        }
        roots().forEach { walk(it, 0) }
        return out.toString()
    }

    /** What an act would touch, for the card: its words, the app, a password field, and the screen's words. */
    private fun describe(args: JSONObject, found: Pair<Rect, AccessibilityNodeInfo>?): JSONObject {
        // The screen as it is now: its windows always (a permission prompt), and the place tapped.
        val wins = roots()
        val hit = found ?: point(args, wins)
        val node = hit?.second
        val ref = args.optString("ref")
        val out = JSONObject().put("name", names[ref]?.takeIf { found != null && refs[ref]?.second == node } ?: node?.let { label(it) }.orEmpty())
            .put("found", found != null).put("app", name).put("password", node?.isPassword == true)
            .put("prompt", wins.any { it.packageName?.toString() in PROMPTS })
        // Every word and button on it only when r1d needs them (`full`): a tap by place, or a final step's card. Walking
        // them before every tap cost as much as the read after it, 2 s on Chipotle's item sheet (2026-10-13).
        if (!args.optBoolean("full")) return out
        // A tap by place on the picture the last read went with: its elements decide what's by the finger (T-147).
        val px = if (found == null && args.has("x") && !args.isNull("x")) args.optDouble("x").toInt() else null
        val snap = snapshot
        if (px != null && snap != null && !snap.stale && !snap.cut &&
            // Nothing changed since, and little was moving then (a sheet still sliding in would have its places wrong).
            synchronized(lock) { snap.mark?.let { m -> motion?.changed(m)?.takeIf { m.moving.cardinality() * 20 < m.cells.size } } }
                ?.let { it < CHANGED_SOME } == true
        ) {
            val py = args.optDouble("y").toInt()
            val near = snap.els.filter { Rect(it.first).apply { inset(-NEAR_PX, -NEAR_PX) }.contains(px, py) }.map { it.second }.distinct()
            return out.put("buttons", org.json.JSONArray(snap.els.map { it.second }.distinct().take(BUTTONS)))
                .put("near", org.json.JSONArray(near)).put("fast", true)
        }
        val screen = StringBuilder()
        val tappable = LinkedHashSet<String>()
        // A tap by place: the buttons it could be on, with room for a web page's lag around each.
        val x = if (args.has("x") && !args.isNull("x")) args.optDouble("x").toInt() else null
        val y = args.optDouble("y").toInt()
        val near = LinkedHashSet<String>()
        // What's on the screen only: inside something off it (a web page's elements with no place), nothing can be under
        // the finger or seen on a card.
        fun walk(n: AccessibilityNodeInfo, depth: Int, offParent: Boolean = false) {
            if (depth > 40) return
            val off = Rect().also { n.getBoundsInScreen(it) }.isEmpty
            if (off && offParent) return
            if (n.isVisibleToUser) {
                if (screen.length <= SCREEN_CHARS && !n.isPassword) n.text?.toString()?.trim()?.takeIf { it.isNotEmpty() }?.let { screen.append(it).append(" · ") }
                if (n.isClickable) label(n).takeIf { it.isNotEmpty() }?.let { l ->
                    if (tappable.size < BUTTONS) tappable.add(l)
                    // (A web page's element off the screen has no place at all, [0,0][0,0]: not near the corner.)
                    if (x != null && Rect().also { n.getBoundsInScreen(it) }.takeUnless { it.isEmpty }?.apply { inset(-NEAR_PX, -NEAR_PX) }?.contains(x, y) == true) near.add(l)
                }
            }
            // The card shows what's on the screen, not a page hidden under it.
            for (c in shownChildren(n)) walk(c, depth + 1, off)
        }
        wins.forEach { walk(it, 0) }
        return out
            .put("screen", screen.toString().removeSuffix(" · ").take(SCREEN_CHARS))
            // Every button's words on the screen: a tap by place can't be told from the button that's there now, so r1d
            // asks before one on a screen with a final step (or, in a test, an add-to-cart) anywhere on it.
            .put("buttons", org.json.JSONArray(tappable.toList()))
            .put("near", org.json.JSONArray(near.toList()))
    }

    /** The element at x,y on the display (the deepest one there), for a tap by point. */
    private fun point(args: JSONObject, wins: List<AccessibilityNodeInfo> = roots()): Pair<Rect, AccessibilityNodeInfo>? {
        if (!args.has("x") || args.isNull("x")) return null
        val x = args.optDouble("x").toInt()
        val y = args.optDouble("y").toInt()
        var best: Pair<Rect, AccessibilityNodeInfo>? = null
        fun walk(n: AccessibilityNodeInfo, depth: Int) {
            if (depth > 40) return
            val r = Rect().also { n.getBoundsInScreen(it) }
            // A web page's link can have no area of its own while its children have it: look inside those too.
            if (!r.isEmpty && !r.contains(x, y)) return
            if (n.isVisibleToUser && !r.isEmpty) best = r to n
            for (c in shownChildren(n)) walk(c, depth + 1)
        }
        wins.forEach { walk(it, 0) }
        return best
    }

    // ---- acting -----------------------------------------------------------------------------------------

    /**
     * [find], waiting a little (up to [FIND_SOON_MS]) for what's named by its words when it isn't there yet: the step
     * before often opened the screen it's on, still drawing (2026-10-13: "Search Uber Eats" typed into right after a tap
     * on the Search tab found no field). Only [describe] waits, which r1d asks before every act: what it finds is what
     * the guards and the card see, and the act itself then finds it at once (or r1d stops it, if nothing was found).
     */
    private fun findSoon(args: JSONObject, done: (Pair<Rect, AccessibilityNodeInfo>?) -> Unit) {
        find(args)?.let { done(it); return }
        if (args.optString("target").isBlank()) { done(null); return }
        val start = SystemClock.uptimeMillis()
        fun again() {
            settle(FIND_SOON_STEP_MS) {
                val hit = if (display == null) null else find(args)
                if (hit != null || display == null || SystemClock.uptimeMillis() - start >= FIND_SOON_MS) done(hit) else again()
            }
        }
        again()
    }

    private fun click(id: String, args: JSONObject) {
        val hit = find(args)
        val byPoint = hit == null && args.has("x") && !args.isNull("x")
        if (hit == null && !byPoint) {
            fail(id, "No element ${args.optString("ref").ifEmpty { args.optString("target") }} on the screen: read it again.")
            return
        }
        val what = args.optString("ref").takeIf { it.isNotEmpty() }?.let { names[it] } ?: hit?.second?.let { label(it) }.orEmpty()
        val was = lastScreen
        val go = { x: Float, y: Float, press: AccessibilityNodeInfo? ->
            // The agent's pointer goes there first, if you're watching (T-137); the tap as it lands.
            aim(x, y) {
                if (display == null) { fail(id, "The app window closed."); return@aim }
                if (press != null) {
                    // An element of a web page: the page clicks that element itself, wherever it is now. A tap at where
                    // Chrome's lagging tree put a bread choice landed on Uber Eats' "Add to cart" (2026-10-13), so a
                    // web element is never tapped by place.
                    if (!press.performAction(AccessibilityNodeInfo.ACTION_CLICK)) {
                        fail(id, "“$what” didn't take the click (a web page inside the app): tap it at its place on the picture (x,y).")
                        return@aim
                    }
                    // Some of a page's elements answer only a real touch, and say they took the page's own click anyway
                    // (Uber Eats' search chips, 2026-10-13); others take it, but slowly. So the full wait, and no place
                    // to tap: told after 3 s where "Done" was, an agent tapped it as Chipotle's sheet closed, and hit
                    // "Add 1 to cart" in its place (a test refused it).
                    afterAct("Tapped $what.", was, nothingSays = webNothing(what)) { reply(id, step(it)) }
                    return@aim
                } else {
                    tap(x, y)
                }
                afterAct("Tapped ${what.ifEmpty { "at ${x.toInt()},${y.toInt()}" }}.", was) { reply(id, step(it)) }
            }
        }
        if (hit == null) { go(args.optDouble("x").toFloat(), args.optDouble("y").toFloat(), null); return }
        // Where it is now, not where the read saw it: the screen may have moved since (2026-10-12: a tap on Uber Eats'
        // "Big Mac® Meal", read while the menu loaded, landed on the menu's search).
        reach(hit.second, what) { n, at ->
            when {
                n != null && at != null -> go(at.x, at.y, if (isWeb(n)) n else null)
                // A web page's element out of view (Uber Eats' "Extra Value Meals", scrolled out of its tab strip): the
                // page's own click, which brings it in.
                n != null && isWeb(n) && n.performAction(AccessibilityNodeInfo.ACTION_CLICK) ->
                    afterAct("Tapped $what.", was, nothingSays = webNothing(what)) { reply(id, step(it)) }
                else -> fail(id, "“$what” isn't on the screen where it can be tapped any more (scrolled away, covered or gone): read it again, or scroll to it.")
            }
        }
    }

    /**
     * Where to tap [node] now: refreshed (or found again by its words, [what], if the app replaced it), brought into
     * view if it's mostly off the screen, and the middle of its visible part; null if it can't be.
     */
    private fun reach(node: AccessibilityNodeInfo, what: String, done: (AccessibilityNodeInfo?, android.graphics.PointF?) -> Unit) {
        val n = if (node.refresh()) node else if (what.isEmpty()) null else find(JSONObject().put("target", what))?.second
        if (n == null) { done(null, null); return }
        shown(n)?.let { done(n, it); return }
        // Mostly off the screen (below the fold, the end of a carousel): the app brings it into view, then where it is.
        n.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SHOW_ON_SCREEN.id)
        settle(SHOW_MS) { done(n, if (n.refresh()) shown(n) else null) }
    }

    /** The middle of [n]'s visible part, if enough of it is on the screen to tap. */
    private fun shown(n: AccessibilityNodeInfo): android.graphics.PointF? {
        if (!n.isVisibleToUser) return null
        val all = area(n)
        val v = Rect(all)
        if (all.isEmpty || !v.intersect(0, 0, width, height)) return null
        val enough = v.width().toLong() * v.height() * 2 >= all.width().toLong() * all.height() || (v.width() >= MIN_HIT && v.height() >= MIN_HIT)
        return if (enough) android.graphics.PointF(v.exactCenterX(), v.exactCenterY()) else null
    }

    private fun type(id: String, args: JSONObject) {
        val text = args.optString("text")
        // A field: the one named, else the one with the focus, else the only one on the screen; never a page or a box
        // around one (keys into a page with no field focused go anywhere).
        val node = find(args)?.second ?: focusedField()
        if (node == null) { fail(id, "No field to type into: give its ref (app_read)."); return }
        if (node.isPassword) { fail(id, "That's a password, card or code field: ask the user to type it themselves (app_ask_user)."); return }
        val at = area(node)
        aim(at.exactCenterX(), at.exactCenterY(), press = false) {
            if (display == null) { fail(id, "The app window closed."); return@aim }
            typeInto(id, node, text, args)
        }
    }

    private fun typeInto(id: String, node: AccessibilityNodeInfo, text: String, args: JSONObject) {
        val submit = args.optBoolean("submit")
        val said = "Typed into ${label(node).ifEmpty { "the field" }}${if (submit) " and sent it" else ""}."
        // Sent: the screen with the words typed in, read just before Enter, is what has to change (a search's results
        // come seconds later; the field's own new words aren't an answer).
        val send = {
            if (!quick) readScreen()
            val was = lastScreen
            if (isWeb(node) || !node.isEditable || !node.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)) key(KeyEvent.KEYCODE_ENTER)
            afterAct(said, was) { reply(id, step(it)) }
        }
        // A field in a web page (Uber Eats' search): the page only hears real keys. Setting its text showed the words
        // but the page never saw them, so Enter searched for nothing (2026-10-12). Tapped, cleared, then typed.
        if (isWeb(node) || !node.isEditable) {
            // What a field says, its hint not counted.
            fun says(f: AccessibilityNodeInfo): String = f.text?.toString().orEmpty().takeUnless { it == f.hintText?.toString() }.orEmpty()
            // Only a start of the words: keys after the first few went nowhere.
            fun partOf(had: String) = had.isNotEmpty() && had.length < text.length && text.startsWith(had, ignoreCase = true)
            // Checked as it goes: the words went in if the field says them, or the picture changed as they were typed
            // (the field's words lag) and the field doesn't say just a start of them. If not, once more: the field as it
            // is now, tapped, and the rest of the words at its end if it kept their start; else cleared and all again.
            // Uber Eats' menu search swaps its field for another as the first keys go in, which kept "Bi" of "Big Mac",
            // and clearing it for a second try did the same again (2026-10-13).
            fun attempt(f: AccessibilityNodeInfo, again: Boolean, todo: String, clear: Boolean) {
                val at = if (f.refresh()) shown(f) else null
                val tapIt = at != null && (again || !f.isFocused)
                if (tapIt) tap(at!!.x, at.y)
                ops.postDelayed({
                    if (display == null) { fail(id, "The app window closed."); return@postDelayed }
                    // Cleared whatever the field's words say: Chrome's lagging tree had it empty while an earlier try's
                    // words were in it, and the next try typed after them, or (the tap having moved the caret) into
                    // them: "McDonald'sMcDonMcDonald'sald's" (2026-10-13).
                    if (clear) {
                        keyWith(KeyEvent.KEYCODE_A, KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON)
                        key(KeyEvent.KEYCODE_DEL)
                    } else {
                        key(KeyEvent.KEYCODE_MOVE_END)
                    }
                    // The clearing drawn first, so it isn't taken for the words going in.
                    ops.postDelayed({
                        val mark = synchronized(lock) { motion?.mark(SystemClock.uptimeMillis()) }
                        typeKeys(todo)
                        // Keys go in a queue, and a page that searches as you type is slow on the R1: looked for until
                        // [TYPED_MS], not once after 200 ms (which retyped words that were on their way).
                        until(TYPED_MS, {
                            val had = if (f.refresh()) says(f) else ""
                            had.contains(text, ignoreCase = true) ||
                                (!partOf(had) && mark != null && (synchronized(lock) { motion?.changed(mark) } ?: 0) >= TYPED_CELLS)
                        }) { ok ->
                            when {
                                display == null -> fail(id, "The app window closed.")
                                ok -> if (submit) send() else afterAct(said) { reply(id, step(readBack(it, f, text))) }
                                !again -> {
                                    val g = (if (f.refresh()) f else null)?.takeIf { it.isEditable } ?: focusedField() ?: f
                                    val kept = says(g)
                                    if (partOf(kept)) attempt(g, true, text.substring(kept.length), clear = false)
                                    else attempt(g, true, text, clear = true)
                                }
                                else -> fail(id, "The words didn't show in “${label(f).ifEmpty { "the field" }}” after two tries. Look at the picture: if they're there, carry on; if not, tap the field on the picture and type again.")
                            }
                        }
                    }, CLEAR_MS)
                }, if (tapIt) FOCUS_MS else 0L)
            }
            attempt(node, false, text, clear = true)
            return
        }
        node.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
        val set = node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text)
        })
        if (!set) typeKeys(text)
        if (submit) send() else afterAct(said) { reply(id, step(readBack(it, node, text))) }
    }

    /**
     * Where [n] is on the screen. A web page's link can report no height at all (Uber Eats' store cards: the link wraps
     * the card, and its children hold the area), so then it's the area its children cover.
     */
    private fun area(n: AccessibilityNodeInfo): Rect {
        val r = Rect().also { n.getBoundsInScreen(it) }
        if (!r.isEmpty) return r
        val u = Rect()
        fun add(c: AccessibilityNodeInfo, depth: Int) {
            if (depth > 4) return
            val b = Rect().also { c.getBoundsInScreen(it) }
            if (!b.isEmpty && c.isVisibleToUser) u.union(b)
            for (i in 0 until c.childCount) child(c, i)?.let { add(it, depth + 1) }
        }
        for (i in 0 until n.childCount) child(n, i)?.let { add(it, 1) }
        if (!u.isEmpty) return u
        // A line in a real place (no height, or no width): its middle can still be tapped. Only nothing at all, at the
        // corner, means it's off the screen.
        if (r.width() > 0 || r.height() > 0) return Rect(r.left, r.top, maxOf(r.right, r.left + 1), maxOf(r.bottom, r.top + 1))
        return r
    }

    /**
     * [n]'s children that can be seen: a page drawn over its siblings (a later drawing order, covering their place, its
     * content filling the screen) hides them. Uber Eats keeps the search it opened a store from under the store, and
     * reads listed both (2026-10-12), so the agent tapped what it couldn't see. An overlay with only a button or two on
     * it doesn't fill the screen, and hides nothing.
     */
    private fun shownChildren(n: AccessibilityNodeInfo): List<AccessibilityNodeInfo> {
        val kids = (0 until n.childCount).mapNotNull { child(n, it) }
        if (kids.size < 2) return kids
        val page = kids.filter { it.isVisibleToUser && covers(Rect().also { r -> it.getBoundsInScreen(r) }) && fills(it) }.maxByOrNull { it.drawingOrder }
            ?: return kids
        val over = Rect().also { page.getBoundsInScreen(it) }
        // Only what's drawn strictly under it: a web page gives every element the same drawing order, which says nothing.
        return kids.filter { k -> k.drawingOrder >= page.drawingOrder || !over.contains(Rect().also { k.getBoundsInScreen(it) }) }
    }

    /** When each blank web page was last nudged, by its bounds (a page stays blank across reads). */
    private var nudged = 0L

    /**
     * Asks Chrome for a blank web page's tree: accessibility focus (as a screen reader gives), and a hover at its middle
     * (as touch exploration sends), which clicks nothing. A real scroll was what made Uber Eats' store appear.
     */
    private fun nudge(web: AccessibilityNodeInfo) {
        val now = SystemClock.uptimeMillis()
        if (!autoNudge || now - nudged < NUDGE_MS) return
        nudged = now
        web.performAction(AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS)
        val b = Rect().also { web.getBoundsInScreen(it) }
        if (b.isEmpty) return
        val x = b.exactCenterX()
        val y = b.exactCenterY()
        val t = SystemClock.uptimeMillis()
        for ((i, action) in intArrayOf(MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_EXIT).withIndex()) {
            val ev = MotionEvent.obtain(t, t + i * 16L, action, x, y + i, 0)
            ev.source = InputDevice.SOURCE_TOUCHSCREEN
            inject(ev)
            ev.recycle()
        }
    }

    /** The R1 has a network that reaches the internet (Android's check passed). */
    private fun online(): Boolean = runCatching {
        val cm = app.getSystemService(android.net.ConnectivityManager::class.java)
        cm.getNetworkCapabilities(cm.activeNetwork)?.hasCapability(android.net.NetworkCapabilities.NET_CAPABILITY_VALIDATED) == true
    }.getOrDefault(false)

    /** Reads nudge a blank web page ([nudge]); the harness turns it off to time a page left alone (T-146). */
    private var autoNudge = true

    /** The web pages on the screen: where each is, and how many nodes of its tree Chrome has handed over (to 200). */
    private fun webs(): org.json.JSONArray {
        val out = org.json.JSONArray()
        fun count(n: AccessibilityNodeInfo, depth: Int, left: IntArray) {
            for (i in 0 until n.childCount) {
                if (left[0] <= 0 || depth > 40) return
                val c = n.getChild(i, PREFETCH) ?: continue
                left[0]--
                count(c, depth + 1, left)
            }
        }
        fun walk(n: AccessibilityNodeInfo, depth: Int) {
            if (depth > 40) return
            if (n.className?.toString() == "android.webkit.WebView") {
                val left = intArrayOf(200)
                count(n, 0, left)
                val b = Rect().also { n.getBoundsInScreen(it) }
                out.put(JSONObject().put("bounds", b.flattenToString()).put("children", n.childCount).put("nodes", 200 - left[0]).put("shown", n.isVisibleToUser))
                return
            }
            for (i in 0 until n.childCount) n.getChild(i, PREFETCH)?.let { walk(it, depth + 1) }
        }
        roots().forEach { walk(it, 0) }
        return out
    }

    /**
     * One way of asking Chrome for a blank web page's tree (T-146), for the harness to time: `focus` (accessibility focus),
     * `hover` (touch exploration's hover), both (`both`: what reads do), `scroll` (a real stroke of 40 pixels up and back,
     * past touch slop so it taps nothing), `show` (ACTION_SHOW_ON_SCREEN), `refresh`, `extra` (the rendering info a screen
     * reader asks for), or `off` and `on` for reads' own nudge.
     */
    private fun nudgeBy(how: String): String {
        when (how) {
            "off", "on" -> { autoNudge = how == "on"; return "reads' nudge $how" }
        }
        var web: AccessibilityNodeInfo? = null
        fun find(n: AccessibilityNodeInfo, depth: Int) {
            if (web != null || depth > 40) return
            if (n.className?.toString() == "android.webkit.WebView" && n.childCount == 0 && n.isVisibleToUser) { web = n; return }
            for (i in 0 until n.childCount) n.getChild(i, PREFETCH)?.let { find(it, depth + 1) }
        }
        roots().forEach { find(it, 0) }
        val w = web ?: return "no blank web page on the screen"
        val b = Rect().also { w.getBoundsInScreen(it) }
        when (how) {
            "focus" -> w.performAction(AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS)
            "hover", "both" -> {
                if (how == "both") w.performAction(AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS)
                val t = SystemClock.uptimeMillis()
                for ((i, action) in intArrayOf(MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_EXIT).withIndex()) {
                    val ev = MotionEvent.obtain(t, t + i * 16L, action, b.exactCenterX(), b.exactCenterY() + i, 0)
                    ev.source = InputDevice.SOURCE_TOUCHSCREEN
                    inject(ev)
                    ev.recycle()
                }
            }
            "scroll" -> {
                val x = b.exactCenterX()
                val y = b.exactCenterY()
                swipe(x, y + 20, x, y - 20)
                SystemClock.sleep(250)
                swipe(x, y - 20, x, y + 20)
            }
            "show" -> w.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SHOW_ON_SCREEN.id)
            "refresh" -> w.refresh()
            "extra" -> w.refreshWithExtraData(AccessibilityNodeInfo.EXTRA_DATA_RENDERING_INFO_KEY, Bundle())
            else -> return "how: focus, hover, both, scroll, show, refresh, extra, off or on"
        }
        return "nudged ($how) the blank page at ${b.flattenToString()}"
    }

    /** Whether [r] covers (nearly) the whole screen. */
    private fun covers(r: Rect) = r.width() >= width * 0.95f && r.height() >= height * 0.9f

    /** Whether what's on [n] (words, or something to tap) spreads over most of the screen: a page, not an overlay. */
    private fun fills(n: AccessibilityNodeInfo): Boolean = fillsSeen.getOrPut(n) { fillsNow(n) }

    /** [fills]' answers for this read (cleared with each [roots]): pages nest, and each level asked again. */
    private val fillsSeen = HashMap<AccessibilityNodeInfo, Boolean>()

    private fun fillsNow(n: AccessibilityNodeInfo): Boolean {
        val u = Rect()
        var seen = 0
        val enough = { u.height() >= height / 2 && u.width() >= width / 2 }
        fun add(c: AccessibilityNodeInfo, depth: Int) {
            // Stopped as soon as it's known: walking on, for each page under another, fetched 300 nodes a read for
            // nothing (Chipotle's item sheet over its store over the search, 2026-10-13).
            if (seen++ > FILL_NODES || depth > 40 || enough()) return
            if (c !== n && c.isVisibleToUser && (c.isClickable || !c.text.isNullOrBlank() || !c.contentDescription.isNullOrBlank() ||
                    c.className?.toString() == "android.webkit.WebView")) {
                // A web page counts even before Chrome lets us read it: drawn over the screen it opened from, it hides
                // that, and the read says it's loading rather than reading what's under it.
                val b = Rect().also { c.getBoundsInScreen(it) }
                if (!b.isEmpty) u.union(b)
            }
            for (i in 0 until c.childCount) child(c, i)?.let { add(it, depth + 1) }
        }
        add(n, 0)
        return enough()
    }

    /** The field to type into when none's named: the one with the focus, else the only one on the screen. */
    private fun focusedField(): AccessibilityNodeInfo? {
        val fields = ArrayList<AccessibilityNodeInfo>()
        fun walk(n: AccessibilityNodeInfo, depth: Int) {
            if (depth > 40 || fields.size > 20) return
            if (n.isVisibleToUser && n.isEditable) fields.add(n)
            for (c in shownChildren(n)) walk(c, depth + 1)
        }
        roots().forEach { walk(it, 0) }
        return fields.firstOrNull { it.isFocused } ?: fields.singleOrNull()
    }

    /** Whether something inside [n], not far down, can be tapped, typed into or checked. */
    private fun actsInside(n: AccessibilityNodeInfo): Boolean {
        var seen = 0
        fun any(c: AccessibilityNodeInfo, depth: Int): Boolean {
            if (seen++ > INSIDE_NODES || depth > 6) return false
            if (c !== n && c.isVisibleToUser && (c.isClickable || c.isEditable || c.isCheckable)) return true
            for (i in 0 until c.childCount) child(c, i)?.let { if (any(it, depth + 1)) return true }
            return false
        }
        return any(n, 0)
    }

    /** What the field says now, after typing [typed] into it: the answer says so, and if it isn't what was typed. */
    private fun readBack(o: JSONObject, field: AccessibilityNodeInfo, typed: String): JSONObject {
        if (!field.refresh() || field.isPassword) return o
        val now = field.text?.toString().orEmpty()
        val note = if (now.contains(typed, ignoreCase = true)) "The field now reads “${now.take(120)}”."
        else "The field now reads “${now.take(120)}”, not what was typed: check the picture (a web page's words can lag)."
        val text = o.optString("text")
        val first = text.substringBefore('\n')
        return o.put("text", first + " " + note + text.removePrefix(first))
    }

    /** Whether [n] is (in) a web page: an app's embedded WebView. */
    private fun isWeb(n: AccessibilityNodeInfo): Boolean {
        var p: AccessibilityNodeInfo? = n
        var i = 0
        while (p != null && i++ < 40) {
            if (p.className?.toString() == "android.webkit.WebView") return true
            p = runCatching { p.parent }.getOrNull()
        }
        return false
    }

    /**
     * After an act: the screen settles and is read ([said] first). While it says it's still loading (a spinner,
     * "Loading"), it settles and reads again, up to [LOADING_MS]: an agent shouldn't choose from a half-loaded page.
     */
    /** Where an act's time went (settling, reading, rounds), sent with its answer for r1d's perf marks. */
    private class Took(var settle: Long = 0, var read: Long = 0, var rounds: Int = 0)

    private fun afterAct(
        said: String,
        was: String? = null,
        start: Long = SystemClock.uptimeMillis(),
        mark: Motion.Mark? = synchronized(lock) { motion?.mark(start) },
        took: Took = Took(),
        quiet: Long = SAME_MS,
        nothingSays: String = NOTHING_SAYS,
        then: (JSONObject) -> Unit,
    ) {
        val t0 = SystemClock.uptimeMillis()
        val done = { o: JSONObject ->
            then(o.put("took", JSONObject().put("settle", took.settle).put("read", took.read).put("rounds", took.rounds)
                .put("all", SystemClock.uptimeMillis() - start)))
        }
        settle {
            if (display == null) { done(JSONObject().put("app", name).put("text", "The app window closed.")); return@settle }
            if (quick) {
                // A step before an app_do's last: what it did, and whether anything changed, from the picture alone. The
                // last step reads the screen; reading after each one cost 1 to 2 s a step on Uber Eats' pages (T-147).
                took.settle += SystemClock.uptimeMillis() - t0
                val nothing = mark != null && (synchronized(lock) { motion?.changed(mark) } ?: CHANGED_SOME) < CHANGED_SOME
                if (nothing && SystemClock.uptimeMillis() - start < quiet) {
                    whenMoved(mark!!, start + quiet) { afterAct(said, was, start, mark, took, quiet, nothingSays, then) }
                } else {
                    done(JSONObject().put("app", name).put("quick", true).put("text", if (nothing) "$said\n$nothingSays" else said))
                }
                return@settle
            }
            val t1 = SystemClock.uptimeMillis()
            took.settle += t1 - t0
            took.rounds++
            val o = readScreen(said)
            val readMs = SystemClock.uptimeMillis() - t1
            took.read += readMs
            val waited = SystemClock.uptimeMillis() - start
            val same = was != null && lastScreen == was
            // How much of the picture the act changed, leaving out what was moving anyway.
            val moved = if (mark == null) 1f else synchronized(lock) { motion?.moved(mark) } ?: 1f
            // The words are the screen's from before the act, yet the picture changed a lot: a web page in the app whose
            // tree hasn't caught up (Uber Eats' item sheet, 2026-10-12: the words still listed the drinks for 12 s).
            val stale = same && moved >= STALE_MOVED
            // Anything visible at all: a checkbox ticked is a few samples (a percentage missed them, 2026-10-13).
            val nothing = mark != null && (synchronized(lock) { motion?.changed(mark) } ?: 0) < CHANGED_SOME
            if (stale) webViews().forEach { nudge(it) }
            when {
                o.optBoolean("loading") && waited < LOADING_MS -> afterAct(said, was, start, mark, took, quiet, nothingSays, then)
                // Nothing visible has happened yet: the app is fetching what it opens (Uber Eats' store came seconds
                // after the tap, the screen still meanwhile). The picture is watched, not the words re-read.
                same && nothing && waited < quiet -> whenMoved(mark!!, start + quiet) { afterAct(said, was, start, mark, took, quiet, nothingSays, then) }
                // Words catching up with a changed picture: a little while, not the whole wait, and not when reading
                // costs seconds itself (Chipotle's item sheet: 9 s a read, and each act read twice, 2026-10-13).
                stale && waited < STALE_MS && readMs < SLOW_READ_MS -> afterAct(said, was, start, mark, took, quiet, nothingSays, then)
                stale -> {
                    snapshot?.stale = true
                    done(o.put("text", o.optString("text").replaceFirst(said, "$said\n$STALE_SAYS")))
                }
                // Nothing at all changed: say so, as an agent tends to assume its tap worked.
                same && nothing -> done(o.put("text", o.optString("text").replaceFirst(said, "$said\n$nothingSays")))
                // A small visible change with the same words (a web page's radio filled in): the tap took; no waiting
                // for words that won't change. Every option on Uber Eats' item sheets cost 6 s before.
                same -> done(o.put("text", o.optString("text").replaceFirst(said, "$said (Only the picture changed: the words read as before.)")))
                else -> done(o)
            }
        }
    }

    /** [then] once the picture changes at all since [mark], or at [until]: watched every 150 ms, cheaply. */
    private fun whenMoved(mark: Motion.Mark, until: Long, then: () -> Unit) {
        val check = object : Runnable {
            override fun run() {
                if (display == null) { then(); return }
                val n = synchronized(lock) { motion?.changed(mark) } ?: CHANGED_SOME
                if (n >= CHANGED_SOME || SystemClock.uptimeMillis() >= until) then() else ops.postDelayed(this, 150)
            }
        }
        ops.postDelayed(check, 150)
    }

    /** The web pages (Chrome's WebViews) on the window's display, the outermost of each. */
    private fun webViews(): List<AccessibilityNodeInfo> {
        val out = ArrayList<AccessibilityNodeInfo>()
        fun walk(n: AccessibilityNodeInfo, depth: Int) {
            if (depth > 40 || out.size > 4) return
            if (n.className?.toString() == "android.webkit.WebView") { out.add(n); return }
            for (i in 0 until n.childCount) child(n, i)?.let { walk(it, depth + 1) }
        }
        roots().forEach { walk(it, 0) }
        return out
    }

    /**
     * Scrolls the list: the one named (or the one it's in), else the biggest that scrolls that way. Its own scroll
     * action first, which doesn't care where a finger lands: the swipe started on Spotify's player bar and on Uber
     * Eats' tab bar, and the list never moved (2026-10-13). A swipe in the middle of the list if it can't.
     */
    private fun scroll(id: String, args: JSONObject) {
        val dir = args.optString("direction", "down")
        val named = find(args)?.second
        val list = (named?.let { scrollerOf(it) }) ?: biggestScroller(dir)
        val area = list?.let { area(it) }?.takeIf { !it.isEmpty } ?: Rect(0, height / 8, width, height * 7 / 8)
        val cx = area.exactCenterX()
        val cy = area.exactCenterY()
        val dy = area.height() * 0.2f
        val dx = area.width() * 0.25f
        val (x0, y0, x1, y1) = when (dir) {
            "up" -> listOf(cx, cy - dy, cx, cy + dy)
            "left" -> listOf(cx - dx, cy, cx + dx, cy)
            "right" -> listOf(cx + dx, cy, cx - dx, cy)
            else -> listOf(cx, cy + dy, cx, cy - dy)
        }
        // The pointer to where the stroke starts, then along it (T-137).
        dragPointer(x0, y0, x1, y1, SWIPE_MS) {
            if (display == null) { fail(id, "The app window closed."); return@dragPointer }
            val action = when (dir) {
                "up" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP
                "left" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_LEFT
                "right" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_RIGHT
                else -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN
            }
            val back = dir == "up" || dir == "left"
            val done = list != null && (list.performAction(action.id) ||
                list.performAction(if (back) AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD else AccessibilityNodeInfo.ACTION_SCROLL_FORWARD))
            if (!done) swipe(x0, y0, x1, y1)
            afterAct("Scrolled $dir.") { reply(id, step(it)) }
        }
    }

    /** The list [n] is, or is in. */
    private fun scrollerOf(n: AccessibilityNodeInfo): AccessibilityNodeInfo? {
        var p: AccessibilityNodeInfo? = n
        var i = 0
        while (p != null && i++ < 30) {
            if (p.isScrollable) return p
            p = runCatching { p.parent }.getOrNull()
        }
        return null
    }

    /** The biggest visible list on the screen that scrolls [dir]'s way (tall for up and down, wide for left and right). */
    private fun biggestScroller(dir: String): AccessibilityNodeInfo? {
        val across = dir == "left" || dir == "right"
        var best: AccessibilityNodeInfo? = null
        var bestSize = 0
        fun walk(n: AccessibilityNodeInfo, depth: Int) {
            if (depth > 40) return
            if (n.isVisibleToUser && n.isScrollable) {
                val b = area(n)
                val size = if (across) b.width() else b.height()
                if (!b.isEmpty && size > bestSize && (across || b.height() >= b.width() / 3)) { best = n; bestSize = size }
            }
            for (c in shownChildren(n)) walk(c, depth + 1)
        }
        roots().forEach { walk(it, 0) }
        return best
    }

    // ---- input into the display -------------------------------------------------------------------------

    private val input: InputManager by lazy { app.getSystemService(InputManager::class.java) }
    private val injectMethod by lazy { InputManager::class.java.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType) }
    private val setDisplay by lazy { InputEvent::class.java.getMethod("setDisplayId", Int::class.javaPrimitiveType) }

    /** The agent's pointer glides there on the UI thread (T-137), and [act] runs back on the window's own as it lands. */
    private fun aim(x: Float, y: Float, press: Boolean = true, act: () -> Unit) {
        main.post { AgentPointer.app.aim(x, y, press) { ops.post(act) } }
    }

    private fun dragPointer(x0: Float, y0: Float, x1: Float, y1: Float, ms: Long, act: () -> Unit) {
        main.post { AgentPointer.app.drag(x0, y0, x1, y1, ms) { ops.post(act) } }
    }

    private fun pulse() {
        main.post { AgentPointer.app.pulse() }
    }

    private fun inject(ev: InputEvent) {
        // No window (it closed while the pointer glided): never onto the R1's own screen, which display -1 would mean.
        if (display == null) return
        // The screen's about to change: what was fetched of it isn't kept for the next walk (your touches come from the
        // UI thread, so the window's own thread drops it when it next looks).
        inputs++
        runCatching {
            setDisplay.invoke(ev, displayId)
            injectMethod.invoke(input, ev, 0)
        }
    }

    private fun touch(down: Long, at: Long, action: Int, x: Float, y: Float) {
        val ev = MotionEvent.obtain(down, at, action, x, y, 0)
        ev.source = InputDevice.SOURCE_TOUCHSCREEN
        inject(ev)
        ev.recycle()
    }

    private fun tap(x: Float, y: Float) {
        val t = SystemClock.uptimeMillis()
        touch(t, t, MotionEvent.ACTION_DOWN, x, y)
        touch(t, t + 60, MotionEvent.ACTION_UP, x, y)
    }

    private fun swipe(x0: Float, y0: Float, x1: Float, y1: Float) {
        val t = SystemClock.uptimeMillis()
        val steps = 12
        touch(t, t, MotionEvent.ACTION_DOWN, x0, y0)
        for (i in 1..steps) {
            val f = i / steps.toFloat()
            touch(t, t + i * 16L, MotionEvent.ACTION_MOVE, x0 + (x1 - x0) * f, y0 + (y1 - y0) * f)
        }
        touch(t, t + (steps + 1) * 16L, MotionEvent.ACTION_UP, x1, y1)
    }

    /** The wheel on the full window: the app scrolls (a tick is a little, down is a positive delta). */
    fun scrollBy(ticks: Int) {
        if (display == null || ticks == 0) return
        lastUse = SystemClock.uptimeMillis()
        val cx = width / 2f
        val cy = height / 2f
        val d = (ticks * WHEEL_PX).coerceIn(-height * 0.4f, height * 0.4f)
        ops.post { swipe(cx, cy + d / 2, cx, cy - d / 2) }
    }

    /** One of your touches on the full window ([dev.r1os.host.shell.AppWindowScreen]), already in display pixels. */
    fun yourTouch(ev: MotionEvent, x: Float, y: Float) {
        lastUse = SystemClock.uptimeMillis()
        val e = MotionEvent.obtain(ev.downTime, ev.eventTime, ev.actionMasked, x, y, 0)
        e.source = InputDevice.SOURCE_TOUCHSCREEN
        inject(e)
        e.recycle()
    }

    private fun key(code: Int) {
        if (code == KeyEvent.KEYCODE_UNKNOWN) return
        val t = SystemClock.uptimeMillis()
        for (action in intArrayOf(KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP)) {
            inject(KeyEvent(t, t, action, code, 0, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD))
        }
    }

    /** A key with modifiers held ([meta]: Ctrl+A selects all in a field). */
    private fun keyWith(code: Int, meta: Int) {
        val t = SystemClock.uptimeMillis()
        for (action in intArrayOf(KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP)) {
            inject(KeyEvent(t, t, action, code, 0, meta, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD))
        }
    }

    private fun typeKeys(text: String) {
        val events = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD).getEvents(text.toCharArray()) ?: return
        events.forEach { inject(KeyEvent(it)) }
    }

    private fun keyCode(k: String): Int = when (k.lowercase()) {
        "enter", "return" -> KeyEvent.KEYCODE_ENTER
        "tab" -> KeyEvent.KEYCODE_TAB
        "escape", "esc" -> KeyEvent.KEYCODE_ESCAPE
        "backspace" -> KeyEvent.KEYCODE_DEL
        "back" -> KeyEvent.KEYCODE_BACK
        "space" -> KeyEvent.KEYCODE_SPACE
        "arrowdown", "down" -> KeyEvent.KEYCODE_DPAD_DOWN
        "arrowup", "up" -> KeyEvent.KEYCODE_DPAD_UP
        "arrowleft", "left" -> KeyEvent.KEYCODE_DPAD_LEFT
        "arrowright", "right" -> KeyEvent.KEYCODE_DPAD_RIGHT
        else -> KeyEvent.KEYCODE_UNKNOWN
    }

    // ---- screenshots and answers ------------------------------------------------------------------------

    /** A step's answer with the screen's picture too (Max, Q62: a screenshot every step, with the lines). */
    private fun step(o: JSONObject): JSONObject {
        if (o.optBoolean("quick")) return o
        val bmp = synchronized(lock) { frame() } ?: return o
        val out = ByteArrayOutputStream()
        bmp.compress(Bitmap.CompressFormat.JPEG, 70, out)
        bmp.recycle()
        return o.put("image", Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)).put("width", width).put("height", height)
    }

    private fun screenshot(id: String) {
        val bmp = synchronized(lock) { frame() } ?: run { fail(id, "No picture yet: the app is still starting."); return }
        val out = ByteArrayOutputStream()
        bmp.compress(Bitmap.CompressFormat.JPEG, 70, out)
        val isSecure = secure(bmp)
        bmp.recycle()
        reply(id, JSONObject().put("image", Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)).put("app", name)
            .put("width", width).put("height", height).put("secure", isSecure))
    }

    private fun reply(id: String, result: Any) =
        R1d.post("/api/apps/window/result", JSONObject().put("id", id).put("ok", true).put("result", result), onError = {}) {}

    private fun fail(id: String, why: String) =
        R1d.post("/api/apps/window/result", JSONObject().put("id", id).put("ok", false).put("error", why), onError = {}) {}

    /**
     * The display's flags (android.hardware.display.DisplayManager): PUBLIC (an accessibility service may read it on
     * Android 15), OWN_CONTENT_ONLY (never a mirror), SUPPORTS_TOUCH, DESTROY_CONTENT_ON_REMOVAL (closing ends the
     * app's screens on it), TRUSTED (the app's own screens open there too), OWN_DISPLAY_GROUP and ALWAYS_UNLOCKED (its
     * own power, no lock screen), TOUCH_FEEDBACK_DISABLED, OWN_FOCUS and STEAL_TOP_FOCUS_DISABLED (keys stay r1os's).
     * Tried on the R1 in T-111: 0x17D49.
     */
    private const val FLAGS = (1 shl 0) or (1 shl 3) or (1 shl 6) or (1 shl 8) or (1 shl 10) or (1 shl 11) or (1 shl 12) or
        (1 shl 13) or (1 shl 14) or (1 shl 16)
    /** How the reader fetches nodes: depth first, in one call, as many as Android allows (50). */
    private const val PREFETCH = AccessibilityNodeInfo.FLAG_PREFETCH_DESCENDANTS_DEPTH_FIRST or AccessibilityNodeInfo.FLAG_PREFETCH_UNINTERRUPTIBLE
    /** Android's permission prompts (its own and Google's builds of the permission controller). */
    private val PROMPTS = setOf("com.android.permissioncontroller", "com.google.android.permissioncontroller")
    private const val PROMPT_SAYS = "Android is asking the user's permission here (a permission prompt): that's theirs to answer, never yours. " +
        "Hand it to them with app_ask_user (say what the app wants and why), then read the screen again."
    /** Ops that would answer a permission prompt (scrolling and reading only look). */
    private val ANSWERS = setOf("click", "type", "key", "back")
    private const val IDLE_MS = 2 * 60_000L
    /** How long to look for the app's screens on the window after it's started there. */
    private const val ARRIVE_MS = 3_000L
    /** How long the reader may take to see a new screen's windows. */
    private const val READABLE_MS = 3_000L
    private const val OPEN_SETTLE_MS = 8_000L
    private const val SETTLE_MS = 4_000L
    private const val MIN_SETTLE_MS = 300L
    /** Frames a second to the full window, and to the chat's small preview. */
    private const val WATCH_FPS = 30
    /** The preview is a corner of a chat: 6 still looks live there, and each one redraws the chat (T-141). */
    private const val PREVIEW_FPS = 6
    /** The window's size in pixels, at the R1's density (ADR-0063). */
    const val WINDOW_W = 600
    const val WINDOW_H = 1200
    /** What apps say when they think they're offline. */
    private val OFFLINE = listOf("you're offline", "you’re offline", "you are offline", "no internet connection")
    /**
     * The reader's images: the latest, the one shown, the one shown before (let go after [LET_GO_MS]), and two for
     * [ImageReader.acquireLatestImage] to take the newest with (at 4 it logged running out 24 times a minute).
     */
    private const val IMAGES = 5
    /** How long a frame no longer shown stays open: a screen may still be drawing it. */
    private const val LET_GO_MS = 100L
    private val SRGB = android.graphics.ColorSpace.get(android.graphics.ColorSpace.Named.SRGB)
    private const val QUIET_MS = 500L
    private const val READER_WAIT_MS = 3_000L
    private const val MAX_LINES = 200
    private const val WHEEL_PX = 60f
    /** A swipe's length in time ([swipe]: 13 steps of 16 ms). */
    private const val SWIPE_MS = 13 * 16L
    /** How long an app may take to bring something into view before a tap. */
    private const val SHOW_MS = 1_500L
    /** Enough of an element on the screen to tap it, in pixels each way. */
    private const val MIN_HIT = 24
    /** After tapping a web field, before typing into it. */
    private const val FOCUS_MS = 250L
    /** How long an act waits for a screen that says it's loading. */
    private const val LOADING_MS = 6_000L
    /** How long an act waits while the screen reads just as it did before it. */
    private const val SAME_MS = 6_000L
    /** This much of the picture changing while the words stay the same means they're stale. */
    private const val STALE_MOVED = 0.2f
    /** Fewer samples than this changing is nothing visible at all. */
    private const val CHANGED_SOME = 2
    /** How long an act waits for stale words to catch up with the picture, and a read too slow to try again for that. */
    private const val STALE_MS = 2_500L
    private const val SLOW_READ_MS = 2_000L
    private fun webNothing(what: String) =
        "(Nothing changed yet: the page may not have taken its own click on “${what.take(40)}”, or may still be acting on it. " +
            "app_wait and look at the picture first; tap its place only if it's still there, the same button, and nothing changed.)"
    private const val NOTHING_SAYS = "(Nothing changed on the screen: the tap may have missed or done nothing. Check the picture, then try its position, another way, or app_wait. A counter (+ or -) or a toggle: app_wait and look again before tapping it again, as a slow page can count both taps.)"
    private const val STALE_SAYS = "(The picture changed but the app hasn't updated these words yet: a web page inside the app. Go by the picture (tap at x,y), or app_wait and read again.)"
    /** At most this often, a nudge to Chrome for a blank web page. */
    private const val NUDGE_MS = 1_000L
    /** After typing keys, before reading the field and sending it: the page takes them in. */
    /** How long typing into a web page's field is looked for, and how long its clearing gets to be drawn first. */
    private const val TYPED_MS = 1500L
    private const val CLEAR_MS = 150L
    /** Cells that typed words change at least: more than a caret blinking in the field (two or three). */
    private const val TYPED_CELLS = 4
    /** How long [findSoon] waits for what's named to appear, and how long it lets the screen settle between looks. */
    private const val FIND_SOON_MS = 4000L
    private const val FIND_SOON_STEP_MS = 1000L
    /** How many buttons' words [describe] sends. */
    private const val BUTTONS = 120
    /** How far around a button a tap by place counts as maybe on it, for [describe]'s `near` (about 4.5 mm). */
    private const val NEAR_PX = 32
    /** How far [actsInside] looks. */
    private const val INSIDE_NODES = 60
    /** How much of a list's name (its first items' words) a read keeps. */
    private const val LIST_NAME = 40
    /** How far [fills] looks over a page that may cover others. */
    private const val FILL_NODES = 150
    /**
     * The app window's frame rate, preview and full screen alike (a virtual display's rate is fixed when it's made). Max,
     * 2026-10-13: "go with 15fps". An app you take over moves to the R1's own screen at its full rate.
     */
    private const val WINDOW_FPS = 15f
    /** How long after the window closes an app opened only for the agent is closed too. */
    private const val FREE_MS = 60_000L
    /** How many nodes off the screen a read takes, inside what's off the screen already. */
    private const val OFF_NODES = 120
    /** An image's address in its name, left out of what's read. */
    private val SRC = Regex("srcb64=\\S+")
    /** How far [content] looks for something on a screen. */
    private const val CONTENT_NODES = 300
    private const val SCREEN_CHARS = 400
}
